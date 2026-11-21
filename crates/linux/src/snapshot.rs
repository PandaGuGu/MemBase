//! 采集与落盘（Linux）：字段结构与 Windows 版一致，`processes.csv` /
//! `processes_by_name.csv` / `services.csv` / `system.json` 都能被
//! 现有的对比、打包、界面代码直接读取。

use crate::procfs;
use crate::win;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct ProcRow {
    pub name: String,
    pub pid: u32,
    pub ppid: u32,
    pub session: u32,
    /// `None` = 读不到（别的用户 / 内核线程），和「真的是 0 MB」区分开
    pub ws_mb: Option<f64>,
    pub priv_mb: Option<f64>,
    pub paged_sys_mb: f64,
    pub peak_ws_mb: f64,
    pub virtual_mb: f64,
    pub cpu_seconds: f64,
    pub handles: u32,
    pub threads: u32,
    pub start_time: String,
    /// 创建时间：开机以来的时钟滴答，和 PID 一起作为进程身份
    pub created: u64,
    pub user: String,
    pub path: String,
    pub command: String,
}

pub struct SvcRow {
    pub name: String,
    pub display: String,
    pub state: String,
    pub start_mode: String,
    pub pid: u32,
    pub host: String,
    pub host_ws: Option<f64>,
    pub host_priv: Option<f64>,
    pub account: String,
    pub path: String,
}

pub struct Snapshot {
    pub captured_local: String,
    pub captured_utc: String,
    pub boot_time: String,
    pub uptime_s: u64,
    pub total_phys_mb: u64,
    pub avail_phys_mb: u64,
    pub used_phys_mb: u64,
    pub load_pct: u32,
    pub commit_limit_mb: u64,
    pub committed_mb: u64,
    pub cpus: u32,
    pub computer: String,
    pub user: String,
    pub drive: String,
    pub os_caption: String,
    pub os_version: String,
    pub os_build: String,
    pub elevated: bool,
    pub collect_ms: u64,
    pub procs: Vec<ProcRow>,
    pub svcs: Vec<SvcRow>,
}

const MB: f64 = 1048576.0;

fn mb(bytes: u64) -> f64 {
    (bytes as f64 / MB * 100.0).round() / 100.0
}

