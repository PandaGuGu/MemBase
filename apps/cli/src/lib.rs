#![cfg_attr(not(windows), allow(dead_code, unused_imports))]
//! deskkit —— 内存基准线 + 桌面图标布局（命令行逻辑）
//!
//! 同一份逻辑被两个入口复用：独立的 `deskkit.exe`（控制台），
//! 以及带参数启动的界面版 `MemBase.exe`（自己接回父控制台）。
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
//!
//! 退出码：
//!   0 成功 / 1 参数错误 / 2 没有基准快照 / 3 读写失败
//!   4 有动作执行失败 / 5 被安全护栏拒绝 / 6 需要管理员权限

#[cfg(windows)]
pub mod layout;
#[cfg(windows)]
pub mod screen;

#[cfg(windows)]
use deskkit_core::{Kind, Tier};
#[cfg(windows)]
use deskkit_win::{close, compare, snapshot, win};
// 常量与平台无关，usage() 在非 Windows 上也要用
use deskkit_core as core;
#[cfg(windows)]
use std::collections::BTreeMap;
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
        r#"deskkit {ver} —— 内存基准线 + 桌面图标布局（每个界面操作都有命令）

用法: deskkit <命令> [选项]

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

  svc                        原子操作：停止 / 启动单个服务
      svc stop <服务名> --execute
      svc start <服务名> --execute

  config                     数据目录（基准线存哪儿）
      config get             当前生效目录 + 从哪来的
      config set <目录>       改成你指定的目录（会写配置文件）
      config clear           恢复系统默认

  commands                   列出所有命令与参数（JSON 供 AI 发现）

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
      --include-self         把采集器自己也算进去
      --self-destruct        采集后删任务和本 exe
      --task NAME            计划任务名（默认 {task}）

  compare                    和最近一次基准做 diff
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
        ver = deskkit_core::VERSION,
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
    deskkit_core::config::config_path().with_file_name("theme.txt")
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
                    eprintln!("用法: deskkit theme set light|dark");
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
    use deskkit_core::config as cfg;
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
                deskkit_win::win::open_folder(&effective);
            }
        }
        "set" => {
            let dir = match pos.get(1) {
                Some(d) => d.to_string(),
                None => {
                    eprintln!("用法: deskkit config set <目录>");
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
                        println!("  提示 : 新目录还没有基准线的话，用 `deskkit baseline capture`");
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
    use deskkit_core::bundle;
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
                    eprintln!("[x] 没有基准快照：{}（先跑 deskkit collect）", root.display());
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
                    eprintln!("用法: deskkit baseline import <包文件.mbz> [--label NAME] [--force]");
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
                        println!("  注意 : 导入的快照是「最新」的一份，deskkit status/trim 会以它为基准");
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
    let dir = match compare::find_latest_baseline(root) {
        Some(d) => d,
        None => {
            eprintln!(
                "[x] 没有基准快照：{}（先跑 deskkit collect）",
                root.display()
            );
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
            ws_mb: p.ws_mb,
            priv_mb: p.priv_mb,
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
fn item_json(it: &core::PlanItem, include_caution: bool) -> String {
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
        "{{\"kind\":\"{}\",\"name\":{},\"pids\":[{}],\"members\":[{}],\"instances\":{},\"mem_mb\":{:.1},\"tier\":\"{}\",\"protect\":{},\"protect_by\":{},\"actionable\":{},\"start_mode\":{},\"reason\":{},\"path\":{}}}",
        core::kind_name(it.kind),
        jstr(&it.name),
        pids.join(","),
        members.join(","),
        it.instances,
        it.mem_mb,
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

    if json {
        println!(
            "{{\"baseline\":{{\"dir\":{},\"captured_at\":{},\"uptime_sec\":{}}},\"live\":{{\"used_mb\":{},\"load_pct\":{},\"process_count\":{}}},\"outliers\":{{\"total\":{},\"suggested\":{},\"caution\":{},\"blocked\":{},\"actionable\":{},\"actionable_mb\":{:.1}}},\"is_admin\":{}}}",
            jstr(&base.dir.to_string_lossy()),
            jstr(&base.captured),
            base.uptime,
            live.used_mb,
            live.load_pct,
            live.proc_count,
            plan.len(),
            sg,
            ca,
            bl,
            act,
            act_mb,
            win::is_admin()
        );
        return;
    }

    println!("基准快照 : {}", base.dir.display());
    println!(
        "           采集于 {}（开机第 {} 秒）",
        base.captured, base.uptime
    );
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
    println!("可处理   : {} 项，约 {:.0} MB", act, act_mb);
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
    let all = has_flag(args, "--all");

    if json {
        let items: Vec<String> = plan
            .iter()
            .filter(|p| all || p.tier != Tier::Blocked)
            .map(|p| item_json(p, include_caution))
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
    let est_mb: f64 = targets.iter().map(|p| p.mem_mb).sum();

    if !execute {
        if json {
            let items: Vec<String> = targets
                .iter()
                .map(|p| item_json(p, include_caution))
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
    ("kill", "原子操作：终止指定进程（名字或 PID）", "<name|pid> --execute --out --grace-wait --force --no-force", "windows"),
    ("svc", "原子操作：停止/启动单个服务", "stop|start <service> --execute", "windows"),
    ("protect", "保护名单（勾选保护 = 永不处理）", "list|add|remove|clear [名字...]", "windows"),
    ("collect", "采集一次快照", "--out --at --include-self --self-destruct --task", "windows"),
    ("compare", "和最近一次基准做 diff", "--min-mb --exclude", "windows"),
    ("baseline", "基准线：查看/校验/采集当前态/导出/导入", "info|verify|capture|export|import [包] [--to FILE] [--label NAME]", "all"),
    ("install", "注册开机计划任务", "--out --at --task", "windows"),
    ("uninstall", "删除开机计划任务", "--task", "windows"),
    ("layout", "桌面图标布局：保存/差异/恢复/修复缓存", "list|save|restore|diff|show|delete|status|probe|repair|dir [名字] [--execute --yes --positions --open]", "windows"),
    ("theme", "外观：查看/切换浅色深色", "get|set light|dark", "all"),
    ("screen", "息屏：只关显示器，不锁屏（热键退出 / 定时自动亮）", "off|status|duration|key [--seconds N --exit-key K --delay N --dry-run]", "windows"),
    ("ui", "打开界面（可指定落在哪一页）", "--page layouts|baseline|protect|boot|cli|tools|settings", "windows"),
    ("config", "数据目录配置：查看/修改/清除", "get|set <目录>|clear [--open]", "all"),
    ("commands", "列出所有命令与参数（本表）", "", "all"),
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
            jstr(deskkit_core::VERSION),
            jstr(&core::default_root()),
            items.join(","),
            actions
        );
        return;
    }
    println!(
        "deskkit {}  数据目录 {}",
        deskkit_core::VERSION,
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
            eprintln!("用法: deskkit kill <进程名|PID> [--execute] [--grace-wait N] [--force|--no-force] [--json]");
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
                .map(|p| format!("{{\"pid\":{},\"name\":{},\"mem_mb\":{:.1},\"path\":{}}}", p.pid, jstr(&p.name), p.ws_mb, jstr(&p.path)))
                .collect();
            println!("{{\"mode\":\"dry_run\",\"target\":{},\"count\":{},\"items\":[{}]}}", jstr(&target), hits.len(), arr.join(","));
        } else {
            println!("[dry-run] 将终止 {} 个进程（加 --execute 才执行）", hits.len());
            for p in &hits {
                println!("  pid {:<7} {:<24} {:>7.1} MB  {}", p.pid, p.name, p.ws_mb, p.path);
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
        eprintln!("用法: deskkit svc <stop|start> <服务名> [--execute] [--json]");
        std::process::exit(EXIT_USAGE);
    }
    let action = pos[0].to_lowercase();
    let name = pos[1].clone();
    if action != "stop" && action != "start" {
        eprintln!("动作只支持 stop / start");
        std::process::exit(EXIT_USAGE);
    }
    if action == "stop" && core::tier_for(Kind::Service, &name, "") == Tier::Blocked {
        eprintln!("[!] 拒绝：{} 属于关键系统服务", name);
        std::process::exit(EXIT_REFUSED);
    }
    if !execute {
        if json {
            println!(
                "{{\"mode\":\"dry_run\",\"action\":{},\"service\":{}}}",
                jstr(&action),
                jstr(&name)
            );
        } else {
            println!("[dry-run] 将 {} 服务 {}（加 --execute 才执行）", if action == "stop" { "停止" } else { "启动" }, name);
        }
        return;
    }
    let r = if action == "stop" {
        win::stop_service(&name)
    } else {
        win::start_service(&name)
    };
    match r {
        Ok(()) => {
            if json {
                println!("{{\"ok\":true,\"action\":{},\"service\":{}}}", jstr(&action), jstr(&name));
            } else {
                println!("已{}：{}", if action == "stop" { "停止" } else { "启动" }, name);
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

#[cfg(windows)]
fn cmd_plan(args: &[String]) {
    let root = root_arg(args);
    let json = want_json(args);
    let include_caution = has_flag(args, "--include-caution");
    let (base, plan) = match build_current_plan(&root, num_arg(args, "--min-mb", 10.0)) {
        Ok(x) => x,
        Err(code) => std::process::exit(code),
    };
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
        let items: Vec<String> = plan.iter().map(|p| item_json(p, include_caution)).collect();
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
        println!("线外 {} 项，可处理 {} 项（约 {:.0} MB）", plan.len(), act, act_mb);
        for p in &plan {
            println!(
                "  {} {:<9} {:<28} {:>7.0} MB",
                core::kind_name(p.kind),
                tier_label(p.tier),
                p.name,
                p.mem_mb
            );
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
                eprintln!("用法: deskkit protect add <名字[,名字...]>");
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
fn cmd_collect(args: &[String]) {
    let out = root_arg(args);
    let at = num_arg(args, "--at", 60.0) as u64;
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

    if json {
        println!(
            "{{\"ok\":true,\"dir\":{},\"uptime_sec\":{},\"process_count\":{},\"service_count\":{},\"used_mb\":{},\"load_pct\":{},\"collect_ms\":{},\"write_ms\":{}}}",
            jstr(&sub.to_string_lossy()),
            snap.uptime_s,
            snap.procs.len(),
            snap.svcs.len(),
            snap.used_phys_mb,
            snap.load_pct,
            snap.collect_ms,
            write_ms
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

    if !want_json(args) {
        if let Err(e) = compare::compare(&root, min_mb, &excludes, true) {
            eprintln!("[x] 对比失败: {}", e);
            std::process::exit(EXIT_IO);
        }
        return;
    }

    let base = match load_baseline(&root) {
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


/// `deskkit ui`：把界面拉起来。页面切换本身也有命令入口，
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
        cmd.env("DESKKIT_PAGE", page.trim());
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

/// 注册开机采集任务。XML、拷 exe、调 schtasks 都在 deskkit-win::task 里，
/// GUI 的按钮走的是同一份实现。
#[cfg(windows)]
fn cmd_install(args: &[String]) {
    let out = root_arg(args);
    let at = num_arg(args, "--at", 60.0) as u64;
    let task = task_arg(args);
    let json = want_json(args);

    let cur = std::env::current_exe().expect("取不到自身路径");
    let res = deskkit_win::task::install(&cur, &out, at, &task);
    let ok = res.is_ok();
    let target = res
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    if json {
        println!(
            "{{\"ok\":{},\"task\":{},\"exe\":{},\"at_sec\":{},\"out\":{}}}",
            ok,
            jstr(&task),
            jstr(&target),
            at,
            jstr(&out.to_string_lossy())
        );
    } else if ok {
        println!("[OK] 计划任务已注册：{}（开机触发，SYSTEM，无窗口）", task);
        println!("     下次开机会自动采集一次，随后任务和 exe 自动消失");
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
    let res = deskkit_win::task::uninstall(&task);
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
            println!("deskkit {}", deskkit_core::VERSION);
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
                eprintln!("deskkit screen 只支持 Windows");
                std::process::exit(EXIT_USAGE);
            }
        }
        "ui" | "gui" => {
            #[cfg(windows)]
            {
                cmd_ui(&argv[2..]);
            }
            #[cfg(not(windows))]
            eprintln!("deskkit ui 只支持 Windows");
            return;
        }
        "layout" | "layouts" | "desk" => {
            #[cfg(windows)]
            {
                std::process::exit(layout::cmd_layout(&argv[2..]));
            }
            #[cfg(not(windows))]
            {
                eprintln!("deskkit layout 只支持 Windows（依赖 explorer 的桌面 ListView）");
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
        eprintln!("deskkit 的采集/清理与桌面布局只支持 Windows；纯逻辑部分用 `cargo test -p deskkit-core` 验证。");
        std::process::exit(EXIT_USAGE);
    }
}
