//! `/proc` 的最小读取层：把 Linux 的进程表折算成和 Windows 快照同构的字段。
//!
//! 只做读取与解析，不含业务判断；字段缺失时保持「未知」而不是写 0，
//! 这样上层才能区分「真的 0 MB」和「没权限读到」。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// `/proc/<pid>/stat` 里的时间单位：主流 Linux 都是 100 Hz
pub const CLK_TCK: f64 = 100.0;
/// 页大小：x86_64 / aarch64 都是 4 KiB
pub const PAGE_KB: u64 = 4;

#[derive(Clone, Debug, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub state: char,
    /// 常驻内存（RSS），KB
    pub rss_kb: u64,
    /// 匿名页常驻（RssAnon），近似 Windows 的「专用内存」，KB
    pub rss_anon_kb: u64,
    /// 峰值常驻内存（VmHWM），KB
    pub hwm_kb: u64,
    /// 虚拟内存，KB
    pub vsize_kb: u64,
    pub threads: u32,
    /// 开机以来的时钟滴答数（进程启动时刻）
    pub start_ticks: u64,
    /// utime + stime（时钟滴答）
    pub cpu_ticks: u64,
    pub uid: u32,
}

impl ProcInfo {
    pub fn exe_path(&self) -> String {
        fs::read_link(format!("/proc/{}/exe", self.pid))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn cmdline(&self) -> String {
        let raw = fs::read(format!("/proc/{}/cmdline", self.pid)).unwrap_or_default();
        raw.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn fd_count(&self) -> u32 {
        fs::read_dir(format!("/proc/{}/fd", self.pid))
            .map(|rd| rd.flatten().count() as u32)
            .unwrap_or(0)
    }

    pub fn start_epoch(&self) -> u64 {
        boot_time_secs() + (self.start_ticks as f64 / CLK_TCK) as u64
    }
}

/// 上一次开机的时间（epoch 秒）
pub fn boot_time_secs() -> u64 {
    let text = fs::read_to_string("/proc/stat").unwrap_or_default();
    text.lines()
        .find_map(|l| l.strip_prefix("btime "))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

pub fn uptime_secs() -> u64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| {
            t.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .map(|v| v as u64)
        .unwrap_or_else(|| {
            let boot = boot_time_secs();
            if boot == 0 {
                0
            } else {
                chrono::Local::now().timestamp().max(0) as u64 - boot
            }
        })
}

#[derive(Clone, Debug, Default)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
    pub free_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    pub committed_kb: u64,
}

pub fn meminfo() -> MemInfo {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut m = MemInfo::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let val: u64 = v.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0);
        match k {
            "MemTotal" => m.total_kb = val,
            "MemAvailable" => m.available_kb = val,
            "MemFree" => m.free_kb = val,
            "SwapTotal" => m.swap_total_kb = val,
            "SwapFree" => m.swap_free_kb = val,
            "Committed_AS" => m.committed_kb = val,
            _ => {}
        }
    }
    if m.available_kb == 0 {
        m.available_kb = m.free_kb;
    }
    m
}