fn os_release() -> (String, String) {
    let text = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let get = |key: &str| -> String {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    (get("PRETTY_NAME"), get("VERSION_ID"))
}

fn kernel_release() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// 内核线程：没有用户态可执行文件，且父进程是 kthreadd(2)
fn is_kernel_thread(p: &procfs::ProcInfo) -> bool {
    p.ppid == 2 && p.exe_path().is_empty()
}

pub fn snapshot_now(exclude_self: bool) -> Snapshot {
    let started = std::time::Instant::now();
    let mem = procfs::meminfo();
    let uptime = procfs::uptime_secs();
    let my_pid = std::process::id();
    let boot = procfs::boot_time_secs();

    let mut procs: Vec<ProcRow> = Vec::new();
    for p in procfs::list() {
        if p.pid == 0 || p.pid == 1 || p.pid == 2 {
            continue;
        }
        if exclude_self && p.pid == my_pid {
            continue;
        }
        if is_kernel_thread(&p) {
            continue;
        }
        if p.state == 'Z' || p.state == 'X' {
            continue;
        }
        let path = p.exe_path();
        // 读不到 exe 也不一定是没权限：内核线程/已退出的进程都会这样，
        // 但这里已经把内核线程和僵尸过滤掉了，剩下的按「未知路径」处理。
        let ws = if p.rss_kb > 0 {
            Some(mb(p.rss_kb * 1024))
        } else {
            None
        };
        let priv_mb = if p.rss_anon_kb > 0 {
            Some(mb(p.rss_anon_kb * 1024))
        } else {
            None
        };
        let created_at = boot + (p.start_ticks as f64 / procfs::CLK_TCK) as u64;
        let start_time = chrono::DateTime::from_timestamp(created_at as i64, 0)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_default();
        procs.push(ProcRow {
            name: p.name.clone(),
            pid: p.pid,
            ppid: p.ppid,
            session: 0,
            ws_mb: ws,
            priv_mb,
            paged_sys_mb: 0.0,
            peak_ws_mb: mb(p.hwm_kb * 1024),
            virtual_mb: mb(p.vsize_kb * 1024),
            cpu_seconds: ((p.cpu_ticks as f64 / procfs::CLK_TCK) * 100.0).round() / 100.0,
            handles: p.fd_count(),
            threads: p.threads,
            start_time,
            created: p.start_ticks,
            user: procfs::uid_name(p.uid),
            path,
            command: p.cmdline(),
        });
    }
    procs.sort_by(|a, b| {
        b.ws_mb
            .unwrap_or(0.0)
            .partial_cmp(&a.ws_mb.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ---- 服务：systemd ----
    let by_pid: BTreeMap<u32, &ProcRow> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut svcs: Vec<SvcRow> = Vec::new();
    for s in win::list_services() {
        let host_proc = by_pid.get(&s.main_pid);
        let host = host_proc.map(|p| p.name.clone()).unwrap_or_default();
        svcs.push(SvcRow {
            name: s.name.clone(),
            display: s.description.clone(),
            state: s.state(),
            start_mode: s.start_mode(),
            pid: s.main_pid,
            host,
            host_ws: host_proc.and_then(|p| p.ws_mb),
            host_priv: host_proc.and_then(|p| p.priv_mb),
            account: String::new(),
            path: if s.exec_path.is_empty() {
                s.fragment_path.clone()
            } else {
                s.exec_path.clone()
            },
        });
    }
    svcs.sort_by(|a, b| a.name.cmp(&b.name));

    let total_mb = mem.total_kb / 1024;
    let avail_mb = mem.available_kb / 1024;
    let used_mb = total_mb.saturating_sub(avail_mb);
    let load = if total_mb > 0 {
        ((used_mb as f64 / total_mb as f64) * 100.0).round() as u32
    } else {
        0
    };
    let (os_caption, os_version) = os_release();

    Snapshot {
        captured_local: win::now_local_string(),
        captured_utc: win::now_utc_string(),
        boot_time: win::boot_time_string(uptime),
        uptime_s: uptime,
        total_phys_mb: total_mb,
        avail_phys_mb: avail_mb,
        used_phys_mb: used_mb,
        load_pct: load.min(100),
        commit_limit_mb: (mem.total_kb + mem.swap_total_kb) / 1024,
        committed_mb: mem.committed_kb / 1024,
        cpus: win::logical_processors(),
        computer: win::computer_name(),
        user: win::user_name(),
        drive: "/".to_string(),
        os_caption: if os_caption.is_empty() {
            "Linux".to_string()
        } else {
            os_caption
        },
        os_version,
        os_build: kernel_release(),
        elevated: win::is_admin(),
        collect_ms: started.elapsed().as_millis() as u64,
        procs,
        svcs,
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn opt_mb(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.2}", x),
        None => String::new(),
    }
}

fn opt_mb_or_dash(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.2}", x),
        None => "—".to_string(),
    }
}

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn write_text(path: &Path, content: &str) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(content.as_bytes())
}

pub fn write_snapshot(dir: &Path, snap: &Snapshot) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;

    // 交互用户：桌面外壳的属主。采集器自己可能是 root / cron，不能拿它冒充登录用户。
    let interactive = procfs::desktop_session_user().unwrap_or_default();

    let mut j = String::from("{\n");
    let mut kv: Vec<(String, String)> = vec![
        ("CapturedAtLocal".into(), jstr(&snap.captured_local)),
        ("CapturedAtUtc".into(), jstr(&snap.captured_utc)),
        ("LastBootTime".into(), jstr(&snap.boot_time)),
        ("UptimeSeconds".into(), snap.uptime_s.to_string()),
        ("CollectMs".into(), snap.collect_ms.to_string()),
        ("Elevated".into(), snap.elevated.to_string()),
        ("Build".into(), jstr(&membase_core::build_id())),
        ("ComputerName".into(), jstr(&snap.computer)),
        ("OsCaption".into(), jstr(&snap.os_caption)),
        ("OsVersion".into(), jstr(&snap.os_version)),
        ("OsBuild".into(), jstr(&snap.os_build)),
        ("Manufacturer".into(), jstr("")),
        ("Model".into(), jstr("")),
        (
            "TotalPhysicalMemoryMB".into(),
            snap.total_phys_mb.to_string(),
        ),
        (
            "TotalVisibleMemoryMB".into(),
            snap.total_phys_mb.to_string(),
        ),
        (
            "FreePhysicalMemoryMB".into(),
            snap.avail_phys_mb.to_string(),
        ),
        ("UsedPhysicalMemoryMB".into(), snap.used_phys_mb.to_string()),
        ("MemoryLoadPercent".into(), snap.load_pct.to_string()),
        ("CommitLimitMB".into(), snap.commit_limit_mb.to_string()),
        ("CommittedMB".into(), snap.committed_mb.to_string()),
        ("PageFileAllocatedMB".into(), "null".into()),
        ("PageFileUsedMB".into(), "null".into()),
        ("PageFilePeakMB".into(), "null".into()),
        ("LogicalProcessors".into(), snap.cpus.to_string()),
        ("PhysicalProcessors".into(), "null".into()),
        (
            "LoggedOnUser".into(),
            jstr(if interactive.is_empty() {
                &snap.user
            } else {
                &interactive
            }),
        ),
        ("InteractiveUser".into(), jstr(&interactive)),
        ("LoggedOn".into(), (!interactive.is_empty()).to_string()),
        ("CollectorUser".into(), jstr(&snap.user)),
        ("SystemDrive".into(), jstr(&snap.drive)),
    ];
    let n = kv.len();
    for (i, (k, v)) in kv.drain(..).enumerate() {
        let comma = if i + 1 == n { "" } else { "," };
        j.push_str(&format!("  {}: {}{}\n", jstr(&k), v, comma));
    }
    j.push_str("}\n");
    write_text(&dir.join("system.json"), &j)?;

    let mut c = String::from(
        "CapturedAt,ProcessName,PID,SessionId,WorkingSetMB,PrivateMB,PagedSystemMB,PeakWorkingSetMB,VirtualMB,CpuSeconds,Handles,Threads,StartTime,UserName,Path,CommandLine\r\n",
    );
    for p in &snap.procs {
        c.push_str(&format!(
            "{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{},{},{},{},{},{}\r\n",
            csv_escape(&snap.captured_local),
            csv_escape(&p.name),
            p.pid,
            p.session,
            opt_mb(p.ws_mb),
            opt_mb(p.priv_mb),
            p.paged_sys_mb,
            p.peak_ws_mb,
            p.virtual_mb,
            p.cpu_seconds,
            p.handles,
            p.threads,
            csv_escape(&p.start_time),
            csv_escape(&p.user),
            csv_escape(&p.path),
            csv_escape(&p.command)
        ));
    }
    write_text(&dir.join("processes.csv"), &c)?;

    let mut agg: BTreeMap<String, (u32, f64, f64, u32)> = BTreeMap::new();
    for p in &snap.procs {
        let e = agg
            .entry(p.name.clone())
            .or_insert((0u32, 0.0f64, 0.0f64, 0u32));
        e.0 += 1;
        match p.ws_mb {
            Some(v) => e.1 += v,
            None => e.3 += 1,
        }
        if let Some(v) = p.priv_mb {
            e.2 += v;
        }
    }
    let mut rows: Vec<(String, u32, f64, f64, u32)> = agg
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                v.0,
                (v.1 * 100.0).round() / 100.0,
                (v.2 * 100.0).round() / 100.0,
                v.3,
            )
        })
        .collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut c = String::from("ProcessName,Instances,WorkingSetMB,PrivateMB,UnknownInstances\r\n");
    for (name, inst, ws, priv_mb, unknown) in &rows {
        c.push_str(&format!(
            "{},{},{:.2},{:.2},{}\r\n",
            csv_escape(name),
            inst,
            ws,
            priv_mb,
            unknown
        ));
    }
    write_text(&dir.join("processes_by_name.csv"), &c)?;

    let mut c = String::from(
        "ServiceName,DisplayName,State,StartMode,DelayedAutoStart,PID,HostProcess,HostWorkingSetMB,HostPrivateMB,ServiceAccount,ExecutablePath\r\n",
    );
    for s in &snap.svcs {
        c.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\r\n",
            csv_escape(&s.name),
            csv_escape(&s.display),
            csv_escape(&s.state),
            csv_escape(&s.start_mode),
            "False",
            s.pid,
            csv_escape(&s.host),
            opt_mb(s.host_ws),
            opt_mb(s.host_priv),
            csv_escape(&s.account),
            csv_escape(&s.path)
        ));
    }
    write_text(&dir.join("services.csv"), &c)?;

    let total_ws: f64 = snap.procs.iter().filter_map(|p| p.ws_mb).sum();
    let total_priv: f64 = snap.procs.iter().filter_map(|p| p.priv_mb).sum();
    let unknown_mem = snap.procs.iter().filter(|p| p.ws_mb.is_none()).count();
    let total_threads: u32 = snap.procs.iter().map(|p| p.threads).sum();
    let total_handles: u32 = snap.procs.iter().map(|p| p.handles).sum();
    let running = snap.svcs.iter().filter(|s| s.state == "Running").count();
    let stopped = snap.svcs.iter().filter(|s| s.state == "Stopped").count();

    let mut m = String::new();
    m.push_str("# 开机内存基准线报告（membase / Rust · Linux）\n\n");
    m.push_str(&format!("- 采集时间：{}\n", snap.captured_local));
    m.push_str(&format!(
        "- 上次开机：{}（采集时已运行 {} 秒）\n",
        snap.boot_time, snap.uptime_s
    ));
    m.push_str(&format!(
        "- 采集耗时：{} 毫秒（受系统负载影响，同一台机器不同时刻可能差数倍，不是性能指标）\n",
        snap.collect_ms
    ));
    m.push_str(&format!(
        "- 采集程序：membase {} ({})\n",
        membase_core::VERSION,
        membase_core::build_id()
    ));
    m.push_str(&format!(
        "- 采集身份：{}（{}）\n",
        snap.user,
        if snap.elevated {
            "root，能看到全部进程"
        } else {
            "普通用户，别的账户的进程内存读不全"
        }
    ));
    if interactive.is_empty() {
        m.push_str("- 交互用户：无（未检测到桌面外壳）\n");
    } else {
        m.push_str(&format!("- 交互用户：{}（桌面已登录）\n", interactive));
    }
    m.push_str(&format!(
        "- 系统：{} {}（内核 {}）\n",
        snap.os_caption, snap.os_version, snap.os_build
    ));
    m.push_str(&format!("- 机器：{}\n\n", snap.computer));
    m.push_str("## 整机内存\n\n| 指标 | 数值 |\n| --- | ---: |\n");
    m.push_str(&format!("| 物理内存总量 | {} MB |\n", snap.total_phys_mb));
    m.push_str(&format!("| 已用 | {} MB |\n", snap.used_phys_mb));
    m.push_str(&format!("| 可用 | {} MB |\n", snap.avail_phys_mb));
    m.push_str(&format!("| 内存占用率 | {} % |\n", snap.load_pct));
    m.push_str(&format!(
        "| 已提交(Committed_AS) | {} MB / 上限 {} MB |\n",
        snap.committed_mb, snap.commit_limit_mb
    ));
    m.push_str(&format!("| 进程数 | {} |\n", snap.procs.len()));
    m.push_str(&format!("| 线程数 | {} |\n", total_threads));
    m.push_str(&format!("| 句柄数（fd） | {} |\n", total_handles));
    if unknown_mem > 0 {
        m.push_str(&format!(
            "| 工作集合计（{} 个进程读不到） | {:.0} MB |\n",
            unknown_mem, total_ws
        ));
        m.push_str(&format!(
            "| 专用内存合计（{} 个进程读不到） | {:.0} MB |\n",
            unknown_mem, total_priv
        ));
    } else {
        m.push_str(&format!("| 工作集合计 | {:.0} MB |\n", total_ws));
        m.push_str(&format!("| 专用内存合计 | {:.0} MB |\n", total_priv));
    }
    m.push_str(&format!(
        "| 服务 | 运行 {} / 停止 {} / 共 {} |\n\n",
        running,
        stopped,
        snap.svcs.len()
    ));

    m.push_str("## 进程名聚合 Top 25\n\n| 进程名 | 实例数 | 工作集(MB) | 专用(MB) | 读不到 |\n| --- | ---: | ---: | ---: | ---: |\n");
    for (name, inst, ws, priv_mb, unknown) in rows.iter().take(25) {
        m.push_str(&format!(
            "| {} | {} | {:.2} | {:.2} | {} |\n",
            name, inst, ws, priv_mb, unknown
        ));
    }
    m.push_str("\n## 单进程 Top 20\n\n| 进程名 | PID | 工作集(MB) | 专用(MB) | 线程 | fd |\n| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for p in snap.procs.iter().take(20) {
        m.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            p.name,
            p.pid,
            opt_mb_or_dash(p.ws_mb),
            opt_mb_or_dash(p.priv_mb),
            p.threads,
            p.handles
        ));
    }
    m.push_str("\n## 服务宿主进程 Top 20（同一宿主只算一次）\n\n| 宿主进程 | PID | 承载服务数 | 工作集(MB) |\n| --- | ---: | ---: | ---: |\n");
    let mut hosts: BTreeMap<u32, (String, usize, f64)> = BTreeMap::new();
    for s in &snap.svcs {
        if s.pid == 0 {
            continue;
        }
        let e = hosts
            .entry(s.pid)
            .or_insert((s.host.clone(), 0usize, s.host_ws.unwrap_or(0.0)));
        e.1 += 1;
    }
    let mut host_rows: Vec<(u32, String, usize, f64)> = hosts
        .into_iter()
        .map(|(pid, (name, cnt, ws))| (pid, name, cnt, ws))
        .collect();
    host_rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
    for (pid, name, cnt, ws) in host_rows.iter().take(20) {
        m.push_str(&format!("| {} | {} | {} | {:.2} |\n", name, pid, cnt, ws));
    }
    write_text(&dir.join("summary.md"), &m)?;

    Ok(())
}

/// 等待到「开机后第 at_secs 秒」。已经过了就不再等。
pub fn wait_until_boot_offset(at_secs: u64) {
    let up = procfs::uptime_secs();
    if up >= at_secs {
        return;
    }
    let delta = at_secs - up;
    println!(
        "[..] 已开机 {} 秒，等待 {} 秒到第 {} 秒",
        up, delta, at_secs
    );
    let mut left = delta;
    while left > 0 {
        let step = left.min(20);
        win::sleep_ms((step * 1000) as u32);
        left -= step;
    }
}

pub fn new_run_dir(root: &Path) -> PathBuf {
    let stamp = win::now_local_string()
        .replace('-', "")
        .replace(':', "")
        .replace(' ', "_");
    let dir = root.join(format!("run_{}", stamp));
    let _ = fs::create_dir_all(&dir);
    dir
}
