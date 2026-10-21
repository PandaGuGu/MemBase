//! 采集与落盘：字段结构与 PowerShell 版完全一致，
//! 这样 Rust 采的基准线可以直接被 PS 版对比工具读，反之亦然。

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
    /// `None` = 没权限读到（未提权时别的账户的进程基本都是 None），
    /// 和「真的是 0 MB」必须区分开：写 0 会让 diff 报出"内存暴跌"的假结论。
    pub ws_mb: Option<f64>,
    pub priv_mb: Option<f64>,
    pub paged_sys_mb: f64,
    pub peak_ws_mb: f64,
    pub virtual_mb: f64,
    pub cpu_seconds: f64,
    pub handles: u32,
    pub threads: u32,
    pub start_time: String,
    /// 创建时间（FILETIME 100ns）：和 PID 一起作为进程身份，防止 PID 复用误伤
    pub created: u64,
    pub user: String,
    pub path: String,
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
    /// 采集时是否已提权。未提权时大部分进程的内存是 `None`，基准不可用于对比。
    pub elevated: bool,
    /// 枚举进程/服务本身花了多少毫秒（不含落盘）
    pub collect_ms: u64,
    pub procs: Vec<ProcRow>,
    pub svcs: Vec<SvcRow>,
}

const MB: f64 = 1048576.0;

fn mb(bytes: u64) -> f64 {
    (bytes as f64 / MB * 100.0).round() / 100.0
}

