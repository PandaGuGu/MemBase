//! 开机采集（Linux）：systemd 用户 timer。
//!
//! Windows 用「SYSTEM 计划任务 + BootTrigger」，Linux 没有 SYSTEM 计划任务；
//! 这里装一个 **user timer**：登录后由 systemd --user 在 `OnBootSec` 到点时
//! 拉起同一个二进制执行 `collect ... --self-destruct`，采完自毁。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 默认任务名；采集完成后任务会自毁，所以常年为空是正常状态。
pub use membase_core::DEFAULT_TASK;

#[derive(Clone, Debug, Default)]
pub struct TaskInfo {
    pub task: String,
    pub installed: bool,
    pub at_sec: u64,
    pub also_sec: Option<u64>,
    pub exe: String,
}

fn unit_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        })
        .join("systemd/user")
}

fn safe_task(task: &str) -> String {
    let s: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.trim().is_empty() {
        DEFAULT_TASK.to_string()
    } else {
        s
    }
}

fn systemctl_user(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("调用 systemctl --user 失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn shell_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 注册开机采集 timer，并把 exe 复制到数据目录（路径稳定，不随原文件挪动失效）
pub fn install(
    src_exe: &Path,
    out: &Path,
    at_sec: u64,
    also_sec: Option<u64>,
    task: &str,
) -> Result<PathBuf, String> {
    let task = safe_task(task);
    if !src_exe.is_file() {
        return Err(format!("找不到可执行文件：{}", src_exe.display()));
    }
    fs::create_dir_all(out).map_err(|e| format!("建目录失败: {e}"))?;
    let target = out.join("membase");
    let same = match (src_exe.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::copy(src_exe, &target).map_err(|e| format!("复制 exe 失败: {e}"))?;
    }

    let dir = unit_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("建 systemd 用户目录失败: {e}"))?;

    let also_arg = also_sec
        .map(|s| format!(" --also {}", s))
        .unwrap_or_default();
    let exec = format!(
        "{} collect --out {} --at {}{} --self-destruct --task {} --json",
        shell_quote(&target.to_string_lossy()),
        shell_quote(&out.to_string_lossy()),
        at_sec,
        also_arg,
        task
    );
    let service = format!(
        "[Unit]\nDescription=MemBase 开机内存基准线采集\nAfter=graphical-session.target\n\n[Service]\nType=oneshot\nExecStart={exec}\nNice=5\n"
    );
    let timer = format!(
        "[Unit]\nDescription=MemBase 开机第 {at_sec} 秒采集基准线\n\n[Timer]\nOnBootSec={at_sec}s\nUnit={task}.service\n\n[Install]\nWantedBy=timers.target\n"
    );

    fs::write(dir.join(format!("{task}.service")), service)
        .map_err(|e| format!("写 service 失败: {e}"))?;
    fs::write(dir.join(format!("{task}.timer")), timer)
        .map_err(|e| format!("写 timer 失败: {e}"))?;

    systemctl_user(&["daemon-reload"])?;
    systemctl_user(&["enable", &format!("{task}.timer")])?;
    Ok(target)
}

/// 删除任务；任务本来就不存在也算成功
pub fn uninstall(task: &str) -> Result<(), String> {
    let task = safe_task(task);
    let dir = unit_dir();
    let timer = format!("{task}.timer");
    let _ = systemctl_user(&["disable", "--now", &timer]);
    let _ = systemctl_user(&["stop", &timer]);
    let _ = fs::remove_file(dir.join(&timer));
    let _ = fs::remove_file(dir.join(format!("{task}.service")));
    let _ = systemctl_user(&["daemon-reload"]);
    Ok(())
}

fn tag_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .map(|v| v.trim().to_string())
}

fn extract_flag(args: &str, flag: &str) -> Option<u64> {
    let i = args.find(flag)?;
    args[i + flag.len()..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

/// 查任务当前状态；查不到就是「未安装」，不算错误
pub fn query(task: &str) -> TaskInfo {
    let task = safe_task(task);
    let mut info = TaskInfo {
        task: task.clone(),
        ..Default::default()
    };
    let timer = unit_dir().join(format!("{task}.timer"));
    let service = unit_dir().join(format!("{task}.service"));
    if !timer.exists() {
        return info;
    }
    let enabled = Command::new("systemctl")
        .args(["--user", "is-enabled", &format!("{task}.timer")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    info.installed = enabled;
    if let Ok(text) = fs::read_to_string(service) {
        if let Some(exec) = tag_value(&text, "ExecStart") {
            info.exe = exec
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('"')
                .to_string();
            info.at_sec = extract_flag(&exec, "--at").unwrap_or(0);
            info.also_sec = extract_flag(&exec, "--also");
        }
    }
    info
}

// ---------------------------------------------------------------- 开机自动恢复桌面布局

pub const LAYOUT_TASK: &str = "MemBaseLayout";

/// 注册「登录后自动恢复布局」的 user timer；exe 会复制到数据目录
pub fn install_layout(
    src_exe: &Path,
    out: &Path,
    layout: &str,
    delay_sec: u64,
    task: &str,
) -> Result<PathBuf, String> {
    let task = safe_task(if task.trim().is_empty() {
        LAYOUT_TASK
    } else {
        task
    });
    if !src_exe.is_file() {
        return Err(format!("找不到可执行文件：{}", src_exe.display()));
    }
    if layout.trim().is_empty() {
        return Err("布局名字不能为空".into());
    }
    fs::create_dir_all(out).map_err(|e| format!("建目录失败: {e}"))?;
    let target = out.join("membase");
    let same = match (src_exe.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::copy(src_exe, &target).map_err(|e| format!("复制 exe 失败: {e}"))?;
    }
    let dir = unit_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("建 systemd 用户目录失败: {e}"))?;

    let exec = format!(
        "{} layout restore {} --execute --yes --wait 180",
        shell_quote(&target.to_string_lossy()),
        shell_quote(layout)
    );
    let service = format!(
        "[Unit]\nDescription=MemBase 登录后恢复桌面图标布局\nAfter=graphical-session.target\n\n[Service]\nType=oneshot\nExecStart={exec}\n"
    );
    let timer = format!(
        "[Unit]\nDescription=MemBase 登录后第 {delay_sec} 秒恢复布局「{layout}」\n\n[Timer]\nOnStartupSec={delay_sec}s\nUnit={task}.service\n\n[Install]\nWantedBy=timers.target\n"
    );
    fs::write(dir.join(format!("{task}.service")), service)
        .map_err(|e| format!("写 service 失败: {e}"))?;
    fs::write(dir.join(format!("{task}.timer")), timer)
        .map_err(|e| format!("写 timer 失败: {e}"))?;

    systemctl_user(&["daemon-reload"])?;
    systemctl_user(&["enable", "--now", &format!("{task}.timer")])?;
    Ok(target)
}

/// 查「登录自动恢复」timer 里记的布局名；没装返回 None
pub fn query_layout(task: &str) -> Option<String> {
    let task = safe_task(if task.trim().is_empty() {
        LAYOUT_TASK
    } else {
        task
    });
    let text = fs::read_to_string(unit_dir().join(format!("{task}.service"))).ok()?;
    let exec = tag_value(&text, "ExecStart")?;
    let i = exec.find("layout restore")?;
    let rest = exec[i + "layout restore".len()..].trim_start();
    let name = if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next()?.to_string()
    } else {
        rest.split_whitespace().next()?.to_string()
    };
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
