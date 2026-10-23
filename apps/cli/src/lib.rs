#![cfg_attr(not(windows), allow(dead_code, unused_imports))]
//! MemBase.exe —— 内存基准线 + 桌面图标布局（命令行逻辑）
//!
//! 同一份逻辑被两个入口复用：带参数启动的便携版 `MemBase.exe`
//! （自己接回父控制台），以及独立的控制台版 `membase.exe`。
//!
//! 两件本来不相干的事在这里共用一套骨架：
//!   * 内存侧：开机基准线 / 线外清单 / 恢复基准（原 MemBase）
//!   * 桌面侧：桌面图标布局 / 差异预览 / 恢复 / 修复缓存（原 DeskPos）
//! 界面上的每个可点操作，在 [`layout::ACTIONS`] 里都有一条等价命令。
//!
//! 设计目标：所有操作都能被脚本 / AI 驱动
//!   * 每个动作一个子命令，没有交互式菜单
//!   * 所有子命令支持 --json，输出结构化结果
//!   * 固定退出码；需要动手的动作默认 dry-run，必须显式 --execute
//!   * `mcp` 子命令把同一套动词以 MCP 协议暴露给 AI（见 [`mcp`]）
//!
//! 退出码：
//!   0 成功 / 1 参数错误 / 2 没有基准快照 / 3 读写失败
//!   4 有动作执行失败 / 5 被安全护栏拒绝 / 6 需要管理员权限

pub mod mcp;

#[cfg(windows)]
pub mod layout;
#[cfg(windows)]
pub mod screen;

#[cfg(windows)]
use membase_core::{Kind, Tier};
#[cfg(windows)]
use membase_win::{close, compare, restart, snapshot, win};
// 常量与平台无关，usage() 在非 Windows 上也要用
use membase_core as core;
#[cfg(windows)]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(windows)]
use std::io::IsTerminal;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::process::Command;

use std::fs;
use std::path::{Path, PathBuf};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const EXIT_USAGE: i32 = 1;
const EXIT_NO_BASELINE: i32 = 2;
const EXIT_IO: i32 = 3;
const EXIT_ACTION_FAILED: i32 = 4;
const EXIT_REFUSED: i32 = 5;

fn usage() {
    println!(
        r#"MemBase.exe {ver} —— 内存基准线 + 桌面图标布局（每个界面操作都有命令）

用法: MemBase.exe <命令> [选项]

  status                     概况：基准线信息 + 当前内存 + 线外计数
  list                       列出线外进程/服务（只读，不动手）
      --all                  连「系统关键」项也列出来
      --min-mb N             小于 N MB 的第三方进程忽略（默认 10）
      --only a,b             只看这些名字（支持 * 通配）
      --exclude a,b          忽略这些名字
      --include-caution      把「谨慎档」也算进可处理计数

  plan                       只生成线外计划（不执行）
      --to FILE              把计划写成 CSV
      --min-mb / --only / --exclude / --include-caution   同 list

  kill                       原子操作：终止指定进程
      kill <进程名|PID> --execute

  svc                        原子操作：停止 / 启动 / 禁止自启单个服务
      svc stop <服务名> --execute
      svc start <服务名> --execute
      svc disable|enable <服务名> --execute

  restart                    局部重启：关掉再按原路径拉起，报告**实测**回收量
      restart <进程名|PID|服务名> --execute --yes
      --wait SEC             拉起后等多久再量工作集（默认 3）
      --path FILE            指定重新拉起用的可执行路径（默认取快照里记的）
      说明：explorer / SearchHost 会闪任务栏，必须显式 --yes；
            没有可执行路径、或属于共享宿主（svchost/dllhost）的一律不动

  slim                       基线瘦身：砍掉基准里开机不需要的服务 / Store 应用 / 功能开关
      --execute --yes        真正执行（不加就是 dry-run）
      --include-caution      连 Windows 目录下的组件一起（可能断网，逐条判断）
      --min-mb / --only / --exclude

  transient                  双采样对比：只在前一次出现、后一次消失的「短命项」

  startup                    启动项扫描：注册表 Run / 启动文件夹 / 计划任务（含 \Microsoft\ 下的）
      startup list           来源 / 名称 / 目标进程 / 是否在基准里 / 建议动作
      --all                  连系统计划任务一起列（VS 安装器的 BackgroundDownload 就在里面）
      startup disable <名称> --execute --yes   禁用（改批准记录或任务状态，不删原值）
      startup enable  <名称> --execute --yes   撤销

  feature                    已知功能开关（表驱动，只写 HKCU，不开放任意注册表编辑）
      feature list           开关当前值 + 证据进程 + 结论（可关 / 已关但仍在跑）
      feature off <key> --execute --yes   关掉，并顺带重启宿主让它当场生效
      feature on  <key> --execute --yes   撤销（写回原值 / 删值回默认）
      --no-restart           只改注册表，不动宿主进程

  config                     数据目录（基准线存哪儿）
      config get             当前生效目录 + 从哪来的
      config set <目录>       改成你指定的目录（会写配置文件）
      config clear           恢复系统默认

  commands                   列出所有命令与参数（JSON 供 AI 发现）

  mcp                        以 MCP 协议（stdio + JSON-RPC 2.0）服务
                             同一套动词暴露给 AI，护栏与命令行完全一致：
                             动手类工具的 execute 与 yes 必须同时为 true
                             客户端配置：command=MemBase.exe, args=[mcp]

  trim                       恢复基准：关掉线外项
      --execute              真正执行（不加就是 dry-run，只出计划）
      --yes                  跳过确认（非交互环境必须带）
      --include-caution      连 Windows 组件一起处理
      --min-mb / --only / --exclude   同 list
      --max N                一次最多处理 N 项（默认 20，0=不限）
      --plan-out FILE        把计划写成 CSV

  protect                    管理保护名单（勾选保护 = 永不处理）
      protect list | add <名字...> | remove <名字...> | clear
      名字格式: chrome / MuMu* / 服务:SensorService

  collect                    采集一次快照
      --out DIR              数据目录（默认 {root}）
      --at SEC               等到开机第 SEC 秒（默认 60）
      --also SEC             再等到第 SEC 秒采第二次（双采样，同一 run 目录）
      --include-self         把采集器自己也算进去
      --self-destruct        采集后删任务和本 exe
      --task NAME            计划任务名（默认 {task}）
      --keep N               只保留最近 N 个 run_ 目录，老的移进回收站（默认 10，0=不轮换）

  compare                    和最近一次基准做 diff
      --label NAME           指定用哪个采样点当基准（T+30s / T+300s / now）
      --min-mb N             只看变化超过 N MB 的（默认 10）
      --exclude a,b          忽略这些名字

  baseline                   基准线本身的查看 / 校验 / 采集 / 导出 / 导入（跨平台 + 采集）
      baseline info          当前生效的是哪份快照、包含哪些文件、整体 crc32
      baseline verify        校验快照完整性（文件齐不齐、CSV 能不能解析、crc32）
      baseline capture [--label NAME]
                             立刻扫描当前状态并设为基准（不用重启，测「干净态」用）
      baseline export [--to FILE]
                             打包成单文件 .mbz（默认 <out>\exports\baseline_<采集时间>.mbz）
      baseline import <FILE> [--label NAME]
                             把包还原成一份新快照；导入后它会是「最新」，即当前基准

  install / uninstall        注册 / 删除开机计划任务
      install --at 30 --also 300
                             双采样：第 30 秒与第 300 秒各采一次，用来分「短命项」和「真常驻」

  layout                     桌面图标布局（原 DeskPos 的全部能力）
      layout list            列出已保存的布局
      layout save [名字]      保存当前桌面（名字缺省=auto-时间戳）
      layout restore [名字]   恢复布局：默认演练，--execute --yes 才动手
      layout diff [名字]      只看差异（--positions 逐个打印当前坐标）
      layout show [名字]      查看布局内容
      layout delete <名字> --yes
      layout status          图标数 / 自动排列 / 分辨率 / shell 接口 / 缓存
      layout probe           诊断 shell 官方接口能不能用
      layout repair          清理损坏的系统布局缓存（默认演练）
      layout dir [--open]    布局目录

  theme                      外观（与界面「设置」页同一个文件）
      theme get | set light|dark

  ui                         打开界面（可选 --page 指定页面）
      ui --page layouts     进程|基准|layouts|protect|boot|cli|tools|settings

  screen                     息屏（只关显示器，不锁屏）
      screen off             立刻关屏；--seconds N 到点自动亮，--exit-key K 换退出键
      screen duration [时长]  默认时长：1:05:09 / 90s / 5m / 1h / 0（不定时）
      screen key [键]         默认退出热键：esc / space / enter / any
      screen status          显示器 / 通道 / 默认时长 / 退出键 / 上次息屏

全局:
  --json                     输出 JSON（适合程序/AI 解析）
  --out DIR                  数据目录，默认 {root}

退出码: 0 成功 / 1 参数 / 2 无基准 / 3 IO / 4 动作失败 / 5 被拒绝"#,
        ver = membase_core::VERSION,
        root = core::default_root(),
        task = core::default_task()
    );
}

// ---------------- 参数小工具 ----------------

fn has_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn num_arg(args: &[String], key: &str, default: f64) -> f64 {
    arg_value(args, key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 可选数值参数：没写返回 None（和「写了 0」区分开）
fn num_arg_opt(args: &[String], key: &str) -> Option<f64> {
    arg_value(args, key).and_then(|v| v.trim().parse().ok())
}

fn list_arg(args: &[String], key: &str) -> Vec<String> {
    arg_value(args, key)
        .map(|v| {
            v.split([',', ';'])
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn root_arg(args: &[String]) -> PathBuf {
    // 优先级：--out > MEMBASE_ROOT > 配置文件 > %ProgramData%\MemBaseline
    let cli_out = arg_value(args, "--out");
    PathBuf::from(core::config::data_root(cli_out.as_deref()))
}

fn task_arg(args: &[String]) -> String {
    arg_value(args, "--task").unwrap_or_else(core::default_task)
}

fn want_json(args: &[String]) -> bool {
    has_flag(args, "--json")
}

/// 取位置参数（跳过 --flag 及其取值）
fn positionals<'a>(args: &'a [String], value_flags: &[&str]) -> Vec<&'a String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a.starts_with("--") {
            if value_flags.contains(&a.as_str()) {
                skip = true;
            }
            continue;
        }
        out.push(a);
    }
    out
}


// ---------------- 外观（与界面「设置」页共用 theme.txt） ----------------

/// 主题文件路径：和基准线配置同目录的 theme.txt
pub fn theme_path() -> PathBuf {
    membase_core::config::config_path().with_file_name("theme.txt")
}

pub fn read_theme() -> bool {
    fs::read_to_string(theme_path())
        .map(|t| t.trim().eq_ignore_ascii_case("dark"))
        .unwrap_or(false)
}

pub fn write_theme(dark: bool) -> std::io::Result<()> {
    let p = theme_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, if dark { "dark" } else { "light" })
}

fn cmd_theme(args: &[String]) {
    let json = want_json(args);
    let pos = positionals(args, &["--out"]);
    let sub = pos.first().map(|s| s.to_lowercase()).unwrap_or_else(|| "get".into());
    let value = pos.get(1).map(|s| s.to_lowercase());

    match sub.as_str() {
        "get" | "show" => {}
        "set" => {
            let dark = match value.as_deref() {
                Some("dark") | Some("深色") | Some("黑") => true,
                Some("light") | Some("浅色") | Some("白") => false,
                _ => {
                    eprintln!("用法: MemBase.exe theme set light|dark");
                    std::process::exit(EXIT_USAGE);
                }
            };
            if let Err(e) = write_theme(dark) {
                eprintln!("[x] 写主题失败: {}", e);
                std::process::exit(EXIT_IO);
            }
        }
        other => {
            eprintln!("未知子命令: theme {}", other);
            std::process::exit(EXIT_USAGE);
        }
    }

    let dark = read_theme();
    if json {
        println!(
            "{{\"ok\":true,\"theme\":{},\"file\":{}}}",
            jstr(if dark { "dark" } else { "light" }),
            jstr(&theme_path().to_string_lossy())
        );
    } else {
        println!("当前外观 : {}", if dark { "深色" } else { "浅色" });
        println!("配置文件 : {}", theme_path().display());
    }
}

// ---------------- 数据目录配置（跨平台） ----------------

fn cmd_config(args: &[String]) {
    use membase_core::config as cfg;
    let json = want_json(args);
    let pos = positionals(args, &["--out", "--to", "--task", "--label"]);
    let sub = pos.first().map(|s| s.as_str()).unwrap_or("get");
    let cli_out = arg_value(args, "--out");

    match sub {
        "get" | "show" => {
            let effective = cfg::data_root(cli_out.as_deref());
            let source = cfg::root_source(cli_out.as_deref());
            let file = cfg::config_path();
            let configured = cfg::load().data_root.unwrap_or_default();
            if json {
                println!(
                    "{{\"ok\":true,\"config_file\":{},\"configured\":{},\"effective\":{},\"source\":{}}}",
                    jstr(&file.to_string_lossy()),
                    jstr(&configured),
                    jstr(&effective),
                    jstr(source)
                );
            } else {
                println!("配置文件 : {}", file.display());
                println!(
                    "已配置   : {}",
                    if configured.trim().is_empty() {
                        "（未设置，用系统默认）".to_string()
                    } else {
                        configured
                    }
                );
                println!("当前生效 : {}", effective);
                println!("来源     : {}", source);
            }
            // 界面「打开数据目录」的等价命令；--json 时仍会把路径打出来
            if has_flag(args, "--open") {
                #[cfg(windows)]
                membase_win::win::open_folder(&effective);
            }
        }
        "set" => {
            let dir = match pos.get(1) {
                Some(d) => d.to_string(),
                None => {
                    eprintln!("用法: MemBase.exe config set <目录>");
                    std::process::exit(EXIT_USAGE);
                }
            };
            if let Err(e) = fs::create_dir_all(&dir) {
                eprintln!("[x] 建目录失败: {}", e);
                std::process::exit(EXIT_IO);
            }
            let new_cfg = cfg::Config {
                data_root: Some(dir.clone()),
            };
            match cfg::save(&new_cfg) {
                Ok(path) => {
                    if json {
                        println!(
                            "{{\"ok\":true,\"config_file\":{},\"data_root\":{}}}",
                            jstr(&path.to_string_lossy()),
                            jstr(&dir)
                        );
                    } else {
                        println!("已设置数据目录：{}", dir);
                        println!("  写入 : {}", path.display());
                        println!("  提示 : 新目录还没有基准线的话，用 `MemBase.exe baseline capture`");
                        println!("         或 GUI 的「扫描当前 → 设为基准」建一份");
                    }
                }
                Err(e) => {
                    eprintln!("[x] 写配置失败: {}", e);
                    std::process::exit(EXIT_IO);
                }
            }
        }
        "clear" | "reset" => {
            match cfg::save(&cfg::Config::default()) {
                Ok(_) => {
                    let eff = cfg::data_root(None);
                    if json {
                        println!(
                            "{{\"ok\":true,\"data_root\":{},\"source\":{}}}",
                            jstr(&eff),
                            jstr("系统默认")
                        );
                    } else {
                        println!("已清除配置，恢复系统默认：{}", eff);
                    }
                }
                Err(e) => {
                    eprintln!("[x] 写配置失败: {}", e);
                    std::process::exit(EXIT_IO);
                }
            }
        }
        other => {
            eprintln!("未知子命令: config {}", other);
            std::process::exit(EXIT_USAGE);
        }
    }
}

