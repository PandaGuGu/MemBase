//! Linux 侧的进程 / 服务 / 系统信息小工具。
//!
//! 这一层的函数名刻意与 `membase_win::win` 对齐，CLI 与 GUI 用同一个
//! 别名 `plat` 引用平台层，不需要为两个系统各写一份界面逻辑。
//! 语义按 Linux 走：服务 = systemd unit，终止 = 信号，管理员 = uid 0。

use crate::procfs;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

#[allow(non_snake_case)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}

pub const TH32CS_SNAPPROCESS: u32 = 0;
pub const PROCESS_TERMINATE: u32 = 0;

pub fn memory_status() -> MEMORYSTATUSEX {
    let m = procfs::meminfo();
    let total = m.total_kb * 1024;
    let avail = m.available_kb * 1024;
    let used = total.saturating_sub(avail);
    let load = if total > 0 {
        ((used as f64 / total as f64) * 100.0).round() as u32
    } else {
        0
    };
    let swap_total = m.swap_total_kb * 1024;
    let swap_avail = m.swap_free_kb * 1024;
    MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        dwMemoryLoad: load.min(100),
        ullTotalPhys: total,
        ullAvailPhys: avail,
        ullTotalPageFile: total + swap_total,
        ullAvailPageFile: avail + swap_avail,
        ullTotalVirtual: total + swap_total,
        ullAvailVirtual: avail + swap_avail,
        ullAvailExtendedVirtual: 0,
    }
}

pub fn uptime_secs() -> u64 {
    procfs::uptime_secs()
}

pub fn sleep_ms(ms: u32) {
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
}

/// 短暂格式化：Linux 的 `created` 是开机以来的时钟滴答，不是 FILETIME。
/// 这里按「开机第 N 秒」显示，界面/命令行都不会看到 1970 年。
pub fn filetime_local_string(v: u64) -> String {
    if v == 0 {
        return String::new();
    }
    let secs = v / procfs::CLK_TCK as u64;
    format!("开机第 {} 秒", secs)
}

pub fn now_local_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn now_utc_string() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn boot_time_string(uptime_s: u64) -> String {
    let boot = chrono::Local::now().timestamp() as u64 - uptime_s;
    chrono::DateTime::from_timestamp(boot as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn computer_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| procfs::uid_name(current_uid()))
}

pub fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn logical_processors() -> u32 {
    std::thread::available_parallelism()
        .map(|v| v.get() as u32)
        .unwrap_or(1)
}

pub fn system_drive() -> String {
    "/".to_string()
}

pub fn is_admin() -> bool {
    current_uid() == 0
}

pub fn is_elevated() -> bool {
    is_admin()
}

pub fn console_utf8() {}

pub fn attach_parent_console() {}

pub fn open_folder(path: &str) {
    let opener = if std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("xfce"))
        .unwrap_or(false)
    {
        "thunar"
    } else {
        "xdg-open"
    };
    let _ = Command::new(opener)
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn process_ws_mb(pid: u32) -> Option<f64> {
    let p = procfs::one(pid)?;
    Some((p.rss_kb as f64 / 1024.0 * 100.0).round() / 100.0)
}

/// 进程身份：开机以来的时钟滴答（与 PID 一起防止复用）
pub fn process_create_time(pid: u32) -> Option<u64> {
    procfs::one(pid).map(|p| p.start_ticks)
}

pub fn process_alive_as(pid: u32, created: u64) -> bool {
    match procfs::one(pid) {
        Some(p) => (created == 0 || p.start_ticks == created) && p.state != 'Z' && p.state != 'X',
        None => false,
    }
}

