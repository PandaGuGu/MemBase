//! 息屏相关命令：`deskkit screen <动词>`
//!
//! 只做一件事：关显示器，不锁屏、不睡眠。
//! 界面与之对应：「全屏黑 + 退出热键 + 到点自动亮」，命令行这边：
//!   * 关屏          → `screen off`
//!   * 息屏时长      → `--seconds N`（支持 `1:05:09` / `5m` / `90s`；0 = 不自动亮）
//!   * 退出热键      → `screen key esc|space|enter|any`（默认 ESC）
//!   * 退出/中止     → 按那个键（命令行下 Ctrl+C）
//!
//! 息屏本身无损可逆，所以不像 trim 那样强制 --execute，只提供 --dry-run 供自检。

use crate::{arg_value, has_flag, want_json, EXIT_ACTION_FAILED, EXIT_USAGE};
use deskkit_core::bundle::jstr;
use deskkit_win::screen::{self, ExitKey};

pub const SCREEN_COMMANDS: &[(&str, &str, &str)] = &[
    ("off", "立刻关显示器（不锁屏）", "[--seconds N] [--minutes N] [--delay N] [--dry-run] [--json]"),
    ("status", "显示器 / 通道 / 默认时长 / 退出键 / 上次息屏", "--json"),
    ("duration", "查看或设置默认息屏时长", "[1:05:09 | 90s | 5m | 0] --json"),
    ("key", "查看或设置退出热键", "[esc|space|enter|any] --json"),
    ("power", "查看或设置「息屏时是否同时关闭显示器」", "[on|off] --json"),
];

pub fn screen_usage() -> String {
    let mut s = String::from("deskkit screen —— 息屏（只关显示器，不锁屏）\n\n用法: deskkit screen <动词> [选项]\n\n");
    for (n, d, a) in SCREEN_COMMANDS {
        s.push_str(&format!("  {:<9} {:<30} {}\n", n, d, a));
    }
    s.push_str("\n选项:\n");
    s.push_str("  --seconds N   息屏 N 秒后自动亮（0 = 不自动亮，等退出键；缺省用默认时长）\n");
    s.push_str("  --minutes N   同上的分钟写法（老习惯，等价于 --seconds N*60）\n");
    s.push_str("  --delay N     等 N 秒再关屏（够你把手从键盘上拿开）\n");
    s.push_str("  --exit-key K  这次用哪个键退出：esc | space | enter | any\n");
    s.push_str("  --dry-run     只检查关屏通道通不通，不真的关屏\n");
    s.push_str("  --json        结构化输出\n");
    s.push_str("\n时长写法: 3600 / 90s / 5m / 1h / 1:05:09 都认；0 表示不定时。\n");
    s.push_str("退出: 界面上按设定热键（默认 ESC）；命令行用 Ctrl+C。\n");
    s.push_str("注意: `screen off` 会真关显示器电源，有的机器关得掉、叫不醒；\n");
    s.push_str("      界面上的息屏默认不碰电源（只盖全屏黑窗），要不要关看 `screen power`。\n");
    s
}

pub fn cmd_screen(args: &[String]) -> i32 {
    // 读分辨率要 DPI 感知，否则 125% 缩放下报的是逻辑像素
    deskkit_win::init();
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    match sub.as_str() {
        "off" | "sleep" => screen_off(&rest),
        "status" | "info" => screen_status(&rest),
        "duration" | "time" | "seconds" | "minutes" => screen_duration(&rest, &sub),
        "key" | "hotkey" | "exit-key" => screen_key(&rest),
        "power" | "power-off" | "display" => screen_power(&rest),
        "" | "help" | "-h" | "--help" => {
            println!("{}", screen_usage());
            0
        }
        other => {
            eprintln!("未知子命令: screen {}", other);
            println!("{}", screen_usage());
            EXIT_USAGE
        }
    }
}

/// 解析时长参数：`--seconds` 优先，其次 `--minutes`（老写法），都没有就用默认值
fn duration_arg(args: &[String]) -> u64 {
    if let Some(v) = arg_value(args, "--seconds") {
        if let Some(n) = screen::parse_duration(&v) {
            return n;
        }
    }
    if let Some(v) = arg_value(args, "--minutes") {
        if let Some(n) = screen::parse_duration(&v) {
            return (n * 60).min(screen::MAX_SECS);
        }
    }
    screen::default_secs()
}

fn exit_key_arg(args: &[String]) -> Option<ExitKey> {
    arg_value(args, "--exit-key")
        .and_then(|v| ExitKey::parse(&v))
        .or_else(|| arg_value(args, "--key").and_then(|v| ExitKey::parse(&v)))
}