// ---------------- 基准线：查看 / 校验 / 导出 / 导入（跨平台） ----------------

fn cmd_baseline(args: &[String]) {
    use membase_core::bundle;
    let root = root_arg(args);
    let json = want_json(args);
    let sub = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .unwrap_or("info");

    match sub {
        #[cfg(windows)]
        "capture" => {
            // 立刻扫描当前状态，直接落成一份新基准（不用等开机 60 秒）
            let label = arg_value(args, "--label").unwrap_or_else(|| "now".to_string());
            let safe: String = label
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
                .collect();
            let label = if safe.is_empty() { "now".to_string() } else { safe };

            let snap = snapshot::snapshot_now(true);
            let run_dir = snapshot::new_run_dir(&root);
            let sub_dir = run_dir.join(&label);
            if let Err(e) = snapshot::write_snapshot(&sub_dir, &snap) {
                eprintln!("[x] 写快照失败: {}", e);
                std::process::exit(EXIT_IO);
            }
            if json {
                println!(
                    "{{\"ok\":true,\"dir\":{},\"captured_at\":{},\"uptime_sec\":{},\"used_mb\":{},\"load_pct\":{},\"process_count\":{},\"service_count\":{},\"active\":true}}",
                    jstr(&sub_dir.to_string_lossy()),
                    jstr(&snap.captured_local),
                    snap.uptime_s,
                    snap.used_phys_mb,
                    snap.load_pct,
                    snap.procs.len(),
                    snap.svcs.len()
                );
            } else {
                println!("已扫描当前状态并设为基准");
                println!("  目录 : {}", sub_dir.display());
                println!(
                    "  内容 : {} 个进程 / {} 个服务，已用 {} MB / 占用率 {}%",
                    snap.procs.len(),
                    snap.svcs.len(),
                    snap.used_phys_mb,
                    snap.load_pct
                );
                println!("  注意 : 这是「最新」的一份，status / list / trim 会立刻以它为基准");
            }
        }

        #[cfg(not(windows))]
        "capture" => {
            eprintln!("[x] baseline capture 需要 Windows（要读进程和服务表）");
            std::process::exit(EXIT_USAGE);
        }

        "info" | "verify" => {
            let dir = match bundle::find_latest_baseline(&root) {
                Some(d) => d,
                None => {
                    eprintln!("[x] 没有基准快照：{}（先跑 MemBase.exe collect）", root.display());
                    std::process::exit(EXIT_NO_BASELINE);
                }
            };
            let captured =
                bundle::snapshot_meta(&dir, "CapturedAtLocal").unwrap_or_else(|| "未知".into());
            let uptime = bundle::snapshot_meta(&dir, "UptimeSeconds").unwrap_or_else(|| "0".into());

            let info = match bundle::inspect_snapshot(&dir) {
                Ok(i) => i,
                Err(e) => {
                    if json {
                        println!("{{\"ok\":false,\"dir\":{},\"error\":{}}}", jstr(&dir.to_string_lossy()), jstr(&e));
                    } else {
                        eprintln!("[x] 校验失败: {}", e);
                    }
                    std::process::exit(EXIT_ACTION_FAILED);
                }
            };

            // verify 额外做一次内容解析
            let mut problems: Vec<String> = Vec::new();
            if sub == "verify" {
                let p = dir.join("processes.csv");
                let procs = fs::read_to_string(&p).unwrap_or_default();
                let rows = core::csv_parse(&procs);
                if rows.len() < 2 {
                    problems.push("processes.csv 没有数据行".into());
                }
                let svc = fs::read_to_string(dir.join("services.csv")).unwrap_or_default();
                let srows = core::csv_parse(&svc);
                if srows.len() < 2 {
                    problems.push("services.csv 没有数据行".into());
                }
                let by = core::parse_by_name(
                    &fs::read_to_string(dir.join("processes_by_name.csv")).unwrap_or_default(),
                );
                if by.is_empty() {
                    problems.push("processes_by_name.csv 解析不出进程名".into());
                }
            }

            let ok = problems.is_empty();
            if json {
                let files: Vec<String> = info
                    .files
                    .iter()
                    .map(|(n, b)| format!("{{\"name\":{},\"bytes\":{}}}", jstr(n), b))
                    .collect();
                let probs: Vec<String> = problems.iter().map(|p| jstr(p)).collect();
                println!(
                    "{{\"ok\":{},\"dir\":{},\"captured_at\":{},\"uptime_sec\":{},\"bytes\":{},\"crc32\":\"{:08x}\",\"files\":[{}],\"problems\":[{}]}}",
                    ok,
                    jstr(&dir.to_string_lossy()),
                    jstr(&captured),
                    uptime.trim(),
                    info.total_bytes,
                    info.crc,
                    files.join(","),
                    probs.join(",")
                );
            } else {
                println!("当前基准 : {}", dir.display());
                println!("采集时间 : {}（开机第 {} 秒）", captured, uptime.trim());
                println!(
                    "体积     : {} 字节 / {} 个文件 / crc32 {:08x}",
                    info.total_bytes,
                    info.files.len(),
                    info.crc
                );
                for (n, b) in &info.files {
                    println!("  {:<24} {:>9} 字节", n, b);
                }
                if sub == "verify" {
                    if ok {
                        println!("校验结果 : 通过");
                    } else {
                        println!("校验结果 : 有问题");
                        for p in &problems {
                            println!("  - {}", p);
                        }
                    }
                }
            }
            if !ok {
                std::process::exit(EXIT_ACTION_FAILED);
            }
        }

        "export" => {
            let dir = match bundle::find_latest_baseline(&root) {
                Some(d) => d,
                None => {
                    eprintln!("[x] 没有基准快照：{}", root.display());
                    std::process::exit(EXIT_NO_BASELINE);
                }
            };
            let base_captured =
                bundle::snapshot_meta(&dir, "CapturedAtLocal").unwrap_or_default();
            let stamp = {
                let s = bundle::stamp_from_local(&base_captured);
                if s.is_empty() {
                    bundle::timestamp()
                } else {
                    s
                }
            };
            let out = match arg_value(args, "--to") {
                Some(p) => PathBuf::from(p),
                None => root
                    .join("exports")
                    .join(format!("baseline_{}.mbz", stamp)),
            };
            match bundle::export_baseline(&dir, &out) {
                Ok(r) => {
                    if json {
                        println!(
                            "{{\"ok\":true,\"from\":{},\"file\":{},\"bytes\":{},\"files\":{},\"crc32\":\"{:08x}\"}}",
                            jstr(&dir.to_string_lossy()),
                            jstr(&r.path.to_string_lossy()),
                            r.bytes,
                            r.files,
                            r.crc
                        );
                    } else {
                        println!("已导出基准线");
                        println!("  来源 : {}", dir.display());
                        println!("  文件 : {}（{} 字节）", r.path.display(), r.bytes);
                        println!("  内容 : {} 个文件，crc32 {:08x}", r.files, r.crc);
                    }
                }
                Err(e) => {
                    eprintln!("[x] 导出失败: {}", e);
                    std::process::exit(EXIT_IO);
                }
            }
        }

        "import" => {
            let pos = positionals(args, &["--out", "--label", "--to", "--task"]);
            let pkg = match pos.iter().find(|p| !matches!(p.as_str(), "import")) {
                Some(p) => PathBuf::from(p),
                None => {
                    eprintln!("用法: MemBase.exe baseline import <包文件.mbz> [--label NAME] [--force]");
                    std::process::exit(EXIT_USAGE);
                }
            };
            if !pkg.exists() {
                eprintln!("[x] 包不存在: {}", pkg.display());
                std::process::exit(EXIT_IO);
            }
            // 先校验包：坏包绝不落地
            let (nfiles, crc, captured) = match bundle::verify_package(&pkg) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("[x] 包校验失败，未导入: {}", e);
                    std::process::exit(EXIT_ACTION_FAILED);
                }
            };
            let label = arg_value(args, "--label").unwrap_or_else(|| "imported".into());
            match bundle::import_baseline(&root, &pkg, &label) {
                Ok(r) => {
                    if json {
                        println!(
                            "{{\"ok\":true,\"package\":{},\"dir\":{},\"files\":{},\"crc32\":\"{:08x}\",\"captured_at\":{},\"active\":true}}",
                            jstr(&pkg.to_string_lossy()),
                            jstr(&r.dir.to_string_lossy()),
                            r.files,
                            r.crc,
                            jstr(&captured)
                        );
                    } else {
                        println!("已导入基准线（{} 个文件，crc32 {:08x}）", nfiles, crc);
                        println!("  包   : {}", pkg.display());
                        println!("  落地 : {}", r.dir.display());
                        println!("  采集 : {}", captured);
                        println!("  注意 : 导入的快照是「最新」的一份，MemBase.exe status/trim 会以它为基准");
                    }
                }
                Err(e) => {
                    eprintln!("[x] 导入失败: {}", e);
                    std::process::exit(EXIT_IO);
                }
            }
        }

        other => {
            eprintln!("未知子命令: baseline {}", other);
            std::process::exit(EXIT_USAGE);
        }
    }
}

fn protect_file(root: &Path) -> PathBuf {
    root.join("protect.txt")
}

fn read_whitelist(root: &Path) -> Vec<String> {
    fs::read_to_string(protect_file(root))
        .map(|t| core::parse_whitelist(&t))
        .unwrap_or_default()
}

fn write_whitelist(root: &Path, items: &[String]) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(protect_file(root), core::serialize_whitelist(items))
}

// ---------------- JSON 小工具 ----------------

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            // 非 ASCII 一律转成 \uXXXX：这样 JSON 输出是纯 ASCII，
            // 不管宿主用 GBK 还是 UTF-8 解码都不会炸
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    o.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn json_str_field(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let i = text.find(&pat)?;
    let rest = &text[i + pat.len()..];
    let q1 = rest.find('"')?;
    let rest2 = &rest[q1 + 1..];
    let q2 = rest2.find('"')?;
    Some(rest2[..q2].to_string())
}

fn json_num_field(text: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\"", key);
    let i = text.find(&pat)?;
    let rest = text[i + pat.len()..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| c == ',' || c == '\n' || c == '}')
        .unwrap_or(rest.len());
    rest[..end].trim().trim_matches('"').parse().ok()
}

// ---------------- 短命项（双采样对比） ----------------

/// 在 root 下找「最近一次带 ≥2 个采样的 run_*」，按开机秒数排序返回各 label。
#[cfg(windows)]
fn latest_multi_sample_run(root: &Path) -> Option<(PathBuf, Vec<(String, PathBuf)>)> {
    let mut best: Option<(std::time::SystemTime, PathBuf, Vec<(String, PathBuf)>)> = None;
    for e in fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        if !p.is_dir() || !name.starts_with("run_") {
            continue;
        }
        let mut labels: Vec<(String, PathBuf)> = Vec::new();
        let mut newest = std::time::UNIX_EPOCH;
        if let Ok(rd) = fs::read_dir(&p) {
            for s in rd.flatten() {
                let sp = s.path();
                if sp.join("system.json").exists() {
                    let mt = fs::metadata(sp.join("system.json"))
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH);
                    if mt > newest {
                        newest = mt;
                    }
                    let l = s.file_name().to_string_lossy().to_string();
                    labels.push((l, sp));
                }
            }
        }
        if labels.len() < 2 {
            continue;
        }
        // T+30s / T+300s 这类 label 按秒数排；排不出来就按修改时间
        labels.sort_by_key(|(l, _)| boot_offset_of(l).unwrap_or(u64::MAX));
        if best.as_ref().map(|(t, _, _)| newest > *t).unwrap_or(true) {
            best = Some((newest, p, labels));
        }
    }
    best.map(|(_, dir, labels)| (dir, labels))
}

#[cfg(windows)]
fn boot_offset_of(label: &str) -> Option<u64> {
    let s = label.strip_prefix("T+")?;
    s.trim_end_matches('s').parse::<u64>().ok()
}