pub fn snapshot_now(exclude_self: bool) -> Snapshot {
    let started = std::time::Instant::now();
    let mem = win::memory_status();
    let uptime = win::uptime_secs();
    let my_pid = std::process::id();

    // ---- 进程 ----
    let mut procs: Vec<ProcRow> = Vec::new();
    for pe in win::snapshot_processes() {
        let pid = pe.th32ProcessID;
        if pid == 0 || pid == 4 {
            continue;
        }
        if exclude_self && pid == my_pid {
            continue;
        }
        let raw = win::from_wide_buf(&pe.szExeFile);
        let name = raw
            .strip_suffix(".exe")
            .or_else(|| raw.strip_suffix(".EXE"))
            .unwrap_or(&raw)
            .to_string();

        let mut path = String::new();
        let mut start_time = String::new();
        let mut created = 0u64;
        let mut handles = 0u32;
        let mut ws_mb: Option<f64> = None;
        let mut priv_mb: Option<f64> = None;
        let mut peak_ws_mb = 0.0f64;
        let mut paged_sys_mb = 0.0f64;
        let mut virtual_mb = 0.0f64;
        let mut cpu_seconds = 0.0f64;
        let mut user = String::new();

        if let Some(h) = win::open_process(
            pid,
            win::PROCESS_QUERY_LIMITED_INFORMATION | win::PROCESS_VM_READ,
        ) {
            path = win::process_image_path(h.0);
            start_time = win::process_start_time(h.0);
            created = win::process_created(h.0).unwrap_or(0);
            handles = win::process_handle_count(h.0);
            user = win::process_user(h.0);
            cpu_seconds = win::process_cpu_seconds(h.0);
            if let Some(pmc) = win::process_memory(h.0) {
                ws_mb = Some(mb(pmc.WorkingSetSize as u64));
                priv_mb = Some(mb(pmc.PrivateUsage as u64));
                peak_ws_mb = mb(pmc.PeakWorkingSetSize as u64);
                paged_sys_mb = mb(pmc.QuotaPagedPoolUsage as u64);
                virtual_mb = mb(pmc.PagefileUsage as u64);
            }
        } else if let Some(h) = win::open_process(pid, win::PROCESS_QUERY_LIMITED_INFORMATION) {
            // 提权进程只拿得到路径和线程数
            path = win::process_image_path(h.0);
            start_time = win::process_start_time(h.0);
            created = win::process_created(h.0).unwrap_or(0);
            handles = win::process_handle_count(h.0);
            user = win::process_user(h.0);
            cpu_seconds = win::process_cpu_seconds(h.0);
        }

        procs.push(ProcRow {
            name,
            pid,
            ppid: pe.th32ParentProcessID,
            session: win::session_id(pid),
            ws_mb,
            priv_mb,
            paged_sys_mb,
            peak_ws_mb,
            virtual_mb,
            cpu_seconds,
            handles,
            threads: pe.cntThreads,
            start_time,
            created,
            user,
            path,
        });
    }
    // 读不到内存的排在最后（用 -1 当哨兵，别让它插到 Top 里去）
    procs.sort_by(|a, b| {
        b.ws_mb
            .unwrap_or(-1.0)
            .partial_cmp(&a.ws_mb.unwrap_or(-1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // pid -> (进程名, 工作集, 专用)
    let mut pid_map: BTreeMap<u32, (String, Option<f64>, Option<f64>)> = BTreeMap::new();
    for p in &procs {
        pid_map.insert(p.pid, (p.name.clone(), p.ws_mb, p.priv_mb));
    }

    // ---- 服务 ----
    let mut svcs: Vec<SvcRow> = Vec::new();
    if let Some(scm) = win::open_scm() {
        for e in win::enum_services(scm.0) {
            let mut start_mode = String::from("Unknown");
            let mut account = String::new();
            let mut bin_path = String::new();
            if let Some(svc) = win::open_service(scm.0, &e.name, win::SERVICE_QUERY_CONFIG) {
                if let Some(cfg) = win::query_service_config(svc.0) {
                    start_mode = win::start_mode_name(cfg.start_type).to_string();
                    account = cfg.account;
                    bin_path = cfg.binary_path;
                }
            }
            let (host, host_ws, host_priv) = match pid_map.get(&e.pid) {
                Some((n, w, p)) => (n.clone(), *w, *p),
                None => (String::new(), None, None),
            };
            svcs.push(SvcRow {
                name: e.name,
                display: e.display,
                state: win::service_state_name(e.state).to_string(),
                start_mode,
                pid: e.pid,
                host,
                host_ws,
                host_priv,
                account,
                path: bin_path,
            });
        }
    }
    svcs.sort_by(|a, b| {
        b.host_ws
            .unwrap_or(0.0)
            .partial_cmp(&a.host_ws.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let os = win::os_info();

    Snapshot {
        captured_local: win::now_local_string(),
        captured_utc: win::now_utc_string(),
        boot_time: win::boot_time_string(uptime),
        uptime_s: uptime,
        total_phys_mb: mem.ullTotalPhys / 1024 / 1024,
        avail_phys_mb: mem.ullAvailPhys / 1024 / 1024,
        used_phys_mb: (mem.ullTotalPhys - mem.ullAvailPhys) / 1024 / 1024,
        load_pct: mem.dwMemoryLoad,
        commit_limit_mb: mem.ullTotalPageFile / 1024 / 1024,
        committed_mb: (mem.ullTotalPageFile - mem.ullAvailPageFile) / 1024 / 1024,
        cpus: win::logical_processors(),
        computer: win::computer_name(),
        user: win::user_name(),
        drive: win::system_drive(),
        os_caption: os.caption,
        os_version: os.version,
        os_build: os.build,
        elevated: win::is_elevated(),
        collect_ms: started.elapsed().as_millis() as u64,
        procs,
        svcs,
    }
}

/// 未知内存写空字段：CSV 里「空」= 没读到，「0.00」= 真的是 0
fn opt_mb(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.2}", x),
        None => String::new(),
    }
}

/// 表格里未知的写成「—」，别让读者以为是 0
fn opt_mb_or_dash(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{:.2}", x),
        None => "—".to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
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

    // ---- system.json ----
    let mut j = String::from("{\n");
    let mut kv: Vec<(String, String)> = vec![
        ("CapturedAtLocal".into(), jstr(&snap.captured_local)),
        ("CapturedAtUtc".into(), jstr(&snap.captured_utc)),
        ("LastBootTime".into(), jstr(&snap.boot_time)),
        ("UptimeSeconds".into(), snap.uptime_s.to_string()),
        ("CollectMs".into(), snap.collect_ms.to_string()),
        ("Elevated".into(), snap.elevated.to_string()),
        ("Build".into(), jstr(&deskkit_core::build_id())),
        ("ComputerName".into(), jstr(&snap.computer)),
        ("OsCaption".into(), jstr(&snap.os_caption)),
        ("OsVersion".into(), jstr(&snap.os_version)),
        ("OsBuild".into(), jstr(&snap.os_build)),
        ("Manufacturer".into(), jstr(&String::new())),
        ("Model".into(), jstr(&String::new())),
        ("TotalPhysicalMemoryMB".into(), snap.total_phys_mb.to_string()),
        ("TotalVisibleMemoryMB".into(), snap.total_phys_mb.to_string()),
        ("FreePhysicalMemoryMB".into(), snap.avail_phys_mb.to_string()),
        ("UsedPhysicalMemoryMB".into(), snap.used_phys_mb.to_string()),
        ("MemoryLoadPercent".into(), snap.load_pct.to_string()),
        ("CommitLimitMB".into(), snap.commit_limit_mb.to_string()),
        ("CommittedMB".into(), snap.committed_mb.to_string()),
        ("PageFileAllocatedMB".into(), "null".into()),
        ("PageFileUsedMB".into(), "null".into()),
        ("PageFilePeakMB".into(), "null".into()),
        ("LogicalProcessors".into(), snap.cpus.to_string()),
        ("PhysicalProcessors".into(), "null".into()),
        ("LoggedOnUser".into(), jstr(&snap.user)),
        ("SystemDrive".into(), jstr(&snap.drive)),
    ];
    let n = kv.len();
    for (i, (k, v)) in kv.drain(..).enumerate() {
        let comma = if i + 1 == n { "" } else { "," };
        j.push_str(&format!("  {}: {}{}\n", jstr(&k), v, comma));
    }
    j.push_str("}\n");
    write_text(&dir.join("system.json"), &j)?;

    // ---- processes.csv ----
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
            ""
        ));
    }
    write_text(&dir.join("processes.csv"), &c)?;

    // ---- processes_by_name.csv ----
    // 末列 UnknownInstances：这一组里有几个实例没读到内存（权限不足）。
    // 有它才算得清「合计 1200 MB」到底是全量还是残缺的合计。
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
        .map(|(k, v)| (k, v.0, (v.1 * 100.0).round() / 100.0, (v.2 * 100.0).round() / 100.0, v.3))
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

    // ---- services.csv ----
    let mut c = String::from(
        "ServiceName,DisplayName,State,StartMode,DelayedAutoStart,PID,HostProcess,HostWorkingSetMB,HostPrivateMB,ServiceAccount,ExecutablePath\r\n",
    );
    for s in &snap.svcs {
        let ws = match s.host_ws {
            Some(v) => format!("{:.2}", v),
            None => String::new(),
        };
        let pv = match s.host_priv {
            Some(v) => format!("{:.2}", v),
            None => String::new(),
        };
        c.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\r\n",
            csv_escape(&s.name),
            csv_escape(&s.display),
            csv_escape(&s.state),
            csv_escape(&s.start_mode),
            "False",
            s.pid,
            csv_escape(&s.host),
            ws,
            pv,
            csv_escape(&s.account),
            csv_escape(&s.path)
        ));
    }
    write_text(&dir.join("services.csv"), &c)?;

    // ---- summary.md ----
    let total_ws: f64 = snap.procs.iter().filter_map(|p| p.ws_mb).sum();
    let total_priv: f64 = snap.procs.iter().filter_map(|p| p.priv_mb).sum();
    let unknown_mem = snap.procs.iter().filter(|p| p.ws_mb.is_none()).count();
    let total_threads: u32 = snap.procs.iter().map(|p| p.threads).sum();
    let total_handles: u32 = snap.procs.iter().map(|p| p.handles).sum();
    let running = snap.svcs.iter().filter(|s| s.state == "Running").count();
    let stopped = snap.svcs.iter().filter(|s| s.state == "Stopped").count();

    let mut m = String::new();
    m.push_str("# 开机内存基准线报告（membase / Rust）\n\n");
    m.push_str(&format!("- 采集时间：{}\n", snap.captured_local));
    m.push_str(&format!(
        "- 上次开机：{}（采集时已运行 {} 秒）\n",
        snap.boot_time, snap.uptime_s
    ));
    m.push_str(&format!("- 采集耗时：{} 毫秒\n", snap.collect_ms));
    m.push_str(&format!("- 采集程序：membase {} ({})\n", deskkit_core::VERSION, deskkit_core::build_id()));
    m.push_str(&format!(
        "- 采集身份：{}（{}）\n",
        snap.user,
        if snap.elevated {
            "已提权"
        } else {
            "未提权，进程内存不完整"
        }
    ));
    m.push_str(&format!("- 机器：{}\n\n", snap.computer));
    m.push_str("## 整机内存\n\n| 指标 | 数值 |\n| --- | ---: |\n");
    m.push_str(&format!("| 物理内存总量 | {} MB |\n", snap.total_phys_mb));
    m.push_str(&format!("| 已用 | {} MB |\n", snap.used_phys_mb));
    m.push_str(&format!("| 可用 | {} MB |\n", snap.avail_phys_mb));
    m.push_str(&format!("| 内存占用率 | {} % |\n", snap.load_pct));
    m.push_str(&format!(
        "| 已提交(Commit) | {} MB / 上限 {} MB |\n",
        snap.committed_mb, snap.commit_limit_mb
    ));
    m.push_str(&format!("| 进程数 | {} |\n", snap.procs.len()));
    m.push_str(&format!("| 线程数 | {} |\n", total_threads));
    m.push_str(&format!("| 句柄数 | {} |\n", total_handles));
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
    if unknown_mem > 0 {
        let (cause, tail) = if snap.elevated {
            (
                "受保护进程：Session 0 伪进程、反作弊等",
                "已提权，剩余这些本来就打不开，不影响排名结论。",
            )
        } else {
            (
                "未提权，别的账户的进程打不开",
                "合计值只算了读得到的部分——这份快照不能用来比进程内存，也不能当瘦身依据。",
            )
        };
        m.push_str(&format!(
            "> ⚠ {} 个进程读不到内存（{}）。{}\n\n",
            unknown_mem, cause, tail
        ));
    }

    m.push_str("## 进程名聚合 Top 25\n\n| 进程名 | 实例数 | 工作集(MB) | 专用(MB) | 读不到 |\n| --- | ---: | ---: | ---: | ---: |\n");
    for (name, inst, ws, priv_mb, unknown) in rows.iter().take(25) {
        m.push_str(&format!(
            "| {} | {} | {:.2} | {:.2} | {} |\n",
            name, inst, ws, priv_mb, unknown
        ));
    }
    m.push_str("\n## 单进程 Top 20\n\n| 进程名 | PID | 工作集(MB) | 专用(MB) | 线程 | 句柄 |\n| --- | ---: | ---: | ---: | ---: | ---: |\n");
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
    let up = win::uptime_secs();
    if up >= at_secs {
        return;
    }
    let delta = at_secs - up;
    println!("[..] 已开机 {} 秒，等待 {} 秒到第 {} 秒", up, delta, at_secs);
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