fn screen_off(args: &[String]) -> i32 {
    let dry = has_flag(args, "--dry-run");
    let delay: u64 = arg_value(args, "--delay")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(300);
    let secs = duration_arg(args);
    let key = exit_key_arg(args).unwrap_or_else(screen::exit_key);
    let human = if secs == 0 {
        "不定时（等你按退出键）".to_string()
    } else {
        format!("{} 后自动亮", screen::fmt_hms(secs))
    };

    if dry {
        let ok = screen::probe();
        if want_json(args) {
            println!(
                "{{\"ok\":{},\"dry_run\":true,\"seconds\":{},\"minutes\":{},\"delay\":{},\"exit_key\":{},\"channel\":{}}}",
                ok,
                secs,
                secs / 60,
                delay,
                jstr(key.as_str()),
                ok
            );
        } else if ok {
            println!(
                "[演练] 关屏通道正常；真正执行会关屏（{}，按 {} 退出）",
                human,
                key.label()
            );
            println!(
                "真正关屏：deskkit screen off --seconds {} --exit-key {}",
                secs,
                key.as_str()
            );
        } else {
            println!("[演练] 关屏通道可能不通（广播没人应答），真实关屏时可能无效");
        }
        return if ok { 0 } else { EXIT_ACTION_FAILED };
    }

    if delay > 0 {
        if !want_json(args) {
            println!("{} 秒后关屏…（Ctrl+C 取消）", delay);
        }
        std::thread::sleep(std::time::Duration::from_secs(delay));
    }

    match screen::off() {
        Ok(()) => {
            if secs > 0 {
                if !want_json(args) {
                    println!(
                        "已关屏，{} 后自动亮（按 {} 或 Ctrl+C 可提前结束；动鼠标也会亮）",
                        screen::fmt_hms(secs),
                        key.label()
                    );
                }
                // 驻留到点：这是「息屏时长」在命令行里的表达，界面版是全屏黑窗口里的倒计时
                std::thread::sleep(std::time::Duration::from_secs(secs));
                screen::wake();
                if want_json(args) {
                    println!(
                        "{{\"ok\":true,\"seconds\":{},\"minutes\":{},\"delay\":{},\"exit_key\":{},\"woke\":true,\"monitors\":{}}}",
                        secs,
                        secs / 60,
                        delay,
                        jstr(key.as_str()),
                        screen::monitor_count()
                    );
                } else {
                    println!("已到时间，显示器已叫醒");
                }
            } else if want_json(args) {
                println!(
                    "{{\"ok\":true,\"seconds\":0,\"minutes\":0,\"delay\":{},\"exit_key\":{},\"woke\":false,\"monitors\":{}}}",
                    delay,
                    jstr(key.as_str()),
                    screen::monitor_count()
                );
            } else {
                println!("已关屏（按 {} 或动一下鼠标就回来）", key.label());
            }
            0
        }
        Err(e) => {
            if want_json(args) {
                println!("{{\"ok\":false,\"error\":{}}}", jstr(&e));
            } else {
                eprintln!("[x] {}", e);
            }
            EXIT_ACTION_FAILED
        }
    }
}

fn screen_status(args: &[String]) -> i32 {
    let monitors = screen::monitor_count();
    let (w, h) = deskkit_win::sys::screen_size();
    let dpi = deskkit_win::sys::dpi_scale();
    let last = screen::last_off().unwrap_or_default();
    let channel = screen::probe();
    let secs = screen::default_secs();
    let key = screen::exit_key();

    if want_json(args) {
        println!(
            "{{\"monitors\":{},\"screen\":{{\"w\":{},\"h\":{}}},\"dpi_scale\":{:.2},\"channel_ok\":{},\"last_off\":{},\"default_seconds\":{},\"default_minutes\":{},\"exit_key\":{}}}",
            monitors,
            w,
            h,
            dpi,
            channel,
            jstr(&last),
            secs,
            secs / 60,
            jstr(key.as_str())
        );
    } else {
        println!("显示器台数   : {}", monitors);
        println!("主屏分辨率   : {}x{}", w, h);
        println!("缩放比例     : {:.0}%", dpi * 100.0);
        println!(
            "关屏通道     : {}",
            if channel {
                "可用"
            } else {
                "不可用（可能处于远程会话）"
            }
        );
        println!(
            "默认息屏时长 : {}",
            if secs == 0 {
                "不定时（等退出键）".to_string()
            } else {
                format!("{}（{} 秒）", screen::fmt_hms(secs), secs)
            }
        );
        println!("退出热键     : {}", key.label());
        println!(
            "上次息屏     : {}",
            if last.is_empty() { "（还没用过）" } else { &last }
        );
        println!("记录文件     : {}", screen::last_off_path().display());
    }
    0
}