#[cfg(windows)]
fn cmd_transient(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let min_mb = num_arg(args, "--min-mb", 5.0);

    let Some((run_dir, labels)) = latest_multi_sample_run(&root) else {
        if json {
            println!(
                "{{\"ok\":false,\"error\":\"no_dual_sample\",\"hint\":\"先注册双采样：MemBase.exe install --at 30 --also 300\",\"transient\":[],\"appeared\":[]}}"
            );
        } else {
            eprintln!(
                "[x] 没有双采样快照：{} 下每个 run_* 都只有一个采样点。\n    先注册双采样：MemBase.exe install --at 30 --also 300",
                root.display()
            );
        }
        std::process::exit(EXIT_NO_BASELINE);
    };
    let (first_label, first_dir) = &labels[0];
    let (last_label, last_dir) = &labels[labels.len() - 1];

    let read = |dir: &Path| {
        core::parse_by_name(
            &fs::read_to_string(dir.join("processes_by_name.csv")).unwrap_or_default(),
        )
    };
    let a = read(first_dir);
    let b = read(last_dir);

    let mut transient: Vec<(String, f64, u32)> = Vec::new();
    for (name, agg) in &a {
        if !b.contains_key(name) && agg.ws_mb >= min_mb {
            transient.push((name.clone(), agg.ws_mb, agg.instances));
        }
    }
    transient.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut appeared: Vec<(String, f64, u32)> = Vec::new();
    for (name, agg) in &b {
        if !a.contains_key(name) && agg.ws_mb >= min_mb {
            appeared.push((name.clone(), agg.ws_mb, agg.instances));
        }
    }
    appeared.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));

    let total: f64 = transient.iter().map(|t| t.1).sum();
    let total = (total * 100.0).round() / 100.0;

    if json {
        let j = |v: &[(String, f64, u32)]| {
            v.iter()
                .map(|(n, mb, inst)| {
                    format!(
                        "{{\"name\":{},\"mem_mb\":{:.1},\"instances\":{}}}",
                        jstr(n),
                        mb,
                        inst
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "{{\"run\":{},\"first\":{},\"second\":{},\"transient_mb\":{:.1},\"transient\":[{}],\"appeared\":[{}]}}",
            jstr(&run_dir.to_string_lossy()),
            jstr(first_label),
            jstr(last_label),
            total,
            j(&transient),
            j(&appeared)
        );
        return;
    }

    println!("双采样：{}", run_dir.display());
    println!("  {} → {}", first_label, last_label);
    if transient.is_empty() && appeared.is_empty() {
        println!("  两点之间没有出现/消失的进程（阈值 {:.0} MB）", min_mb);
        return;
    }
    if !transient.is_empty() {
        println!(
            "\n短命项（{} 有、{} 没有）：{} 项，合计 {:.1} MB",
            first_label,
            last_label,
            transient.len(),
            total
        );
        println!("  {:<34} {:>9}  {}", "进程名", "内存", "实例");
        for (n, mb, inst) in &transient {
            println!("  {:<34} {:>7.1} MB  {}", elide_cli(n, 34), mb, inst);
        }
        println!("\n这些是「起了又退」的，不该算进常驻基线；判断瘦身收益时要扣掉。");
    }
    if !appeared.is_empty() {
        println!(
            "\n后出现的（{} 才起来）：{} 项",
            last_label,
            appeared.len()
        );
        for (n, mb, inst) in appeared.iter().take(15) {
            println!("  {:<34} {:>7.1} MB  {}", elide_cli(n, 34), mb, inst);
        }
    }
}

// ---------------- 基准 / 实时数据（Windows） ----------------

#[cfg(windows)]
struct Baseline {
    dir: PathBuf,
    captured: String,
    uptime: u64,
    procs: BTreeMap<String, core::ProcAgg>,
    svcs: BTreeMap<String, String>,
}

#[cfg(windows)]
fn load_baseline(root: &Path) -> Result<Baseline, i32> {
    load_baseline_label(root, None)
}

/// 按 label 取基准（`T+30s` / `T+300s` / `now`）；`None` = 最新那份
#[cfg(windows)]
fn load_baseline_label(root: &Path, label: Option<&str>) -> Result<Baseline, i32> {
    let dir = match compare::find_baseline(root, label) {
        Some(d) => d,
        None => {
            match label {
                Some(l) => eprintln!(
                    "[x] 在 {} 下找不到 label 为 {} 的快照（用 `MemBase.exe status` 看现有采样点）",
                    root.display(),
                    l
                ),
                None => eprintln!(
                    "[x] 没有基准快照：{}（先跑 MemBase.exe collect）",
                    root.display()
                ),
            }
            return Err(EXIT_NO_BASELINE);
        }
    };
    let sysj = fs::read_to_string(dir.join("system.json")).unwrap_or_default();
    let captured = json_str_field(&sysj, "CapturedAtLocal").unwrap_or_else(|| "未知".to_string());
    let uptime = json_num_field(&sysj, "UptimeSeconds").unwrap_or(0.0) as u64;
    let procs = core::parse_by_name(
        &fs::read_to_string(dir.join("processes_by_name.csv")).unwrap_or_default(),
    );
    let svcs =
        core::parse_service_states(&fs::read_to_string(dir.join("services.csv")).unwrap_or_default());
    Ok(Baseline {
        dir,
        captured,
        uptime,
        procs,
        svcs,
    })
}

#[cfg(windows)]
struct LiveData {
    procs: Vec<core::LiveProc>,
    svcs: Vec<core::LiveSvc>,
    used_mb: u64,
    load_pct: u32,
    proc_count: usize,
}

#[cfg(windows)]
fn live_now() -> LiveData {
    let snap = snapshot::snapshot_now(true);
    let procs = snap
        .procs
        .iter()
        .map(|p| core::LiveProc {
            name: p.name.clone(),
            pid: p.pid,
            ws_mb: p.ws_mb.unwrap_or(0.0),
            priv_mb: p.priv_mb.unwrap_or(0.0),
            unknown: if p.ws_mb.is_none() { 1 } else { 0 },
            path: p.path.clone(),
            created: p.created,
        })
        .collect();
    let svcs = snap
        .svcs
        .iter()
        .map(|s| core::LiveSvc {
            name: s.name.clone(),
            state: s.state.clone(),
            start_mode: s.start_mode.clone(),
            pid: s.pid,
            ws_mb: s.host_ws.unwrap_or(0.0),
            path: s.path.clone(),
        })
        .collect();
    LiveData {
        procs,
        svcs,
        used_mb: snap.used_phys_mb,
        load_pct: snap.load_pct,
        proc_count: snap.procs.len(),
    }
}

#[cfg(windows)]
fn build_current_plan(root: &Path, min_mb: f64) -> Result<(Baseline, Vec<core::PlanItem>), i32> {
    let base = load_baseline(root)?;
    let live = live_now();
    let protected = win::protected_pids();
    let wl = read_whitelist(root);
    let plan = core::build_plan(
        &base.procs,
        &base.svcs,
        &live.procs,
        &live.svcs,
        &protected,
        &wl,
        min_mb,
    );
    Ok((base, plan))
}

#[cfg(windows)]
fn filter_plan(
    plan: Vec<core::PlanItem>,
    only: &[String],
    exclude: &[String],
) -> Vec<core::PlanItem> {
    plan.into_iter()
        .filter(|p| {
            if !only.is_empty()
                && !only
                    .iter()
                    .any(|pat| core::wildcard_match(&p.name, pat))
            {
                return false;
            }
            if exclude
                .iter()
                .any(|pat| core::wildcard_match(&p.name, pat))
            {
                return false;
            }
            true
        })
        .collect()
}

#[cfg(windows)]
fn item_json(
    it: &core::PlanItem,
    include_caution: bool,
    heal: &BTreeMap<String, String>,
) -> String {
    let pids: Vec<String> = it.pids().iter().map(|p| p.to_string()).collect();
    // members 带创建时间：调用方可以据此判断「我看到的这个实例」是否还在，
    // 而不是只拿一串随时可能被系统复用的 PID。
    let members: Vec<String> = it
        .members
        .iter()
        .map(|m| {
            format!(
                "{{\"pid\":{},\"created\":{},\"created_at\":{}}}",
                m.pid,
                m.created,
                jstr(&win::filetime_local_string(m.created))
            )
        })
        .collect();
    format!(
        "{{\"kind\":\"{}\",\"name\":{},\"pids\":[{}],\"members\":[{}],\"instances\":{},\"mem_mb\":{:.1},\"host_pid\":{},\"shared_host\":{},\"reclaim_mb\":{:.1},\"self_heal\":{},\"tier\":\"{}\",\"protect\":{},\"protect_by\":{},\"actionable\":{},\"start_mode\":{},\"reason\":{},\"path\":{}}}",
        core::kind_name(it.kind),
        jstr(&it.name),
        pids.join(","),
        members.join(","),
        it.instances,
        it.mem_mb,
        it.host_pid,
        it.shared_host,
        it.reclaim_mb,
        // 曾实测到「杀掉又回来」的进程：restart 对它们没有净收益
        it.kind == Kind::Process && heal.contains_key(&it.name),
        core::tier_name(it.tier),
        it.protect,
        jstr(&it.protect_by),
        it.actionable(include_caution),
        jstr(&it.start_mode),
        jstr(&it.reason),
        jstr(&it.path)
    )
}

#[cfg(windows)]
fn plan_counts(plan: &[core::PlanItem], include_caution: bool) -> (usize, usize, usize, usize, f64) {
    let suggested = plan.iter().filter(|p| p.tier == Tier::Suggested).count();
    let caution = plan.iter().filter(|p| p.tier == Tier::Caution).count();
    let blocked = plan.iter().filter(|p| p.tier == Tier::Blocked).count();
    let (n, mb) = core::plan_totals(plan, include_caution);
    (suggested, caution, blocked, n, mb)
}

#[cfg(windows)]
fn tier_label(t: Tier) -> &'static str {
    match t {
        Tier::Suggested => "建议",
        Tier::Caution => "谨慎",
        Tier::Blocked => "关键",
    }
}

// ---------------- 子命令 ----------------

/// 同一个 run 目录下的其它采样点（双采样时会有 T+30s / T+300s 两个）
#[cfg(windows)]
fn sibling_labels(dir: &Path) -> Vec<String> {
    let mut out: Vec<(u64, String)> = Vec::new();
    let Some(run) = dir.parent() else {
        return Vec::new();
    };
    if let Ok(rd) = fs::read_dir(run) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join("system.json").exists() {
                let l = e.file_name().to_string_lossy().to_string();
                out.push((boot_offset_of(&l).unwrap_or(u64::MAX), l));
            }
        }
    }
    out.sort();
    out.into_iter().map(|(_, l)| l).collect()
}

#[cfg(windows)]
fn cmd_status(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let base = match load_baseline(&root) {
        Ok(b) => b,
        Err(code) => std::process::exit(code),
    };
    let live = live_now();
    let (_, plan) = match build_current_plan(&root, num_arg(args, "--min-mb", 10.0)) {
        Ok(x) => x,
        Err(code) => std::process::exit(code),
    };
    let (sg, ca, bl, act, act_mb) = plan_counts(&plan, false);
    let heal = read_selfheal(&root);

    if json {
        println!(
            "{{\"baseline\":{{\"dir\":{},\"captured_at\":{},\"uptime_sec\":{},\"labels\":[{}]}},\"live\":{{\"used_mb\":{},\"load_pct\":{},\"process_count\":{}}},\"outliers\":{{\"total\":{},\"suggested\":{},\"caution\":{},\"blocked\":{},\"actionable\":{},\"actionable_mb\":{:.1}}},\"is_admin\":{},\"self_heal\":[{}]}}",
            jstr(&base.dir.to_string_lossy()),
            jstr(&base.captured),
            base.uptime,
            sibling_labels(&base.dir)
                .iter()
                .map(|l| jstr(l))
                .collect::<Vec<_>>()
                .join(","),
            live.used_mb,
            live.load_pct,
            live.proc_count,
            plan.len(),
            sg,
            ca,
            bl,
            act,
            act_mb,
            win::is_admin(),
            heal.keys()
                .map(|k| jstr(k))
                .collect::<Vec<_>>()
                .join(",")
        );
        return;
    }

    println!("基准快照 : {}", base.dir.display());
    println!(
        "           采集于 {}（开机第 {} 秒）",
        base.captured, base.uptime
    );
    let labels = sibling_labels(&base.dir);
    if labels.len() > 1 {
        println!(
            "           同一次开机的采样点：{}（compare --label 可切换）",
            labels.join(" / ")
        );
    }
    println!(
        "当前内存 : {} MB / 占用率 {}% / {} 个进程",
        live.used_mb, live.load_pct, live.proc_count
    );
    println!(
        "线外条目 : {} 项（建议 {} / 谨慎 {} / 关键 {}）",
        plan.len(),
        sg,
        ca,
        bl
    );
    println!("可处理   : {} 项，预计回收约 {:.0} MB（按宿主去重）", act, act_mb);
    if !heal.is_empty() {
        let names: Vec<String> = heal.keys().cloned().collect();
        println!("自愈型   : {} 个（{}）", names.len(), names.join(" / "));
        println!("           杀掉会被外壳重新拉起，restart 对它们没有净收益");
    }
    println!(
        "权限     : {}",
        if win::is_admin() {
            "管理员"
        } else {
            "普通用户（部分动作会失败）"
        }
    );
}

#[cfg(windows)]
fn cmd_list(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let include_caution = has_flag(args, "--include-caution");
    let min_mb = num_arg(args, "--min-mb", 10.0);
    let (base, plan) = match build_current_plan(&root, min_mb) {
        Ok(x) => x,
        Err(code) => std::process::exit(code),
    };
    let plan = filter_plan(
        plan,
        &list_arg(args, "--only"),
        &list_arg(args, "--exclude"),
    );
    let (sg, ca, bl, act, act_mb) = plan_counts(&plan, include_caution);
    let heal = read_selfheal(&root);
    let all = has_flag(args, "--all");

    if json {
        let items: Vec<String> = plan
            .iter()
            .filter(|p| all || p.tier != Tier::Blocked)
            .map(|p| item_json(p, include_caution, &heal))
            .collect();
        println!(
            "{{\"baseline\":{{\"dir\":{},\"captured_at\":{},\"uptime_sec\":{}}},\"counts\":{{\"total\":{},\"suggested\":{},\"caution\":{},\"blocked\":{},\"actionable\":{},\"actionable_mb\":{:.1}}},\"items\":[{}]}}",
            jstr(&base.dir.to_string_lossy()),
            jstr(&base.captured),
            base.uptime,
            plan.len(),
            sg,
            ca,
            bl,
            act,
            act_mb,
            items.join(",")
        );
        return;
    }

    println!("基准：{}", base.dir.display());
    println!(
        "线外 {} 项：建议 {} / 谨慎 {} / 关键 {}；可处理 {} 项（约 {:.0} MB）",
        plan.len(),
        sg,
        ca,
        bl,
        act,
        act_mb
    );
    println!();
    for p in &plan {
        if p.tier == Tier::Blocked && !all {
            continue;
        }
        let mark = if p.protect {
            "[已保护]"
        } else if p.tier == Tier::Blocked {
            "[关键]  "
        } else {
            "        "
        };
        let tail = if p.path.is_empty() { &p.reason } else { &p.path };
        println!(
            "{} {} {:<9} {:<28} {:>8.0} MB  {}",
            mark,
            core::kind_name(p.kind),
            tier_label(p.tier),
            p.name,
            p.mem_mb,
            tail
        );
    }
}