fn signal(pid: u32, sig: i32) -> Result<(), String> {
    if pid <= 1 || pid == std::process::id() {
        return Err("拒绝向 init / 自身发信号".into());
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

pub fn terminate_process(pid: u32) -> Result<(), String> {
    signal(pid, libc::SIGKILL)
}

pub fn terminate_process_checked(pid: u32, created: u64) -> Result<(), String> {
    if !process_alive_as(pid, created) {
        return Err("进程已不存在或 PID 已被复用".into());
    }
    signal(pid, libc::SIGKILL)
}

/// 「先礼」：给进程发 SIGTERM，等价 Windows 的 WM_CLOSE。
/// 返回值是「确实发到了」的 PID 计数（0 = 没有可关的窗口/进程）。
pub fn request_close_for(pids: &[u32]) -> HashMap<u32, usize> {
    let mut out = HashMap::new();
    for &pid in pids {
        if let Some(p) = procfs::one(pid) {
            if p.state == 'Z' || p.state == 'X' {
                continue;
            }
            // 内核线程没有用户态可退出，直接跳过（走强杀也没意义）
            if p.exe_path().is_empty() {
                continue;
            }
            if signal(pid, libc::SIGTERM).is_ok() {
                out.insert(pid, 1);
            }
        }
    }
    out
}

pub fn protected_pids() -> Vec<u32> {
    procfs::self_tree()
}

pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if let Some(end) = chars[i + 2..].iter().position(|c| *c == '}') {
                    let key: String = chars[i + 2..i + 2 + end].iter().collect();
                    out.push_str(&std::env::var(&key).unwrap_or_default());
                    i += end + 3;
                    continue;
                }
            }
            let end = chars[i + 1..]
                .iter()
                .position(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map(|e| e + i + 1)
                .unwrap_or(chars.len());
            let key: String = chars[i + 1..end].iter().collect();
            if key.is_empty() {
                out.push('$');
                i += 1;
            } else {
                out.push_str(&std::env::var(&key).unwrap_or_default());
                i = end;
            }
            continue;
        }
        if chars[i] == '~' && out.is_empty() {
            if let Ok(home) = std::env::var("HOME") {
                out.push_str(&home);
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 删除到回收站：`~/.local/share/Trash/files`
pub fn recycle_path(path: &Path) -> Result<(), String> {
    let trash = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("Trash/files");
    std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| "deleted".into());
    let mut target = trash.join(&name);
    let mut n = 1;
    while target.exists() {
        target = trash.join(format!("{}.{}", name.to_string_lossy(), n));
        n += 1;
    }
    std::fs::rename(path, &target).map_err(|e| format!("移动到回收站失败: {e}"))
}

/// Linux 没有 Appx / Store 应用
pub fn remove_appx(_full_name: &str) -> Result<(), String> {
    Err("Linux 没有 Store 应用（Appx），该动作不适用".into())
}

// ---------------- 服务：systemd ----------------

#[derive(Clone, Debug, Default)]
pub struct SvcInfo {
    pub name: String,
    pub description: String,
    pub active_state: String,
    pub sub_state: String,
    pub unit_file_state: String,
    pub main_pid: u32,
    pub exec_path: String,
    pub fragment_path: String,
}

impl SvcInfo {
    /// 与 Windows 侧一致的 `Running` / `Stopped` 两种状态名
    pub fn state(&self) -> String {
        if self.active_state == "active" {
            "Running".to_string()
        } else {
            "Stopped".to_string()
        }
    }

    /// 与 Windows 侧一致的 `Auto` / `Manual` / `Disabled`
    pub fn start_mode(&self) -> String {
        match self.unit_file_state.as_str() {
            "enabled" | "enabled-runtime" | "linked" => "Auto".to_string(),
            "disabled" | "masked" | "masked-runtime" => "Disabled".to_string(),
            _ => "Manual".to_string(),
        }
    }
}

pub fn systemctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("systemctl")
        .args(args)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("调用 systemctl 失败: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() {
            format!("systemctl {:?} 失败", args)
        } else {
            msg
        })
    }
}