fn screen_duration(args: &[String], sub: &str) -> i32 {
    let arg = args.iter().find(|a| !a.starts_with("--")).cloned();
    match arg {
        None => {
            let secs = screen::default_secs();
            if want_json(args) {
                println!(
                    "{{\"ok\":true,\"default_seconds\":{},\"default_minutes\":{},\"text\":{}}}",
                    secs,
                    secs / 60,
                    jstr(&screen::fmt_hms(secs))
                );
            } else {
                println!(
                    "默认息屏时长 : {}（{} 秒）",
                    if secs == 0 {
                        "不定时".to_string()
                    } else {
                        screen::fmt_hms(secs)
                    },
                    secs
                );
                println!("设置方法     : deskkit screen duration 0:05:30");
                println!("               也认 90s / 5m / 1h / 330（纯数字=秒）");
            }
            0
        }
        Some(v) => {
            // `screen minutes 30` 这种老写法：纯数字按分钟理解
            let parsed = if sub == "minutes" && v.chars().all(|c| c.is_ascii_digit()) {
                v.parse::<u64>().ok().map(|m| (m * 60).min(screen::MAX_SECS))
            } else {
                screen::parse_duration(&v)
            };
            let Some(secs) = parsed else {
                eprintln!("看不懂这个时长：{v}（用法：1:05:09 / 90s / 5m / 1h / 330）");
                return EXIT_USAGE;
            };
            match screen::set_default_secs(secs) {
                Ok(()) => {
                    if want_json(args) {
                        println!(
                            "{{\"ok\":true,\"default_seconds\":{},\"default_minutes\":{},\"text\":{}}}",
                            secs,
                            secs / 60,
                            jstr(&screen::fmt_hms(secs))
                        );
                    } else {
                        println!(
                            "已把默认息屏时长设为 {}（{} 秒）；界面上的时/分/秒会跟着变",
                            screen::fmt_hms(secs),
                            secs
                        );
                    }
                    0
                }
                Err(e) => {
                    eprintln!("[x] {}", e);
                    EXIT_ACTION_FAILED
                }
            }
        }
    }
}

/// `screen power [on|off]`：息屏时要不要顺便关掉显示器电源（默认关，最稳妥）
fn screen_power(args: &[String]) -> i32 {
    let arg = args.iter().find(|a| !a.starts_with("--")).cloned();
    match arg {
        None => {
            let on = screen::power_off();
            if want_json(args) {
                println!("{{\"ok\":true,\"power_off\":{}}}", on);
            } else {
                println!(
                    "同时关闭显示器 : {}",
                    if on { "开（退出时会尝试叫醒）" } else { "关（只盖全屏黑窗）" }
                );
                println!("设置方法       : deskkit screen power on | off");
            }
            0
        }
        Some(v) => {
            let on = match v.to_lowercase().as_str() {
                "1" | "on" | "true" | "yes" | "开" => true,
                "0" | "off" | "false" | "no" | "关" => false,
                _ => {
                    eprintln!("用法: deskkit screen power on|off（默认 off：只盖全屏黑窗）");
                    return EXIT_USAGE;
                }
            };
            match screen::set_power_off(on) {
                Ok(()) => {
                    if want_json(args) {
                        println!("{{\"ok\":true,\"power_off\":{}}}", on);
                    } else {
                        println!(
                            "已把「同时关闭显示器」设为 {}；界面上的勾选框会跟着变\n{}",
                            if on { "开" } else { "关" },
                            if on {
                                "注意：有的机器关得掉、叫不醒，退出时按电源键进锁屏再回来"
                            } else {
                                "只盖全屏黑窗，ESC 一定退得出来"
                            }
                        );
                    }
                    0
                }
                Err(e) => {
                    eprintln!("[x] {}", e);
                    EXIT_ACTION_FAILED
                }
            }
        }
    }
}

fn screen_key(args: &[String]) -> i32 {
    let arg = args.iter().find(|a| !a.starts_with("--")).cloned();
    match arg {
        None => {
            let k = screen::exit_key();
            if want_json(args) {
                println!("{{\"ok\":true,\"exit_key\":{}}}", jstr(k.as_str()));
            } else {
                println!("当前退出热键 : {}", k.label());
                println!("可选项       : esc / space / enter / any");
                println!("设置方法     : deskkit screen key space");
            }
            0
        }
        Some(v) => match ExitKey::parse(&v) {
            Some(k) => match screen::set_exit_key(k) {
                Ok(()) => {
                    if want_json(args) {
                        println!("{{\"ok\":true,\"exit_key\":{}}}", jstr(k.as_str()));
                    } else {
                        println!("已把退出热键设为 {}；界面上的下拉框会跟着变", k.label());
                    }
                    0
                }
                Err(e) => {
                    eprintln!("[x] {}", e);
                    EXIT_ACTION_FAILED
                }
            },
            None => {
                eprintln!("用法: deskkit screen key esc|space|enter|any");
                EXIT_USAGE
            }
        },
    }
}