#[cfg(windows)]
fn cmd_trim(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let include_caution = has_flag(args, "--include-caution");
    let min_mb = num_arg(args, "--min-mb", 10.0);
    let max = num_arg(args, "--max", 20.0) as usize;

    let (base, plan) = match build_current_plan(&root, min_mb) {
        Ok(x) => x,
        Err(code) => std::process::exit(code),
    };
    let plan = filter_plan(
        plan,
        &list_arg(args, "--only"),
        &list_arg(args, "--exclude"),
    );

    if let Some(path) = arg_value(args, "--plan-out") {
        let mut csv =
            String::from("Kind,Name,Pids,Instances,MemMB,Tier,Protect,Actionable,Reason\n");
        for p in &plan {
            csv.push_str(&format!(
                "{},{},{},{},{:.1},{},{},{},{}\n",
                core::kind_name(p.kind),
                p.name,
                p.pids()
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join("|"),
                p.instances,
                p.mem_mb,
                core::tier_name(p.tier),
                p.protect,
                p.actionable(include_caution),
                p.reason.replace(',', ";")
            ));
        }
        if let Err(e) = fs::write(&path, csv) {
            eprintln!("[x] 写计划失败: {}", e);
            std::process::exit(EXIT_IO);
        }
        println!("计划已写入 {}", path);
        return;
    }

    let targets: Vec<&core::PlanItem> = core::plan_actionable(&plan, include_caution);
    // 按宿主去重后的「真正能兑现」的量：进程项与同名服务项常指向同一个进程，
    // 共享宿主的服务项 reclaim 已经是 0（详见 core::plan_reclaim）
    let est_mb = core::plan_reclaim(&targets);
    let heal = read_selfheal(&root);

    if !execute {
        if json {
            let items: Vec<String> = targets
                .iter()
                .map(|p| item_json(p, include_caution, &heal))
                .collect();
            println!(
                "{{\"mode\":\"dry_run\",\"baseline\":{},\"count\":{},\"mem_mb\":{:.1},\"items\":[{}]}}",
                jstr(&base.dir.to_string_lossy()),
                targets.len(),
                est_mb,
                items.join(",")
            );
            return;
        }
        println!(
            "[dry-run] 将处理 {} 项，约 {:.0} MB（加 --execute 才真正执行）",
            targets.len(),
            est_mb
        );
        let proc_count = targets.iter().filter(|p| p.kind == Kind::Process).count();
        if proc_count > 0 {
            let grace = num_arg(args, "--grace-wait", 3.0).max(0.0);
            println!(
                "          其中 {} 项是进程：有窗口的先收关闭请求（等同点 ✕），{} 秒后才强制结束；服务直接停止",
                proc_count, grace
            );
        }
        for p in &targets {
            let tail = if p.path.is_empty() { &p.reason } else { &p.path };
            println!(
                "  {} {:<28} {:>7.0} MB  {}",
                core::kind_name(p.kind),
                p.name,
                p.mem_mb,
                tail
            );
        }
        return;
    }

    if targets.is_empty() {
        if json {
            println!("{{\"mode\":\"execute\",\"count\":0,\"ok\":0,\"failed\":0,\"results\":[]}}");
        } else {
            println!("没有需要处理的项目。");
        }
        return;
    }
    if max > 0 && targets.len() > max {
        eprintln!(
            "[!] 本次拟处理 {} 项，超过上限 {}（用 --max 0 或更大的值放开）",
            targets.len(),
            max
        );
        std::process::exit(EXIT_REFUSED);
    }
    // 先礼后兵：有窗口的先收 WM_CLOSE，等宽限期；没有窗口的直接终止。
    // --force 跳过礼貌阶段（旧行为），--no-force 则允许「关不掉就留着」。
    // 两个一起给时按 --force 处理：明确要求快的意图优先。
    let force_flag = has_flag(args, "--force");
    let grace_sec = num_arg(args, "--grace-wait", 3.0).max(0.0);
    let opts = close::CloseOptions {
        graceful: !force_flag,
        grace_ms: (grace_sec * 1000.0) as u32,
        force: force_flag || !has_flag(args, "--no-force"),
    };

    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("[!] 非交互环境必须显式加 --yes 才能执行");
            std::process::exit(EXIT_REFUSED);
        }
        let mode_desc = if !opts.graceful {
            "直接终止（--force）".to_string()
        } else if opts.force {
            format!("有窗口的先发关闭请求，{} 秒后仍未退出的才强制结束", grace_sec)
        } else {
            format!("有窗口的先发关闭请求，{} 秒后仍未退出的保留不动", grace_sec)
        };
        println!(
            "即将处理 {} 项（约 {:.0} MB）。{}。输入 yes 继续，其他取消：",
            targets.len(),
            est_mb,
            mode_desc
        );
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim() != "yes" {
            println!("已取消。");
            return;
        }
    }

    let protected = win::protected_pids();
    let mut results: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut fail = 0usize;

    // 所有进程组一起提交：共用同一个宽限期，20 组不会变成 20 × 3 秒
    let proc_groups: Vec<(String, Vec<close::Target>)> = targets
        .iter()
        .filter(|p| p.kind == Kind::Process)
        .map(|p| {
            (
                p.name.clone(),
                p.members
                    .iter()
                    .map(|m| close::Target {
                        pid: m.pid,
                        created: m.created,
                    })
                    .collect(),
            )
        })
        .collect();
    let views: Vec<close::Group> = proc_groups
        .iter()
        .map(|(n, t)| close::Group {
            name: n,
            targets: t,
        })
        .collect();
    let proc_reports = close::close_groups(&views, opts, &protected);

    let mut proc_idx = 0usize;
    for p in &targets {
        match p.kind {
            Kind::Process => {
                let rep = &proc_reports[proc_idx];
                proc_idx += 1;
                let acted = rep.closed + rep.forced;
                if rep.done() {
                    ok += 1;
                } else {
                    fail += 1;
                }
                lines.push(rep.one_line(&p.name));
                results.push(format!(
                    "{{\"kind\":\"process\",\"name\":{},\"ok\":{},\"killed\":{},\"requested\":{},\"closed\":{},\"forced\":{},\"exited\":{},\"skipped\":{},\"remaining\":{},\"message\":{}}}",
                    jstr(&p.name),
                    rep.done(),
                    acted,
                    rep.requested,
                    rep.closed,
                    rep.forced,
                    rep.exited,
                    rep.skipped,
                    rep.remaining,
                    jstr(&rep.one_line(&p.name))
                ));
            }
            Kind::Service => match win::stop_service(&p.name) {
                Ok(()) => {
                    ok += 1;
                    lines.push(format!("{}: 已停止", p.name));
                    results.push(format!(
                        "{{\"kind\":\"service\",\"name\":{},\"ok\":true,\"message\":{}}}",
                        jstr(&p.name),
                        jstr("已停止")
                    ));
                }
                Err(e) => {
                    fail += 1;
                    lines.push(format!("{}: {}", p.name, e));
                    results.push(format!(
                        "{{\"kind\":\"service\",\"name\":{},\"ok\":false,\"message\":{}}}",
                        jstr(&p.name),
                        jstr(&e)
                    ));
                }
            },
        }
    }

    if json {
        println!(
            "{{\"mode\":\"execute\",\"baseline\":{},\"count\":{},\"ok\":{},\"failed\":{},\"results\":[{}]}}",
            jstr(&base.dir.to_string_lossy()),
            targets.len(),
            ok,
            fail,
            results.join(",")
        );
    } else {
        println!("执行完成：成功 {} 项，失败 {} 项", ok, fail);
        for r in &lines {
            println!("  {}", r);
        }
    }

    if fail > 0 {
        std::process::exit(EXIT_ACTION_FAILED);
    }
}

// ---------------- 自描述的命令表 ----------------

const COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("status", "概况：基准线 + 当前内存 + 线外计数", "", "windows"),
    ("list", "列出线外进程/服务（只读）", "--all --min-mb --only --exclude --include-caution", "windows"),
    ("plan", "只生成线外计划，不执行", "--min-mb --only --exclude --include-caution --to FILE", "windows"),
    ("trim", "恢复基准：先礼后兵地关掉线外项", "--execute --yes --include-caution --max --min-mb --only --exclude --plan-out --grace-wait --force --no-force", "windows"),
    ("slim", "基线瘦身：砍掉基准里开机就不需要的服务（禁用 / 卸载 Store 应用）", "--min-mb --include-caution --only --exclude --execute --yes", "windows"),
    ("restart", "局部重启：关掉再拉起，报告实测回收量", "<进程名|PID|服务名> --execute --yes --wait --path --grace-wait --force --no-force", "windows"),
    ("feature", "已知功能开关（只写 HKCU）：列出 / 关掉 / 撤销", "list|off|on <key> --execute --yes --no-restart --wait", "windows"),
    ("startup", "启动项扫描：注册表 Run / 启动文件夹 / 非 Microsoft 计划任务", "list|disable|enable <名称> --execute --yes", "windows"),
    ("transient", "双采样对比：列出只在前一次出现的短命项", "--min-mb --json", "windows"),
    ("kill", "原子操作：终止指定进程（名字或 PID）", "<name|pid> --execute --out --grace-wait --force --no-force", "windows"),
    ("svc", "原子操作：停止/启动/禁止自启单个服务", "stop|start|disable|enable <service> --execute", "windows"),
    ("protect", "保护名单（勾选保护 = 永不处理）", "list|add|remove|clear [名字...]", "windows"),
    ("collect", "采集一次快照", "--out --at --include-self --self-destruct --task", "windows"),
    ("compare", "和最近一次基准做 diff", "--min-mb --exclude", "windows"),
    ("baseline", "基准线：查看/校验/采集当前态/导出/导入", "info|verify|capture|export|import [包] [--to FILE] [--label NAME]", "all"),
    ("install", "注册开机计划任务", "--out --at --task", "windows"),
    ("uninstall", "删除开机计划任务", "--task", "windows"),
    ("layout", "桌面图标布局：保存/差异/恢复/修复缓存", "list|save|restore|diff|show|delete|status|probe|repair|dir|flags [名字] [--execute --yes --positions --open --snap-grid on|off]", "windows"),
    ("theme", "外观：查看/切换浅色深色", "get|set light|dark", "all"),
    ("screen", "息屏：只关显示器，不锁屏（热键退出 / 定时自动亮）", "off|status|duration|key [--seconds N --exit-key K --delay N --dry-run]", "windows"),
    ("ui", "打开界面（可指定落在哪一页）", "--page layouts|baseline|protect|boot|cli|tools|settings", "windows"),
    ("config", "数据目录配置：查看/修改/清除", "get|set <目录>|clear [--open]", "all"),
    ("commands", "列出所有命令与参数（本表）", "", "all"),
    ("mcp", "以 MCP 协议（stdio）服务，供 AI 直接调用", "", "windows"),
    ("version", "打印版本", "", "all"),
];