fn parse_show_block(block: &str) -> Option<SvcInfo> {
    let mut s = SvcInfo::default();
    for line in block.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "Id" => s.name = v.trim().to_string(),
            "Description" => s.description = v.trim().to_string(),
            "ActiveState" => s.active_state = v.trim().to_string(),
            "SubState" => s.sub_state = v.trim().to_string(),
            "UnitFileState" => s.unit_file_state = v.trim().to_string(),
            "MainPID" => s.main_pid = v.trim().parse().unwrap_or(0),
            "ExecStart" => {
                // { path=/usr/bin/foo ; argv[]=... ; ignore_errors=no ; ... }
                if let Some(i) = v.find("path=") {
                    let rest = &v[i + 5..];
                    let end = rest
                        .find(|c: char| c == ' ' || c == ';')
                        .unwrap_or(rest.len());
                    s.exec_path = rest[..end].trim().to_string();
                }
            }
            "FragmentPath" => s.fragment_path = v.trim().to_string(),
            _ => {}
        }
    }
    if s.name.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 一次 `systemctl show` 拿全部服务，避免每个 unit 调一次（100+ 次会明显变慢）
pub fn list_services() -> Vec<SvcInfo> {
    let mut out = Vec::new();
    if let Ok(text) = systemctl(&[
        "show",
        "--type=service",
        "--all",
        "--no-pager",
        "-p",
        "Id",
        "-p",
        "Description",
        "-p",
        "ActiveState",
        "-p",
        "SubState",
        "-p",
        "UnitFileState",
        "-p",
        "MainPID",
        "-p",
        "ExecStart",
        "-p",
        "FragmentPath",
    ]) {
        for block in text.split("\n\n") {
            if let Some(s) = parse_show_block(block) {
                out.push(s);
            }
        }
    }
    // 兜底：老 systemd 的 show 不接受 --type
    if out.is_empty() {
        if let Ok(text) = systemctl(&[
            "list-units",
            "--type=service",
            "--all",
            "--no-legend",
            "--no-pager",
            "--plain",
        ]) {
            for line in text.lines() {
                let mut it = line.split_whitespace();
                let Some(unit) = it.next() else { continue };
                let _load = it.next();
                let active = it.next().unwrap_or("");
                let sub = it.next().unwrap_or("");
                out.push(SvcInfo {
                    name: unit.to_string(),
                    description: it.collect::<Vec<_>>().join(" "),
                    active_state: if active == "active" {
                        "active".into()
                    } else {
                        "inactive".into()
                    },
                    sub_state: sub.to_string(),
                    ..Default::default()
                });
            }
        }
    }
    out
}

fn unit_name(name: &str) -> String {
    if name.ends_with(".service") {
        name.to_string()
    } else {
        format!("{}.service", name)
    }
}

pub fn stop_service(name: &str) -> Result<(), String> {
    systemctl(&["stop", &unit_name(name)]).map(|_| ())
}

pub fn start_service(name: &str) -> Result<(), String> {
    systemctl(&["start", &unit_name(name)]).map(|_| ())
}

pub fn disable_service_start(name: &str) -> Result<(), String> {
    systemctl(&["disable", &unit_name(name)]).map(|_| ())
}

pub fn enable_service_start(name: &str) -> Result<(), String> {
    systemctl(&["enable", &unit_name(name)]).map(|_| ())
}

pub fn service_start_mode(name: &str) -> Option<String> {
    let unit = unit_name(name);
    let out = Command::new("systemctl")
        .args(["show", "-p", "UnitFileState", "--value", &unit])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if v.is_empty() {
        return None;
    }
    Some(
        match v.as_str() {
            "enabled" | "enabled-runtime" | "linked" => "Auto",
            "disabled" | "masked" | "masked-runtime" => "Disabled",
            _ => "Manual",
        }
        .to_string(),
    )
}

pub fn service_pid(name: &str) -> Option<u32> {
    let unit = unit_name(name);
    let out = Command::new("systemctl")
        .args(["show", "-p", "MainPID", "--value", &unit])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

// ---------------- 不适用于 Linux 的 Windows 专用接口 ----------------

pub fn user_reg_dword(_path: &str, _value: &str) -> Option<u32> {
    None
}

pub fn set_user_reg_dword(_path: &str, _value: &str, _val: u32) -> Result<(), String> {
    Err("Linux 没有注册表；该功能开关不适用".into())
}

pub fn delete_user_reg_value(_path: &str, _value: &str) -> Result<(), String> {
    Err("Linux 没有注册表；该功能开关不适用".into())
}