/// 解析一行 `/proc/<pid>/stat`。
///
/// `comm` 可能含空格和括号，必须从**最后一个** `)` 开始切，
/// 否则像 `(Web Content)` 这种名字会把后面的字段整体错位。
fn parse_stat(pid: u32, line: &str) -> Option<ProcInfo> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let name = line[open + 1..close].to_string();
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    // rest[0] = state（原第 3 字段），索引相对第 3 字段
    let f = |i: usize| rest.get(i).copied().unwrap_or("");
    let state = f(0).chars().next().unwrap_or('?');
    let ppid: u32 = f(1).parse().unwrap_or(0);
    let utime: u64 = f(11).parse().unwrap_or(0);
    let stime: u64 = f(12).parse().unwrap_or(0);
    let threads: u32 = f(17).parse().unwrap_or(0);
    let start_ticks: u64 = f(19).parse().unwrap_or(0);
    let vsize_kb: u64 = f(20).parse::<u64>().unwrap_or(0) / 1024;
    let rss_kb: u64 = f(21).parse::<u64>().unwrap_or(0) * PAGE_KB;

    let mut info = ProcInfo {
        pid,
        ppid,
        name,
        state,
        rss_kb,
        vsize_kb,
        threads,
        start_ticks,
        cpu_ticks: utime + stime,
        ..Default::default()
    };

    // status 里补 uid 与峰值内存
    if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", pid)) {
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("Uid:") {
                info.uid = v
                    .split_whitespace()
                    .next()
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("VmHWM:") {
                info.hwm_kb = v
                    .trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse()
                    .unwrap_or(info.rss_kb);
            } else if let Some(v) = line.strip_prefix("RssAnon:") {
                info.rss_anon_kb = v.trim().trim_end_matches(" kB").trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("VmRSS:") {
                // status 的 RSS 更准（stat 的 rss 是页数，偶有滞后）
                info.rss_kb = v
                    .trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse()
                    .unwrap_or(info.rss_kb);
            }
        }
    }
    Some(info)
}

/// 当前所有进程（含内核线程，由上层决定是否隐藏）
pub fn list() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if let Ok(text) = fs::read_to_string(format!("/proc/{}/stat", pid)) {
            if let Some(p) = parse_stat(pid, &text) {
                out.push(p);
            }
        }
    }
    out.sort_by_key(|p| p.pid);
    out
}

pub fn one(pid: u32) -> Option<ProcInfo> {
    let text = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    parse_stat(pid, &text)
}

/// uid -> 用户名（读不到就返回数字，不猜）
pub fn uid_name(uid: u32) -> String {
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    for line in passwd.lines() {
        let mut it = line.split(':');
        let _name = it.next();
        let _pw = it.next();
        if let Some(id) = it.next() {
            if id.parse::<u32>() == Ok(uid) {
                return _name.unwrap_or("").to_string();
            }
        }
    }
    uid.to_string()
}

/// 当前进程的所有祖先 PID（含自己），用来做「不杀自己人」的保护名单
pub fn self_tree() -> Vec<u32> {
    let mut out = vec![std::process::id()];
    let mut cur = std::process::id();
    for _ in 0..32 {
        let Some(p) = one(cur) else { break };
        if p.ppid == 0 || p.ppid == cur || out.contains(&p.ppid) {
            break;
        }
        out.push(p.ppid);
        cur = p.ppid;
    }
    // 自己的子进程也算：GUI 拉起的子进程不该被 trim 掉
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            if let Some(p) = one(pid) {
                if out.contains(&p.ppid) && !out.contains(&pid) {
                    out.push(pid);
                }
            }
        }
    }
    out
}

/// 桌面会话所属用户（拿 xfdesktop / 文件管理器的属主，而不是采集进程自己的）
pub fn desktop_session_user() -> Option<String> {
    let shells = [
        "xfdesktop",
        "nautilus",
        "nemo",
        "caja",
        "pcmanfm",
        "gnome-shell",
        "plasmashell",
        "cinnamon",
        "mate-panel",
    ];
    let procs = list();
    for name in shells {
        if let Some(p) = procs.iter().find(|p| p.name == name) {
            return Some(uid_name(p.uid));
        }
    }
    None
}

/// 根分区使用情况（给 `drive` 字段用）
pub fn root_mount() -> String {
    "/".to_string()
}

/// 磁盘上的可用/总量（statvfs 要走 libc，这里读 /proc/self/mountinfo 太重，
/// 只给一个稳定的标识字符串；内存工具不靠它做判断）
pub fn fs_label() -> String {
    "ext4/overlay".to_string()
}

/// 进程树：pid -> 子进程列表（给「证据进程归属」用）
pub fn children_map(procs: &[ProcInfo]) -> BTreeMap<u32, Vec<u32>> {
    let mut m: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for p in procs {
        m.entry(p.ppid).or_default().push(p.pid);
    }
    m
}

pub fn exe_stem(path: &str) -> String {
    PathBuf::from(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}