/// 界面动作表导出成 JSON：每个按钮的等价命令都在这里，
/// 这样「人能点的脚本一定能发」这件事是可被机器检查的。
#[cfg(windows)]
fn actions_json() -> String {
    layout::ACTIONS
        .iter()
        .map(|a| {
            format!(
                "{{\"key\":{},\"page\":{},\"label\":{},\"cli\":{},\"note\":{}}}",
                jstr(a.key),
                jstr(a.page),
                jstr(a.label),
                jstr(a.cli),
                jstr(a.note)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(not(windows))]
fn actions_json() -> String {
    String::new()
}

fn cmd_commands(args: &[String]) {
    let json = want_json(args);
    if json {
        let items: Vec<String> = COMMANDS
            .iter()
            .map(|(n, d, a, p)| {
                format!(
                    "{{\"name\":{},\"desc\":{},\"args\":{},\"platform\":{}}}",
                    jstr(n),
                    jstr(d),
                    jstr(a),
                    jstr(p)
                )
            })
            .collect();
        let actions = actions_json();
        println!(
            "{{\"version\":{},\"data_root\":{},\"exit_codes\":{{\"0\":\"ok\",\"1\":\"usage\",\"2\":\"no_baseline\",\"3\":\"io\",\"4\":\"action_failed\",\"5\":\"refused\"}},\"commands\":[{}],\"gui_actions\":[{}]}}",
            jstr(membase_core::VERSION),
            jstr(&core::default_root()),
            items.join(","),
            actions
        );
        return;
    }
    println!(
        "MemBase.exe {}  数据目录 {}",
        membase_core::VERSION,
        core::default_root()
    );
    println!();
    for (n, d, a, p) in COMMANDS {
        println!("  {:<10} {:<38} {}", n, d, p);
        if !a.is_empty() {
            println!("  {:<10}   {}", "", a);
        }
    }
    #[cfg(windows)]
    {
        println!();
        println!("界面动作 → 命令（界面「命令行」页同源）:");
        for a in layout::ACTIONS {
            println!("  {:<10} {:<22} {}", a.page, a.label, a.cli);
        }
    }
}

// ---------------- 原子操作：单进程 / 单服务 ----------------

#[cfg(windows)]
fn cmd_kill(args: &[String]) {
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let pos = positionals(args, &["--out"]);
    let target = match pos.first() {
        Some(t) => t.to_string(),
        None => {
            eprintln!("用法: MemBase.exe kill <进程名|PID> [--execute] [--grace-wait N] [--force|--no-force] [--json]");
            std::process::exit(EXIT_USAGE);
        }
    };
    let snap = snapshot::snapshot_now(true);
    let protected = win::protected_pids();

    let by_pid: Option<u32> = target.parse().ok();
    let hits: Vec<&snapshot::ProcRow> = snap
        .procs
        .iter()
        .filter(|p| match by_pid {
            Some(pid) => p.pid == pid,
            None => p.name.eq_ignore_ascii_case(&target),
        })
        .collect();

    if hits.is_empty() {
        if json {
            println!("{{\"ok\":false,\"target\":{},\"count\":0,\"message\":{}}}", jstr(&target), jstr("没找到匹配的进程"));
        } else {
            println!("没找到匹配的进程：{}", target);
        }
        std::process::exit(EXIT_NO_BASELINE);
    }

    // 护栏：关键进程 / 自身进程树
    let mut refused: Vec<String> = Vec::new();
    for p in &hits {
        if core::tier_for(Kind::Process, &p.name, &p.path) == Tier::Blocked {
            refused.push(format!("{} 属于关键系统进程", p.name));
        } else if protected.contains(&p.pid) {
            refused.push(format!("{} (pid {}) 属于本工具自身进程树", p.name, p.pid));
        }
    }
    if !refused.is_empty() {
        for r in &refused {
            eprintln!("[!] 拒绝: {}", r);
        }
        std::process::exit(EXIT_REFUSED);
    }

    if !execute {
        if json {
            let arr: Vec<String> = hits
                .iter()
                .map(|p| format!("{{\"pid\":{},\"name\":{},\"mem_mb\":{:.1},\"path\":{}}}", p.pid, jstr(&p.name), p.ws_mb.unwrap_or(0.0), jstr(&p.path)))
                .collect();
            println!("{{\"mode\":\"dry_run\",\"target\":{},\"count\":{},\"items\":[{}]}}", jstr(&target), hits.len(), arr.join(","));
        } else {
            println!("[dry-run] 将终止 {} 个进程（加 --execute 才执行）", hits.len());
            for p in &hits {
                println!("  pid {:<7} {:<24} {:>7.1} MB  {}", p.pid, p.name, p.ws_mb.unwrap_or(0.0), p.path);
            }
        }
        return;
    }

    // 和 trim 共用同一份编排：有可见窗口的先 WM_CLOSE，
    // 宽限期后才在身份校验下强杀。--force 用来要回旧行为。
    let force_flag = has_flag(args, "--force");
    let grace_sec = num_arg(args, "--grace-wait", 3.0).max(0.0);
    let opts = close::CloseOptions {
        graceful: !force_flag,
        grace_ms: (grace_sec * 1000.0) as u32,
        force: force_flag || !has_flag(args, "--no-force"),
    };
    let members: Vec<close::Target> = hits
        .iter()
        .map(|p| close::Target {
            pid: p.pid,
            created: p.created,
        })
        .collect();
    let rep = close::close_group(&target, &members, opts, &protected);
    let acted = rep.closed + rep.forced + rep.exited;

    if json {
        println!(
            "{{\"mode\":\"execute\",\"target\":{},\"ok\":{},\"failed\":{},\"requested\":{},\"closed\":{},\"forced\":{},\"exited\":{},\"skipped\":{},\"remaining\":{},\"message\":{}}}",
            jstr(&target),
            rep.remaining == 0 && acted > 0,
            rep.remaining,
            rep.requested,
            rep.closed,
            rep.forced,
            rep.exited,
            rep.skipped,
            rep.remaining,
            jstr(&rep.one_line(&target))
        );
    } else {
        println!("{}", rep.one_line(&target));
    }
    if rep.remaining > 0 {
        std::process::exit(EXIT_ACTION_FAILED);
    }
}

#[cfg(windows)]
fn cmd_svc(args: &[String]) {
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let pos = positionals(args, &["--out"]);
    if pos.len() < 2 {
        eprintln!("用法: MemBase.exe svc <stop|start|disable|enable> <服务名> [--execute] [--json]");
        eprintln!("  disable = start=disabled，下次开机不再起来（只 stop 的话重启就回来了）");
        eprintln!("  enable  = 恢复成手动启动（不会自动拉起）");
        std::process::exit(EXIT_USAGE);
    }
    let action = pos[0].to_lowercase();
    let name = pos[1].clone();
    if !matches!(action.as_str(), "stop" | "start" | "disable" | "enable") {
        eprintln!("动作只支持 stop / start / disable / enable");
        std::process::exit(EXIT_USAGE);
    }
    let mutating = action != "start" && action != "enable";
    if mutating && core::tier_for(Kind::Service, &name, "") == Tier::Blocked {
        eprintln!("[!] 拒绝：{} 属于关键系统服务", name);
        std::process::exit(EXIT_REFUSED);
    }
    if !execute {
        let verb = match action.as_str() {
            "stop" => "停止",
            "start" => "启动",
            "disable" => "禁止开机自启",
            _ => "恢复为手动启动",
        };
        if json {
            println!(
                "{{\"mode\":\"dry_run\",\"action\":{},\"service\":{}}}",
                jstr(&action),
                jstr(&name)
            );
        } else {
            println!("[dry-run] 将{}服务 {}（加 --execute 才执行）", verb, name);
        }
        return;
    }
    let r = match action.as_str() {
        "stop" => win::stop_service(&name),
        "start" => win::start_service(&name),
        "disable" => win::disable_service_start(&name),
        _ => win::enable_service_start(&name),
    };
    match r {
        Ok(()) => {
            let verb = match action.as_str() {
                "stop" => "已停止",
                "start" => "已启动",
                "disable" => "已禁止开机自启",
                _ => "已恢复为手动启动",
            };
            if json {
                println!("{{\"ok\":true,\"action\":{},\"service\":{}}}", jstr(&action), jstr(&name));
            } else {
                println!("{}：{}", verb, name);
            }
        }
        Err(e) => {
            if json {
                println!("{{\"ok\":false,\"action\":{},\"service\":{},\"error\":{}}}", jstr(&action), jstr(&name), jstr(&e));
            } else {
                eprintln!("失败：{}", e);
            }
            std::process::exit(EXIT_ACTION_FAILED);
        }
    }
}

/// 停掉启动项拉起的进程，再禁用启动项本身。
///
/// 两步都要做，少一步都白干：
///   * 只禁用启动项 → 现在这一份进程还在跑，要等下次开机才见效；
///   * 只停进程 → 下次开机它又被拉起来。
///
/// 禁用走 `StartupApproved` / `schtasks /change /disable`，不删原值，随时可撤销。
#[cfg(windows)]
fn stop_and_disable_startup(
    exe: &str,
    source: Option<core::startup::StartupSource>,
    name: &str,
) -> Result<(), String> {
    let Some(src) = source else {
        return Err("启动项来源缺失（清单构造有误）".into());
    };
    if !exe.is_empty() {
        let snap = snapshot::snapshot_now(true);
        let protected = win::protected_pids();
        let targets: Vec<close::Target> = snap
            .procs
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(exe))
            .map(|p| close::Target {
                pid: p.pid,
                created: p.created,
            })
            .collect();
        if !targets.is_empty() {
            let opts = close::CloseOptions {
                graceful: true,
                grace_ms: 1500,
                force: true,
            };
            // 停不掉不算失败：启动项禁用才是持久的那一步，进程可以等系统自己回收
            let _ = close::close_group(exe, &targets, opts, &protected);
        }
    }
    membase_win::startup::set_enabled(src, name, false)
}

/// `MemBase.exe slim`：以开机基准为输入，列出「开机就有、但根本不需要」的服务。
///
/// 和 `trim` 是反的：`trim` 关掉「基准里没有、现在却在跑」的线外项；
/// `slim` 动的是基准本身——它改的是**下一次开机的起点**，不是这一秒的占用。
#[cfg(windows)]
fn cmd_slim(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let include_caution = has_flag(args, "--include-caution");
    let min_mb = num_arg(args, "--min-mb", 5.0);

    // 先卡 --yes：不管有没有可砍项、是不是 --json，缺了它一律拒绝（免得以为执行了/没执行各说一套）
    if execute && !yes {
        eprintln!("[!] --execute 需要同时给 --yes（这一步会改服务配置 / 卸载应用）");
        std::process::exit(EXIT_REFUSED);
    }

    let base = match load_baseline(&root) {
        Ok(b) => b,
        Err(code) => std::process::exit(code),
    };
    let svc_text = fs::read_to_string(base.dir.join("services.csv")).unwrap_or_default();
    let rows = core::parse_service_rows(&svc_text);
    let mut items = core::build_trim_plan(&rows, min_mb, include_caution);

    let only = list_arg(args, "--only");
    let exclude = list_arg(args, "--exclude");
    if !only.is_empty() {
        items.retain(|it| only.iter().any(|n| core::wildcard_match(&it.name, n)));
    }
    if !exclude.is_empty() {
        items.retain(|it| !exclude.iter().any(|n| core::wildcard_match(&it.name, n)));
    }

    // 基准是历史快照：已经禁用/已卸载的项要跳过，否则每次跑都会重复建议同一批
    // （功能开关没有服务可查，跳过这一步，它另有「当前开关值」作为依据）
    let before = items.len();
    items.retain(|it| match it.action {
        core::TrimAction::FeatureOff | core::TrimAction::StopAndDisableStartup => true,
        _ => match win::service_start_mode(&it.name) {
            Some(m) => m != "Disabled",
            None => false,
        },
    });
    let already = before - items.len();

    // 开机自启的第三方程序：既不是服务、也不是 appx 的那一类。
    // 2026-10-22 实测的缺口：VS 安装器的 BackgroundDownload 一开机 339 MB，
    // 因为挂在 `\Microsoft\VisualStudio\Updates\BackgroundDownload` 下，
    // 旧版 startup 一刀切跳过 Microsoft 目录后，trim / slim / feature 三条路也都够不着。
    // 这里按「进程真的在跑 + 内存过阈值」把它捞回来，系统任务里的空挂项自然不出现。
    let nodes = proc_nodes();
    let startup_raw = membase_win::startup::scan();
    let startup_items =
        core::startup::build_startup_items(&startup_raw, &baseline_proc_names(&root));
    let mut mem_by_exe: BTreeMap<String, f64> = BTreeMap::new();
    for n in &nodes {
        if let Some(ws) = n.ws_mb {
            *mem_by_exe.entry(n.name.to_lowercase()).or_insert(0.0) += ws;
        }
    }
    let mut startup_trim =
        core::build_startup_trim_items(&startup_items, &mem_by_exe, min_mb, include_caution);
    if !only.is_empty() {
        startup_trim.retain(|it| only.iter().any(|n| core::wildcard_match(&it.host, n)));
    }
    if !exclude.is_empty() {
        startup_trim.retain(|it| !exclude.iter().any(|n| core::wildcard_match(&it.host, n)));
    }
    let startup_count = startup_trim.len();
    items.extend(startup_trim);

    // 功能开关：以「当前还开着 + 证据进程在跑」为准，折成同样的清单行。
    // 只写 HKCU，动作名 feature-off，撤销在 `MemBase.exe feature on <key>`。
    let states = feature_states();
    let evs = feature_evidence_map(&nodes);
    let mut feature_items = core::build_feature_items(core::feature::FEATURE_SWITCHES, &states, &evs);
    if !only.is_empty() {
        feature_items.retain(|it| only.iter().any(|n| core::wildcard_match(&it.name, n)));
    }
    if !exclude.is_empty() {
        feature_items.retain(|it| !exclude.iter().any(|n| core::wildcard_match(&it.name, n)));
    }
    let feature_count = feature_items.len();
    items.extend(feature_items);

    let savings = core::trim_savings(&items);

    if json {
        let arr: Vec<String> = items
            .iter()
            .map(|it| {
                format!(
                    "{{\"name\":{},\"display\":{},\"action\":{},\"package\":{},\"feature\":{},\"mem_mb\":{:.1},\"tier\":{},\"start_mode\":{},\"path\":{},\"reason\":{}}}",
                    jstr(&it.name),
                    jstr(&it.display),
                    jstr(it.action.key()),
                    it.appx.as_deref().map(jstr).unwrap_or_else(|| "null".into()),
                    it.feature.map(jstr).unwrap_or_else(|| "null".into()),
                    it.mem_mb,
                    jstr(core::tier_name(it.tier)),
                    jstr(&it.start_mode),
                    jstr(&it.path),
                    jstr(&it.reason)
                )
            })
            .collect();
        println!(
            "{{\"mode\":\"{}\",\"baseline\":{},\"count\":{},\"savings_mb\":{:.1},\"items\":[{}]}}",
            if execute { "execute" } else { "dry_run" },
            jstr(&base.captured),
            items.len(),
            savings,
            arr.join(",")
        );
        if !execute {
            return;
        }
    } else {
        println!("[..] 基准：{}（{} 采集）", base.dir.display(), base.captured);
        if items.is_empty() {
            println!("基准里没有值得砍的自启服务（阈值 {:.0} MB）", min_mb);
            return;
        }
        println!(
            "共 {} 项，预计开机少占 ≤{:.0} MB（同一宿主只算一次；共享宿主的项单独标注）\n",
            items.len(),
            savings
        );
        println!(
            "  {:<12} {:>9}  {:<30} {:<7} {}",
            "动作", "内存", "名称", "启动", "说明"
        );
        for it in &items {
            println!(
                "  {:<12} {:>7.1} MB  {:<30} {:<7} {}",
                it.action.label(),
                it.mem_mb,
                elide_cli(&it.name, 30),
                it.start_mode,
                elide_cli(&it.reason, 60)
            );
        }
        println!();
        if feature_count > 0 {
            println!(
                "（其中 {} 项是功能开关：只写 HKCU，撤销用 MemBase.exe feature on <key>）",
                feature_count
            );
        }
        if startup_count > 0 {
            println!(
                "（其中 {} 项是启动项：停进程 + 禁用启动项，撤销用 MemBase.exe startup enable <名称>）",
                startup_count
            );
        }
        if !include_caution {
            println!("（Windows 目录下的系统组件默认不列，加 --include-caution 查看）");
        }
        if already > 0 {
            println!("（另有 {} 项已经禁用或已卸载，跳过）", already);
        }
        if !execute {
            println!("加 --execute --yes 真正执行；卸载类可用 --exclude 跳过。");
            return;
        }
    }

    let mut ok = 0usize;
    let mut fail = 0usize;
    for it in &items {
        let r = match it.action {
            core::TrimAction::DisableService => win::disable_service_start(&it.name),
            core::TrimAction::UninstallAppx => {
                remove_appx(it.appx.as_deref().unwrap_or_default())
            }
            core::TrimAction::FeatureOff => match it.feature.and_then(core::feature::feature_by_key) {
                Some(sw) => apply_switch(sw, true).map(|_| ()),
                None => Err(format!("开关键不存在: {}", it.name)),
            },
            core::TrimAction::StopAndDisableStartup => {
                stop_and_disable_startup(&it.host, it.startup_source, &it.startup_name)
            }
        };
        match r {
            Ok(()) => {
                ok += 1;
                if !json {
                    println!("  ✓ {} {}", it.action.label(), it.name);
                }
            }
            Err(e) => {
                fail += 1;
                if !json {
                    println!("  ✗ {} {}：{}", it.action.label(), it.name, e);
                }
            }
        }
    }
    if !json {
        println!("\n完成：成功 {}，失败 {}。这些改动在**下次开机**才反映到基准上。", ok, fail);
    }
}

// ---------------- 启动项 ----------------

/// 基准里出现过的进程名（用来判断「这条启动项是不是真常驻」）
#[cfg(windows)]
fn baseline_proc_names(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(dir) = compare::find_latest_baseline(root) else {
        return out;
    };
    let rows = compare::read_csv(&dir.join("processes_by_name.csv"));
    if rows.is_empty() {
        return out;
    }
    if let Some(i) = rows[0]
        .iter()
        .position(|h| h.eq_ignore_ascii_case("ProcessName"))
    {
        for r in rows.iter().skip(1) {
            if let Some(n) = r.get(i) {
                if !n.is_empty() {
                    out.insert(n.clone());
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn cmd_startup(args: &[String]) {
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let pos = positionals(args, &["--out"]);
    let sub = pos
        .first()
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "list".into());

    let root = root_arg(args);
    let raw = membase_win::startup::scan();
    let baseline = baseline_proc_names(&root);
    let items = core::startup::build_startup_items(&raw, &baseline);

    if sub == "list" || sub == "ls" {
        // `\Microsoft\` 下的系统任务有 250 多个，默认不铺满屏幕；
        // 但 `--all` 要能看到它们 —— VS 安装器的 BackgroundDownload 就在里面。
        let all = has_flag(args, "--all");
        let system_count = items.iter().filter(|i| i.system).count();
        let shown: Vec<&core::startup::StartupItem> = items
            .iter()
            .filter(|i| all || !i.system)
            .collect();

        if json {
            let arr: Vec<String> = shown
                .iter()
                .map(|it| {
                    format!(
                        "{{\"name\":{},\"source\":{},\"source_label\":{},\"command\":{},\"exe\":{},\"enabled\":{},\"tier\":{},\"in_baseline\":{},\"system\":{},\"suggestion\":{},\"note\":{},\"detail\":{}}}",
                        jstr(&it.name),
                        jstr(it.source.key()),
                        jstr(it.source.label()),
                        jstr(&it.command),
                        jstr(&it.exe),
                        it.enabled,
                        jstr(core::tier_name(it.tier)),
                        it.in_baseline,
                        it.system,
                        jstr(it.suggestion),
                        jstr(&it.note),
                        jstr(&it.detail)
                    )
                })
                .collect();
            println!(
                "{{\"mode\":\"list\",\"count\":{},\"system_hidden\":{},\"items\":[{}]}}",
                shown.len(),
                if all { 0 } else { system_count },
                arr.join(",")
            );
            return;
        }
        if shown.is_empty() {
            println!("没扫到任何启动项（Run / 启动文件夹 / 计划任务都是空的）");
            return;
        }
        let on = shown.iter().filter(|i| i.enabled).count();
        println!(
            "共 {} 项启动项：启用 {} / 已禁用 {}（来源：Run、启动文件夹、计划任务）\n",
            shown.len(),
            on,
            shown.len() - on
        );
        println!(
            "  {:<6} {:<18} {:<26} {:<16} {:<8} {}",
            "状态", "来源", "名称", "目标进程", "分级", "说明"
        );
        for it in &shown {
            println!(
                "  {:<6} {:<18} {:<26} {:<16} {:<8} {}",
                if it.enabled { "启用" } else { "已禁用" },
                elide_cli(it.source.label(), 18),
                elide_cli(&it.name, 26),
                elide_cli(if it.exe.is_empty() { "—" } else { &it.exe }, 16),
                core::tier_name(it.tier),
                elide_cli(
                    &if it.note.is_empty() {
                        it.suggestion_cn().to_string()
                    } else {
                        format!("{}（{}）", it.suggestion_cn(), it.note)
                    },
                    52
                )
            );
        }
        println!();
        if system_count > 0 && !all {
            println!(
                "（另有 {} 个系统计划任务（\\Microsoft\\ 下）默认不列，加 --all 查看）",
                system_count
            );
        }
        println!("禁用一项：MemBase.exe startup disable <名称> --execute --yes");
        println!("撤销    ：MemBase.exe startup enable  <名称> --execute --yes");
        println!("禁用只改「启动批准记录 / 任务状态」，**不删原始命令行**，随时可撤销。");
        return;
    }

    let enable = match sub.as_str() {
        "disable" | "off" => false,
        "enable" | "on" => true,
        other => {
            eprintln!("未知子命令: startup {}（可用 list / disable / enable）", other);
            std::process::exit(EXIT_USAGE);
        }
    };
    let key = match pos.get(1) {
        Some(k) => k.trim().to_string(),
        None => {
            eprintln!("用法: MemBase.exe startup {} <名称> [--execute --yes] [--json]", if enable { "enable" } else { "disable" });
            std::process::exit(EXIT_USAGE);
        }
    };
    let hits: Vec<&core::startup::StartupItem> = items
        .iter()
        .filter(|i| {
            i.name.eq_ignore_ascii_case(&key)
                || i.exe.eq_ignore_ascii_case(&key)
                || i.name.trim_start_matches('\\').eq_ignore_ascii_case(key.trim_start_matches('\\'))
        })
        .collect();
    if hits.is_empty() {
        eprintln!("没找到启动项：{}（用 startup list 看准确名字）", key);
        std::process::exit(EXIT_NO_BASELINE);
    }
    if hits.len() > 1 {
        eprintln!("「{}」匹配到 {} 项，请用完整名字：", key, hits.len());
        for h in &hits {
            eprintln!("  {}  [{}]", h.name, h.source.label());
        }
        std::process::exit(EXIT_USAGE);
    }
    let it = hits[0];

    if !execute {
        if json {
            println!(
                "{{\"mode\":\"dry_run\",\"name\":{},\"source\":{},\"action\":{},\"enabled\":{}}}",
                jstr(&it.name),
                jstr(it.source.key()),
                jstr(if enable { "enable" } else { "disable" }),
                it.enabled
            );
        } else {
            println!(
                "[dry-run] 将{}：{}（{}）",
                if enable { "启用" } else { "禁用" },
                it.name,
                it.source.label()
            );
            println!("          {}", it.command);
            if enable {
                println!("          方式：写批准记录为启用 / schtasks /enable（原值不动）");
            } else {
                println!(
                    "          方式：{}",
                    if it.source == core::startup::StartupSource::ScheduledTask {
                        "schtasks /change /disable（任务保留，随时可恢复）"
                    } else {
                        "写 StartupApproved 批准记录（不删原命令行）"
                    }
                );
            }
            println!("加 --execute --yes 真正执行。");
        }
        return;
    }
    if !yes {
        eprintln!("[!] 改启动项要显式加 --yes");
        std::process::exit(EXIT_REFUSED);
    }
    match membase_win::startup::set_enabled(it.source, &it.name, enable) {
        Ok(()) => {
            if json {
                println!(
                    "{{\"mode\":\"execute\",\"ok\":true,\"name\":{},\"source\":{},\"enabled\":{}}}",
                    jstr(&it.name),
                    jstr(it.source.key()),
                    enable
                );
            } else {
                println!(
                    "已{}：{}（{}）· 下次登录生效",
                    if enable { "启用" } else { "禁用" },
                    it.name,
                    it.source.label()
                );
            }
        }
        Err(e) => {
            eprintln!("[x] {}", e);
            std::process::exit(EXIT_ACTION_FAILED);
        }
    }
}



// ---------------- 功能开关（只写 HKCU） ----------------

/// 当前进程树，喂给 core 做「证据进程归属」统计
#[cfg(windows)]
fn proc_nodes() -> Vec<core::feature::ProcNode> {
    snapshot::snapshot_now(true)
        .procs
        .iter()
        .map(|p| core::feature::ProcNode {
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.clone(),
            ws_mb: p.ws_mb,
        })
        .collect()
}

/// 读一条开关的全部注册表值，合并成一个状态
#[cfg(windows)]
fn switch_state(sw: &core::feature::FeatureSwitch) -> core::feature::SwitchState {
    let states: Vec<core::feature::SwitchState> = sw
        .values
        .iter()
        .map(|v| match win::user_reg_dword(v.key_path, v.value) {
            // 值不存在 = 从没设过，走系统默认，不能当「已关」
            None => core::feature::SwitchState::Missing,
            Some(x) if x == v.off => core::feature::SwitchState::Off,
            Some(_) => core::feature::SwitchState::On,
        })
        .collect();
    core::feature::combine_state(&states)
}

#[cfg(windows)]
fn feature_states() -> BTreeMap<&'static str, core::feature::SwitchState> {
    core::feature::FEATURE_SWITCHES
        .iter()
        .map(|sw| (sw.key, switch_state(sw)))
        .collect()
}

#[cfg(windows)]
fn feature_evidence_map(
    nodes: &[core::feature::ProcNode],
) -> BTreeMap<&'static str, core::feature::FeatureEvidence> {
    core::feature::FEATURE_SWITCHES
        .iter()
        .map(|sw| (sw.key, core::feature::feature_evidence(sw, nodes)))
        .collect()
}

/// 写入一条开关的目标状态。`off = true` 写关掉的值，否则写回/删除回到默认。
#[cfg(windows)]
fn apply_switch(sw: &core::feature::FeatureSwitch, off: bool) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    for v in sw.values {
        if off {
            win::set_user_reg_dword(v.key_path, v.value, v.off)?;
            done.push(format!(r"HKCU\{}\{} = {}", v.key_path, v.value, v.off));
        } else {
            match v.on {
                Some(on) => {
                    win::set_user_reg_dword(v.key_path, v.value, on)?;
                    done.push(format!(r"HKCU\{}\{} = {}", v.key_path, v.value, on));
                }
                None => {
                    win::delete_user_reg_value(v.key_path, v.value)?;
                    done.push(format!(r"HKCU\{}\{} 已删除（回系统默认）", v.key_path, v.value));
                }
            }
        }
    }
    Ok(done)
}

/// 找某个进程名当前的可执行路径（重启时按这条路径拉起）
#[cfg(windows)]
fn live_exe_path(name: &str) -> String {
    restart::processes_named(name)
        .into_iter()
        .map(|p| p.path)
        .find(|p| !p.is_empty())
        .unwrap_or_default()
}

#[cfg(windows)]
fn cmd_feature(args: &[String]) {
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let no_restart = has_flag(args, "--no-restart");
    let settle_ms = (num_arg(args, "--wait", 3.0).max(0.5) * 1000.0) as u32;

    let pos = positionals(args, &["--out", "--wait"]);
    let sub = pos.first().map(|s| s.to_lowercase()).unwrap_or_else(|| "list".into());

    // ---- list：开关值 + 证据 + 结论 ----
    if sub == "list" || sub == "ls" {
        let nodes = proc_nodes();
        let states = feature_states();
        let evs = feature_evidence_map(&nodes);
        if json {
            let arr: Vec<String> = core::feature::FEATURE_SWITCHES
                .iter()
                .map(|sw| {
                    let st = states.get(sw.key).copied().unwrap_or(core::feature::SwitchState::Missing);
                    let ev = evs.get(sw.key).copied().unwrap_or_default();
                    let vd = core::feature::feature_verdict(st, &ev);
                    format!(
                        "{{\"key\":{},\"label\":{},\"state\":{},\"state_label\":{},\"verdict\":{},\"verdict_label\":{},\"instances\":{},\"mem_mb\":{:.1},\"foreign\":{},\"host\":{},\"host_running\":{},\"note\":{},\"measured\":{}}}",
                        jstr(sw.key),
                        jstr(sw.label),
                        jstr(st.name()),
                        jstr(st.label()),
                        jstr(vd.name()),
                        jstr(vd.label()),
                        ev.instances,
                        ev.mem_mb,
                        ev.foreign,
                        jstr(sw.host_proc),
                        ev.host_running,
                        jstr(sw.note),
                        jstr(sw.measured)
                    )
                })
                .collect();
            println!(
                "{{\"mode\":\"list\",\"count\":{},\"items\":[{}]}}",
                core::feature::FEATURE_SWITCHES.len(),
                arr.join(",")
            );
            return;
        }
        println!("功能开关（只写 HKCU，不碰组策略；改完下次开机 / 重启宿主才见效）\n");
        for sw in core::feature::FEATURE_SWITCHES {
            let st = states.get(sw.key).copied().unwrap_or(core::feature::SwitchState::Missing);
            let ev = evs.get(sw.key).copied().unwrap_or_default();
            let vd = core::feature::feature_verdict(st, &ev);
            println!("[{}] {:<20} {}", vd.label(), sw.key, sw.label);
            println!(
                "        开关 {} · 证据 {} 个 {} / {:.1} MB（{}）{}",
                st.label(),
                ev.instances,
                sw.evidence_proc,
                ev.mem_mb,
                if sw.host_proc.is_empty() {
                    "这个进程自己就是宿主".to_string()
                } else if ev.host_running {
                    format!("宿主 {} 在跑", sw.host_proc)
                } else {
                    format!("宿主 {} 没起", sw.host_proc)
                },
                if ev.foreign > 0 {
                    format!("· 另有 {} 个同名进程归属别处，未计入", ev.foreign)
                } else {
                    String::new()
                }
            );
            println!("        {}", sw.note);
            if !sw.measured.is_empty() {
                println!("        实测：{}", sw.measured);
            }
            println!();
        }
        println!("关掉某项：MemBase.exe feature off <key> --execute --yes");
        println!("撤销    ：MemBase.exe feature on <key> --execute --yes");
        return;
    }

    // ---- off / on ----
    let off = match sub.as_str() {
        "off" | "disable" => true,
        "on" | "enable" | "undo" => false,
        other => {
            eprintln!("未知子命令: feature {}（可用 list / off / on）", other);
            std::process::exit(EXIT_USAGE);
        }
    };
    let key = match pos.get(1) {
        Some(k) => k.to_string(),
        None => {
            eprintln!(
                "用法: MemBase.exe feature {} <key> [--execute --yes] [--no-restart]\n可选 key: {}",
                if off { "off" } else { "on" },
                core::feature::feature_keys().join(", ")
            );
            std::process::exit(EXIT_USAGE);
        }
    };
    let sw = match core::feature::feature_by_key(&key) {
        Some(s) => s,
        None => {
            eprintln!(
                "未知开关: {}（可选 {}）",
                key,
                core::feature::feature_keys().join(", ")
            );
            std::process::exit(EXIT_USAGE);
        }
    };

    let nodes = proc_nodes();
    let before_state = switch_state(sw);
    let before_ev = core::feature::feature_evidence(sw, &nodes);
    let before_verdict = core::feature::feature_verdict(before_state, &before_ev);

    if !execute {
        if json {
            println!(
                "{{\"mode\":\"dry_run\",\"key\":{},\"action\":{},\"state\":{},\"instances\":{},\"mem_mb\":{:.1},\"would_restart_host\":{},\"host\":{}}}",
                jstr(sw.key),
                jstr(if off { "off" } else { "on" }),
                jstr(before_state.name()),
                before_ev.instances,
                before_ev.mem_mb,
                off && before_ev.instances > 0 && !no_restart && !sw.host_proc.is_empty(),
                jstr(sw.host_proc)
            );
        } else {
            println!(
                "[dry-run] {} {}（当前 {}，证据 {} 个 / {:.1} MB）",
                if off { "关掉" } else { "恢复" },
                sw.label,
                before_state.label(),
                before_ev.instances,
                before_ev.mem_mb
            );
            for v in sw.values {
                match (off, v.on) {
                    (true, _) => println!(
                        "          写 HKCU\\{}\\{} = {}",
                        v.key_path, v.value, v.off
                    ),
                    (false, Some(on)) => println!(
                        "          写 HKCU\\{}\\{} = {}",
                        v.key_path, v.value, on
                    ),
                    (false, None) => {
                        println!("          删 HKCU\\{}\\{}", v.key_path, v.value)
                    }
                }
            }
            if off && before_ev.instances > 0 && !no_restart {
                if sw.host_proc.is_empty() {
                    println!(
                        "          之后再关掉 {}（开关关掉后它本来就该退场）",
                        sw.evidence_proc
                    );
                } else {
                    println!(
                        "          之后再重启一次 {}，让改动落到当前这次开机上",
                        sw.host_proc
                    );
                }
            }
            println!("加 --execute --yes 真正执行。");
        }
        return;
    }

    if !yes {
        eprintln!("[!] 改注册表要显式加 --yes（这一步动的是当前用户的系统设置）");
        std::process::exit(EXIT_REFUSED);
    }

    let written = match apply_switch(sw, off) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[x] {}", e);
            std::process::exit(EXIT_ACTION_FAILED);
        }
    };

    // 关掉之后，实例往往还赖在内存里：重启 / 关掉宿主是最小代价的「让它现在生效」
    let mut host_report: Option<restart::RestartReport> = None;
    if off && !no_restart {
        let after_write_ev = core::feature::feature_evidence(sw, &proc_nodes());
        if after_write_ev.instances > 0 {
            let opts = close::CloseOptions::default();
            let protected = win::protected_pids();
            if sw.host_proc.is_empty() {
                // 证据进程自己就是宿主：开关关掉后它本来就该退场，只关不拉
                host_report = Some(restart::close_only(
                    sw.evidence_proc,
                    opts,
                    &protected,
                    settle_ms,
                ));
            } else {
                let path = live_exe_path(sw.host_proc);
                if !path.is_empty() {
                    host_report = Some(restart::restart_process(
                        sw.host_proc,
                        &path,
                        opts,
                        &protected,
                        settle_ms,
                    ));
                }
            }
        }
    }

    let nodes = proc_nodes();
    let after_state = switch_state(sw);
    let after_ev = core::feature::feature_evidence(sw, &nodes);
    let after_verdict = core::feature::feature_verdict(after_state, &after_ev);

    if json {
        println!(
            "{{\"mode\":\"execute\",\"key\":{},\"action\":{},\"written\":[{}],\"before\":{{\"state\":{},\"instances\":{},\"mem_mb\":{:.1},\"verdict\":{}}},\"after\":{{\"state\":{},\"instances\":{},\"mem_mb\":{:.1},\"verdict\":{}}},\"host_restart\":{}}}",
            jstr(sw.key),
            jstr(if off { "off" } else { "on" }),
            written.iter().map(|w| jstr(w)).collect::<Vec<_>>().join(","),
            jstr(before_state.name()),
            before_ev.instances,
            before_ev.mem_mb,
            jstr(before_verdict.name()),
            jstr(after_state.name()),
            after_ev.instances,
            after_ev.mem_mb,
            jstr(after_verdict.name()),
            match &host_report {
                Some(r) => format!(
                    "{{\"name\":{},\"revived\":{},\"reclaimed_mb\":{:.1},\"message\":{}}}",
                    jstr(&r.name),
                    r.revived,
                    r.reclaimed_mb,
                    jstr(&r.message)
                ),
                None => "null".to_string(),
            }
        );
        return;
    }

    println!("已{}：{}", if off { "关掉" } else { "恢复" }, sw.label);
    for w in &written {
        println!("  {}", w);
    }
    if let Some(r) = &host_report {
        println!("  宿主 {}：{}", r.name, r.message);
    }
    println!(
        "  证据：{} 个 / {:.1} MB → {} 个 / {:.1} MB（{}）",
        before_ev.instances,
        before_ev.mem_mb,
        after_ev.instances,
        after_ev.mem_mb,
        after_verdict.label()
    );
    if after_verdict == core::feature::FeatureVerdict::OffButAlive {
        println!(
            "  注意：开关已经是关的，实例仍在跑。这台机器上重启 {} 也回收不到（关掉就自愈），",
            if sw.host_proc.is_empty() { sw.evidence_proc } else { sw.host_proc }
        );
        println!("        真实成因不在这个开关上，别把它算进收益。");
    }
    if !sw.measured.is_empty() {
        println!("  实测记录：{}", sw.measured);
    }
}

// ---------------- 局部重启 ----------------

#[cfg(windows)]
fn cmd_restart(args: &[String]) {
    let json = want_json(args);
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let settle_ms = (num_arg(args, "--wait", 3.0).max(0.3) * 1000.0) as u32;
    let pos = positionals(args, &["--out", "--wait", "--path"]);
    let target = match pos.first() {
        Some(t) => t.to_string(),
        None => {
            eprintln!("用法: MemBase.exe restart <进程名|PID|服务名> [--execute --yes] [--wait N] [--json]");
            std::process::exit(EXIT_USAGE);
        }
    };

    let snap = snapshot::snapshot_now(true);
    let by_pid: Option<u32> = target.parse().ok();
    let hits: Vec<&snapshot::ProcRow> = snap
        .procs
        .iter()
        .filter(|p| match by_pid {
            Some(pid) => p.pid == pid,
            None => p.name.eq_ignore_ascii_case(&target),
        })
        .collect();

    // ---- 服务：不在进程表里、但服务管理器认识 ----
    if hits.is_empty() {
        if win::service_start_mode(&target).is_none() {
            eprintln!("没找到进程或服务：{}", target);
            std::process::exit(EXIT_NO_BASELINE);
        }
        let tier = core::tier_for(Kind::Service, &target, "");
        let check = core::feature::restart_check(core::feature::RestartKind::Service, &target, "", tier);
        if !check.allowed {
            eprintln!("[!] 拒绝：{}（{}）", target, check.reason);
            std::process::exit(EXIT_REFUSED);
        }
        if !execute {
            let pid = win::service_pid(&target);
            let mb = pid.and_then(win::process_ws_mb).unwrap_or(0.0);
            if json {
                println!(
                    "{{\"mode\":\"dry_run\",\"kind\":\"service\",\"target\":{},\"running\":{},\"mem_mb\":{:.1}}}",
                    jstr(&target),
                    pid.is_some(),
                    mb
                );
            } else {
                println!("[dry-run] 将重启服务 {}（当前 {} / {:.1} MB）", target, if pid.is_some() { "运行中" } else { "已停止" }, mb);
                println!("          stop → start，执行后按实测报告宿主工作集变化");
            }
            return;
        }
        if !yes {
            eprintln!("[!] 非交互执行必须加 --yes");
            std::process::exit(EXIT_REFUSED);
        }
        let rep = restart::restart_service(&target, settle_ms);
        print_restart(&rep, json);
        if !rep.ok {
            std::process::exit(EXIT_ACTION_FAILED);
        }
        return;
    }

    // ---- 进程 ----
    let name = hits[0].name.clone();
    let tier = core::tier_for(Kind::Process, &name, &hits[0].path);
    let exe = match arg_value(args, "--path") {
        Some(p) => p,
        None => hits
            .iter()
            .map(|p| p.path.clone())
            .find(|p| !p.is_empty())
            .unwrap_or_default(),
    };
    let check = core::feature::restart_check(core::feature::RestartKind::Process, &name, &exe, tier);
    if !check.allowed {
        eprintln!("[!] 拒绝重启 {}：{}", name, check.reason);
        std::process::exit(EXIT_REFUSED);
    }

    let before_mb: f64 = hits.iter().filter_map(|p| p.ws_mb).sum();
    let unknown = hits.iter().filter(|p| p.ws_mb.is_none()).count();

    if !execute {
        if json {
            println!(
                "{{\"mode\":\"dry_run\",\"kind\":\"process\",\"target\":{},\"instances\":{},\"mem_mb\":{:.1},\"unknown\":{},\"path\":{},\"needs_yes\":{}}}",
                jstr(&name),
                hits.len(),
                before_mb,
                unknown,
                jstr(&exe),
                core::feature::restart_needs_yes(&name)
            );
        } else {
            println!(
                "[dry-run] 将重启 {}：{} 个实例 / {:.1} MB{}",
                name,
                hits.len(),
                before_mb,
                if unknown > 0 {
                    format!("（{} 个读不到内存）", unknown)
                } else {
                    String::new()
                }
            );
            println!("          可执行路径 {}", exe);
            for p in &hits {
                println!(
                    "          pid {:<7} {:>8.1} MB  {}",
                    p.pid,
                    p.ws_mb.unwrap_or(0.0),
                    p.path
                );
            }
            if hits.len() > 1 {
                println!("          注意：按名字重启会把同名实例全部关掉再拉起");
            }
            println!("          执行后等 {} 毫秒再实测回收量（不报理论值）", settle_ms);
        }
        return;
    }

    // explorer / SearchHost 会闪任务栏、关掉已打开的窗口：必须显式 --yes
    if core::feature::restart_needs_yes(&name) && !yes {
        eprintln!(
            "[!] 重启 {} 会关掉它现在的窗口（任务栏会闪一下），必须显式加 --yes",
            name
        );
        std::process::exit(EXIT_REFUSED);
    }
    if !yes && !std::io::stdin().is_terminal() {
        eprintln!("[!] 非交互环境必须显式加 --yes 才能执行");
        std::process::exit(EXIT_REFUSED);
    }
    if !yes {
        println!(
            "即将重启 {}（{} 个实例 / {:.1} MB）。输入 yes 继续：",
            name,
            hits.len(),
            before_mb
        );
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if line.trim() != "yes" {
            println!("已取消。");
            return;
        }
    }

    let force_flag = has_flag(args, "--force");
    let grace_sec = num_arg(args, "--grace-wait", 3.0).max(0.0);
    let opts = close::CloseOptions {
        graceful: !force_flag,
        grace_ms: (grace_sec * 1000.0) as u32,
        force: force_flag || !has_flag(args, "--no-force"),
    };
    let rep = restart::restart_process(&name, &exe, opts, &win::protected_pids(), settle_ms);
    // 自愈结论落盘：下次 `status` / `list` 直接标出来，不用再杀一遍才知道
    if rep.revived {
        note_selfheal(&root_arg(args), &name);
    }
    print_restart(&rep, json);
    if !rep.ok {
        std::process::exit(EXIT_ACTION_FAILED);
    }
}

#[cfg(windows)]
fn print_restart(rep: &restart::RestartReport, json: bool) {
    if json {
        println!(
            "{{\"mode\":\"execute\",\"kind\":{},\"target\":{},\"ok\":{},\"before\":{{\"instances\":{},\"mem_mb\":{:.1},\"unknown\":{}}},\"after\":{{\"instances\":{},\"mem_mb\":{:.1},\"unknown\":{}}},\"reclaimed_mb\":{:.1},\"closed\":{},\"forced\":{},\"relaunched\":{},\"revived\":{},\"message\":{},\"errors\":[{}]}}",
            jstr(rep.kind),
            jstr(&rep.name),
            rep.ok,
            rep.before_instances,
            rep.before_mb,
            rep.before_unknown,
            rep.after_instances,
            rep.after_mb,
            rep.after_unknown,
            rep.reclaimed_mb,
            rep.closed,
            rep.forced,
            rep.relaunched,
            rep.revived,
            jstr(&rep.message),
            rep.errors.iter().map(|e| jstr(e)).collect::<Vec<_>>().join(",")
        );
        return;
    }
    println!("{}", rep.message);
    if rep.reclaimed_mb < 0.0 {
        println!("  说明：新实例冷启动会更胖，回收量按实测如实报（负值就是负值）");
    }
    for e in &rep.errors {
        println!("  ! {}", e);
    }
}


/// 卸载 Store 应用：实现在 win crate（界面与命令行共用同一份，别各写一套）
#[cfg(windows)]
fn remove_appx(full_name: &str) -> Result<(), String> {
    win::remove_appx(full_name)
}

#[cfg(windows)]
fn elide_cli(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max.saturating_sub(1)].iter().collect::<String>())
    }
}

#[cfg(windows)]
fn cmd_plan(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let include_caution = has_flag(args, "--include-caution");
    let (base, plan) = match build_current_plan(&root, num_arg(args, "--min-mb", 10.0)) {
        Ok(x) => x,
        Err(code) => std::process::exit(code),
    };
    let heal = read_selfheal(&root);
    let plan = filter_plan(
        plan,
        &list_arg(args, "--only"),
        &list_arg(args, "--exclude"),
    );
    let (sg, ca, bl, act, act_mb) = plan_counts(&plan, include_caution);

    if let Some(path) = arg_value(args, "--to") {
        let mut csv = String::from("Kind,Name,Pids,Instances,MemMB,Tier,Protect,Actionable,Reason\n");
        for p in &plan {
            csv.push_str(&format!(
                "{},{},{},{},{:.1},{},{},{},{}\n",
                core::kind_name(p.kind),
                p.name,
                p.pids().iter().map(|x| x.to_string()).collect::<Vec<_>>().join("|"),
                p.instances,
                p.mem_mb,
                core::tier_name(p.tier),
                p.protect,
                p.actionable(include_caution),
                p.reason.replace(',', ";")
            ));
        }
        if let Err(e) = fs::write(&path, csv) {
            eprintln!("[x] 写计划失败: {}", e);
            std::process::exit(EXIT_IO);
        }
        if !json {
            println!("计划已写入 {}", path);
        }
    }

    if json {
        let items: Vec<String> = plan.iter().map(|p| item_json(p, include_caution, &heal)).collect();
        println!(
            "{{\"mode\":\"plan\",\"baseline\":{},\"counts\":{{\"total\":{},\"suggested\":{},\"caution\":{},\"blocked\":{},\"actionable\":{},\"actionable_mb\":{:.1}}},\"items\":[{}]}}",
            jstr(&base.dir.to_string_lossy()),
            plan.len(),
            sg,
            ca,
            bl,
            act,
            act_mb,
            items.join(",")
        );
    } else if arg_value(args, "--to").is_none() {
        println!(
            "线外 {} 项，可处理 {} 项（预计回收约 {:.0} MB）",
            plan.len(),
            act,
            act_mb
        );
        for p in &plan {
            // 共享宿主的服务：显示的 MB 是宿主全量，但停掉这一项不会让宿主退出
            let mark = if p.shared_host {
                "  ← 共享宿主，停这一项本身不释放内存"
            } else {
                ""
            };
            println!(
                "  {} {:<9} {:<28} {:>7.0} MB{}",
                core::kind_name(p.kind),
                tier_label(p.tier),
                p.name,
                p.mem_mb,
                mark
            );
        }
        if act > 0 {
            println!();
            println!("（回收量按宿主去重：同一 PID 只算一次，共享宿主的服务项计 0）");
        }
    }
}

#[cfg(windows)]
fn cmd_protect(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");

    let normalize = |raw: &str| -> String {
        if let Some(i) = raw.find(':') {
            let (head, tail) = (&raw[..i], &raw[i + 1..]);
            let kind = match head {
                "进程" | "process" | "proc" | "p" => "进程",
                "服务" | "service" | "svc" | "s" => "服务",
                _ => "进程",
            };
            format!("{}:{}", kind, tail.trim())
        } else {
            format!("进程:{}", raw.trim())
        }
    };

    let mut items = read_whitelist(&root);

    // add / remove 的名字解析：跳过 --flag 及其取值，支持逗号分隔
    let mut names: Vec<String> = Vec::new();
    let mut skip_next = false;
    for a in args.iter().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a.starts_with("--") {
            if matches!(
                a.as_str(),
                "--out" | "--task" | "--min-mb" | "--max" | "--only" | "--exclude"
            ) {
                skip_next = true;
            }
            continue;
        }
        for part in a.split([',', ';']) {
            let t = part.trim();
            if !t.is_empty() {
                names.push(normalize(t));
            }
        }
    }

    match sub {
        "add" => {
            if names.is_empty() {
                eprintln!("用法: MemBase.exe protect add <名字[,名字...]>");
                std::process::exit(EXIT_USAGE);
            }
            for n in names {
                if !items.contains(&n) {
                    items.push(n);
                }
            }
            if let Err(e) = write_whitelist(&root, &items) {
                eprintln!("[x] 写保护名单失败: {}", e);
                std::process::exit(EXIT_IO);
            }
        }
        "remove" | "rm" | "del" => {
            items.retain(|x| !names.contains(x));
            if let Err(e) = write_whitelist(&root, &items) {
                eprintln!("[x] 写保护名单失败: {}", e);
                std::process::exit(EXIT_IO);
            }
        }
        "clear" => {
            items.clear();
            if let Err(e) = write_whitelist(&root, &items) {
                eprintln!("[x] 写保护名单失败: {}", e);
                std::process::exit(EXIT_IO);
            }
        }
        "list" | "ls" => {}
        other => {
            eprintln!("未知子命令: protect {}", other);
            std::process::exit(EXIT_USAGE);
        }
    }

    if json {
        let arr: Vec<String> = items.iter().map(|s| jstr(s)).collect();
        println!(
            "{{\"file\":{},\"count\":{},\"protect\":[{}]}}",
            jstr(&protect_file(&root).to_string_lossy()),
            items.len(),
            arr.join(",")
        );
    } else if items.is_empty() {
        println!("保护名单为空（{}）", protect_file(&root).display());
    } else {
        println!(
            "保护名单（{} 项，{}）：",
            items.len(),
            protect_file(&root).display()
        );
        for i in &items {
            println!("  {}", i);
        }
    }
}

#[cfg(windows)]
/// 一次采集的摘要（只用来打印，不进快照文件）
#[cfg(windows)]
struct SnapStats {
    procs: usize,
    svcs: usize,
    used_mb: u64,
    load_pct: u32,
    uptime_s: u64,
}

/// 自愈型进程的记录文件（数据目录里的第 5 类内容，只读信息）
#[cfg(windows)]
fn selfheal_file(root: &Path) -> PathBuf {
    root.join("selfheal.txt")
}

#[cfg(windows)]
fn read_selfheal(root: &Path) -> BTreeMap<String, String> {
    core::selfheal::parse(&fs::read_to_string(selfheal_file(root)).unwrap_or_default())
}

/// 记一条「杀掉又回来了」的结论。只有真的跑过 `restart` 才会写，
/// 不做自动探测 —— 探测本身就要杀进程，那正是要避免的打地鼠。
#[cfg(windows)]
pub fn note_selfheal(root: &Path, name: &str) {
    let p = selfheal_file(root);
    let text = fs::read_to_string(&p).unwrap_or_default();
    let next = core::selfheal::note(
        &text,
        name,
        &win::now_local_string(),
        core::selfheal::MAX_ENTRIES,
    );
    if let Err(e) = fs::write(&p, next) {
        eprintln!("[!] 自愈记录写入失败（不影响本次结果）: {}", e);
    }
}

/// 轮换旧基准目录，返回被移进回收站的名字。`keep == 0` = 不轮换。
///
/// 数据目录里只有 `run_*` 会无限增长（每次开机一个）。早先只能手动清，
/// 2026-10-22 实测已经堆到 11 个 —— 改成自动保留最近 `keep` 个，老的走回收站。
#[cfg(windows)]
fn rotate_runs(root: &Path, keep: usize) -> Vec<String> {
    if keep == 0 {
        return Vec::new();
    }
    let dir = root;
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    let mut done = Vec::new();
    for name in core::plan_rotation(&names, keep) {
        if win::recycle_path(&dir.join(&name)).is_ok() {
            done.push(name);
        }
    }
    done
}

fn cmd_collect(args: &[String]) {
    let out = root_arg(args);
    let at = num_arg(args, "--at", 60.0) as u64;
    let also = num_arg_opt(args, "--also")
        .map(|v| v as u64)
        .filter(|v| *v > at);
    let include_self = has_flag(args, "--include-self");
    let self_destruct = has_flag(args, "--self-destruct");
    let json = want_json(args);
    let task = task_arg(args);

    snapshot::wait_until_boot_offset(at);

    let snap = snapshot::snapshot_now(include_self);
    let run_dir = snapshot::new_run_dir(&out);
    let sub = run_dir.join(format!("T+{}s", at));
    let write_started = std::time::Instant::now();
    if let Err(e) = snapshot::write_snapshot(&sub, &snap) {
        eprintln!("[x] 写盘失败: {}（保留计划任务，下次开机重试）", e);
        std::process::exit(EXIT_IO);
    }
    let write_ms = write_started.elapsed().as_millis() as u64;

    // 第二点：同一次开机、同一个 run 目录下再采一次。
    // 只在前一次出现、这一次消失的进程就是「短命启动项」，别让它污染基准。
    let mut second: Option<(PathBuf, SnapStats)> = None;
    if let Some(sec) = also {
        snapshot::wait_until_boot_offset(sec);
        let snap2 = snapshot::snapshot_now(include_self);
        let sub2 = run_dir.join(format!("T+{}s", sec));
        if let Err(e) = snapshot::write_snapshot(&sub2, &snap2) {
            eprintln!("[!] 第二次采集写盘失败: {}（第一点已保留）", e);
        } else {
            second = Some((
                sub2,
                SnapStats {
                    procs: snap2.procs.len(),
                    svcs: snap2.svcs.len(),
                    used_mb: snap2.used_phys_mb,
                    load_pct: snap2.load_pct,
                    uptime_s: snap2.uptime_s,
                },
            ));
        }
    }

    // 基准目录轮换：老的 run_* 移进回收站，只留最近 keep 份（`--keep 0` 关掉）
    let keep = num_arg(args, "--keep", 10.0) as usize;
    let rotated = rotate_runs(&out, keep);

    if json {
        let second_json = match &second {
            Some((p, s)) => format!(
                "{{\"dir\":{},\"uptime_sec\":{},\"process_count\":{},\"service_count\":{},\"used_mb\":{},\"load_pct\":{}}}",
                jstr(&p.to_string_lossy()),
                s.uptime_s,
                s.procs,
                s.svcs,
                s.used_mb,
                s.load_pct
            ),
            None => "null".to_string(),
        };
        println!(
            "{{\"ok\":true,\"dir\":{},\"uptime_sec\":{},\"process_count\":{},\"service_count\":{},\"used_mb\":{},\"load_pct\":{},\"collect_ms\":{},\"write_ms\":{},\"also\":{}}}",
            jstr(&sub.to_string_lossy()),
            snap.uptime_s,
            snap.procs.len(),
            snap.svcs.len(),
            snap.used_phys_mb,
            snap.load_pct,
            snap.collect_ms,
            write_ms,
            second_json
        );
    } else {
        println!(
            "[OK] 采集完成：{} 个进程 / {} 个服务，开机第 {} 秒",
            snap.procs.len(),
            snap.svcs.len(),
            snap.uptime_s
        );
        println!("     输出：{}", sub.display());
        println!(
            "     已用内存 {} MB / 占用率 {} %",
            snap.used_phys_mb, snap.load_pct
        );
        println!(
            "     采集耗时 {} 毫秒（落盘另计 {} 毫秒）",
            snap.collect_ms, write_ms
        );
        if let Some((p, s)) = &second {
            println!(
                "[OK] 第二次采集：{} 个进程 / {} 个服务，开机第 {} 秒，已用 {} MB",
                s.procs,
                s.svcs,
                s.uptime_s,
                s.used_mb
            );
            println!("     输出：{}", p.display());
        }
        if !rotated.is_empty() {
            println!(
                "     轮换：{} 个旧基准已移入回收站（保留最近 {} 份，--keep 可调）",
                rotated.len(),
                keep
            );
        }
    }

    if self_destruct {
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", &task, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        if let Ok(exe) = std::env::current_exe() {
            let p = exe.to_string_lossy().to_string();
            // 自己删不掉自己：等两秒再让 cmd 动手。
            // 这里必须用 raw_arg：`.arg(line)` 会把内层引号转义成 \"，
            // 而 cmd 不认这种转义，del 收到的是坏路径（报“文件名、目录名或卷标语法不正确”）。
            // /S 让 cmd 只剥掉最外层引号，路径两侧的引号原样留给 del。
            let line = format!("/S /C \"ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"\"", p);
            let _ = Command::new("cmd")
                .raw_arg(line)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    }
}

#[cfg(windows)]
fn cmd_compare(args: &[String]) {
    let root = root_arg(args);
    let min_mb = num_arg(args, "--min-mb", 10.0);
    let excludes = list_arg(args, "--exclude");
    let label = arg_value(args, "--label").filter(|l| !l.trim().is_empty());

    if !want_json(args) {
        if let Err(e) = compare::compare(&root, min_mb, &excludes, true, label.as_deref()) {
            eprintln!("[x] 对比失败: {}", e);
            std::process::exit(EXIT_IO);
        }
        return;
    }

    let base = match load_baseline_label(&root, label.as_deref()) {
        Ok(b) => b,
        Err(code) => std::process::exit(code),
    };
    let live = live_now();
    let mut agg: BTreeMap<String, core::ProcAgg> = BTreeMap::new();
    for p in &live.procs {
        let e = agg.entry(p.name.clone()).or_default();
        e.instances += 1;
        e.ws_mb += p.ws_mb;
        e.priv_mb += p.priv_mb;
        e.unknown += p.unknown;
    }
    let diffs = core::diff_procs(&base.procs, &agg);
    let mut items: Vec<String> = Vec::new();
    let mut total_delta = 0.0f64;
    for d in &diffs {
        if excludes.iter().any(|pat| core::wildcard_match(&d.name, pat)) {
            continue;
        }
        if d.delta.abs() < min_mb
            && !matches!(d.kind, core::DeltaKind::New | core::DeltaKind::Gone)
        {
            continue;
        }
        total_delta += d.delta;
        items.push(format!(
            "{{\"name\":{},\"status\":{},\"base_mb\":{:.1},\"live_mb\":{:.1},\"delta_mb\":{:.1},\"base_instances\":{},\"live_instances\":{}}}",
            jstr(&d.name),
            jstr(match d.kind {
                core::DeltaKind::New => "new",
                core::DeltaKind::Gone => "gone",
                core::DeltaKind::Up => "up",
                core::DeltaKind::Down => "down",
                core::DeltaKind::Same => "same",
                core::DeltaKind::Unknown => "unknown",
            }),
            d.base_ws,
            d.live_ws,
            d.delta,
            d.base_inst,
            d.live_inst
        ));
    }
    println!(
        "{{\"baseline\":{{\"dir\":{},\"captured_at\":{}}},\"live\":{{\"used_mb\":{},\"process_count\":{}}},\"delta_mb\":{:.1},\"items\":[{}]}}",
        jstr(&base.dir.to_string_lossy()),
        jstr(&base.captured),
        live.used_mb,
        live.proc_count,
        total_delta,
        items.join(",")
    );
}


/// `MemBase.exe ui`：把界面拉起来。页面切换本身也有命令入口，
/// 这样「界面上的每个操作」连「打开界面并停在布局页」都能被脚本表达。
#[cfg(windows)]
fn cmd_ui(args: &[String]) {
    let page = arg_value(args, "--page").unwrap_or_default();
    let cur = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[x] 取不到自身路径: {}", e);
            std::process::exit(EXIT_IO);
        }
    };
    let is_gui = cur
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("ui"))
        .unwrap_or(false);
    let gui = if is_gui {
        cur.clone()
    } else {
        cur.with_file_name("MemBase.exe")
    };
    if !gui.is_file() {
        eprintln!("[x] 找不到界面程序：{}", gui.display());
        std::process::exit(EXIT_IO);
    }
    let mut cmd = Command::new(&gui);
    if !page.trim().is_empty() {
        cmd.env("MEMBASE_PAGE", page.trim());
    }
    match cmd.spawn() {
        Ok(_) => {
            if want_json(args) {
                println!(
                    "{{\"ok\":true,\"exe\":{},\"page\":{}}}",
                    jstr(&gui.to_string_lossy()),
                    jstr(page.trim())
                );
            } else {
                println!("已打开界面：{}", gui.display());
            }
        }
        Err(e) => {
            eprintln!("[x] 启动失败: {}", e);
            std::process::exit(EXIT_IO);
        }
    }
}

/// 注册开机采集任务。XML、拷 exe、调 schtasks 都在 membase-win::task 里，
/// GUI 的按钮走的是同一份实现。
#[cfg(windows)]
fn cmd_install(args: &[String]) {
    let out = root_arg(args);
    let at = num_arg(args, "--at", 60.0) as u64;
    let also = num_arg_opt(args, "--also")
        .map(|v| v as u64)
        .filter(|v| *v > at);
    let task = task_arg(args);
    let json = want_json(args);

    let cur = std::env::current_exe().expect("取不到自身路径");
    let res = membase_win::task::install(&cur, &out, at, also, &task);
    let ok = res.is_ok();
    let target = res
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    if json {
        println!(
            "{{\"ok\":{},\"task\":{},\"exe\":{},\"at_sec\":{},\"also_sec\":{},\"out\":{}}}",
            ok,
            jstr(&task),
            jstr(&target),
            at,
            also.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
            jstr(&out.to_string_lossy())
        );
    } else if ok {
        println!("[OK] 计划任务已注册：{}（开机触发，SYSTEM，无窗口）", task);
        match also {
            Some(s) => println!(
                "     下次开机在第 {} 秒与第 {} 秒各采一次（双采样），采完任务和 exe 自动消失",
                at, s
            ),
            None => println!("     下次开机会自动采集一次，随后任务和 exe 自动消失"),
        }
    } else {
        eprintln!("[x] {}", res.unwrap_err());
    }
    if !ok {
        std::process::exit(EXIT_REFUSED);
    }
}

#[cfg(windows)]
fn cmd_uninstall(args: &[String]) {
    let task = task_arg(args);
    let json = want_json(args);
    let res = membase_win::task::uninstall(&task);
    let ok = res.is_ok();
    if json {
        println!("{{\"ok\":{},\"task\":{}}}", ok, jstr(&task));
    } else if let Err(e) = &res {
        eprintln!("[x] 删除失败：{}", e);
    } else {
        println!("[OK] 已删除计划任务：{}", task);
    }
}

// ---------------- 入口 ----------------

#[cfg(windows)]
fn dispatch(cmd: &str, rest: &[String]) {
    match cmd {
        "status" => cmd_status(rest),
        "list" | "ls" => cmd_list(rest),
        "plan" => cmd_plan(rest),
        "trim" | "restore" => cmd_trim(rest),
        "slim" => cmd_slim(rest),
        "restart" => cmd_restart(rest),
        "feature" | "switch" => cmd_feature(rest),
        "startup" | "start" => cmd_startup(rest),
        "transient" | "short-lived" => cmd_transient(rest),
        "kill" => cmd_kill(rest),
        "svc" => cmd_svc(rest),
        "commands" => cmd_commands(rest),
        "protect" => cmd_protect(rest),
        "collect" => cmd_collect(rest),
        "compare" | "diff" => cmd_compare(rest),
        "install" => cmd_install(rest),
        "uninstall" => cmd_uninstall(rest),
        other => {
            eprintln!("未知命令: {}\n", other);
            usage();
            std::process::exit(EXIT_USAGE);
        }
    }
}

pub fn run() {
    #[cfg(windows)]
    if std::io::stdout().is_terminal() {
        win::console_utf8();
    }
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        usage();
        return;
    }
    let cmd = argv[1].as_str();
    match cmd {
        "help" | "-h" | "--help" => {
            usage();
            return;
        }
        "version" | "-V" | "--version" => {
            println!("MemBase {}", membase_core::VERSION);
            return;
        }
        "baseline" => {
            // 只操作文件，跨平台可用
            cmd_baseline(&argv[2..]);
            return;
        }
        "commands" => {
            cmd_commands(&argv[2..]);
            return;
        }
        "mcp" => {
            // MCP 服务端：走 stdio 的 JSON-RPC，把同一套动词暴露给 AI
            #[cfg(windows)]
            {
                std::process::exit(mcp::serve());
            }
            #[cfg(not(windows))]
            {
                eprintln!("MemBase.exe mcp 只支持 Windows（工具本身依赖 Win32）");
                std::process::exit(EXIT_USAGE);
            }
        }
        "config" => {
            cmd_config(&argv[2..]);
            return;
        }
        "theme" => {
            cmd_theme(&argv[2..]);
            return;
        }
        "screen" | "display" => {
            #[cfg(windows)]
            {
                std::process::exit(screen::cmd_screen(&argv[2..]));
            }
            #[cfg(not(windows))]
            {
                eprintln!("MemBase.exe screen 只支持 Windows");
                std::process::exit(EXIT_USAGE);
            }
        }
        "ui" | "gui" => {
            #[cfg(windows)]
            {
                cmd_ui(&argv[2..]);
            }
            #[cfg(not(windows))]
            eprintln!("MemBase.exe ui 只支持 Windows");
            return;
        }
        "layout" | "layouts" | "desk" => {
            #[cfg(windows)]
            {
                std::process::exit(layout::cmd_layout(&argv[2..]));
            }
            #[cfg(not(windows))]
            {
                eprintln!("MemBase.exe layout 只支持 Windows（依赖 explorer 的桌面 ListView）");
                std::process::exit(EXIT_USAGE);
            }
        }
        _ => {}
    }
    #[cfg(windows)]
    dispatch(cmd, &argv[2..]);
    #[cfg(not(windows))]
    {
        let _ = cmd;
        eprintln!("MemBase.exe 的采集/清理与桌面布局只支持 Windows；纯逻辑部分用 `cargo test -p membase-core` 验证。");
        std::process::exit(EXIT_USAGE);
    }
}
