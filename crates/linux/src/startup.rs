//! 启动项扫描（Linux）：XDG autostart（`~/.config/autostart`、`/etc/xdg/autostart`）
//! 与 systemd 用户服务。
//!
//! 禁用方式：
//!   * .desktop：在用户目录写一份同名覆盖文件，`Hidden=true`（不删系统原文件，可逆）；
//!   * systemd 用户服务：`systemctl --user disable`（单元文件保留）。

use membase_core::startup::{StartupRaw, StartupSource};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn user_autostart() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".config"))
        .join("autostart")
}

fn system_autostart() -> PathBuf {
    PathBuf::from("/etc/xdg/autostart")
}

#[derive(Default)]
struct DesktopEntry {
    name: String,
    exec: String,
    hidden: bool,
    autostart_enabled: Option<bool>,
}

fn parse_desktop(text: &str) -> DesktopEntry {
    let mut e = DesktopEntry::default();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Name" if e.name.is_empty() => e.name = v.to_string(),
            "Exec" => e.exec = v.to_string(),
            "Hidden" => e.hidden = v.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => {
                e.autostart_enabled = Some(!v.eq_ignore_ascii_case("false"))
            }
            _ => {}
        }
    }
    e
}

/// 去掉 Exec 里的字段码（%U / %f / %F …），只留可执行的命令行
fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !t.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn scan_dir(dir: &Path, source: StartupSource, system: bool, out: &mut Vec<StartupRaw>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let fname = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !fname.to_ascii_lowercase().ends_with(".desktop") {
            continue;
        }
        let text = fs::read_to_string(&p).unwrap_or_default();
        let entry = parse_desktop(&text);
        if entry.exec.trim().is_empty() {
            continue;
        }
        // 用户目录里的同名文件覆盖系统项：已经由用户目录扫过的不重复列
        if source == StartupSource::AllUsersFolder && user_autostart().join(&fname).exists() {
            continue;
        }
        let enabled = if entry.hidden {
            Some(false)
        } else {
            entry.autostart_enabled.or(Some(true))
        };
        out.push(StartupRaw {
            source,
            name: if entry.name.is_empty() {
                fname.clone()
            } else {
                entry.name.clone()
            },
            command: clean_exec(&entry.exec),
            enabled,
            detail: p.to_string_lossy().to_string(),
            system,
        });
    }
}

fn scan_systemd_user() -> Vec<StartupRaw> {
    let out = Command::new("systemctl")
        .args([
            "--user",
            "list-unit-files",
            "--type=service",
            "--state=enabled",
            "--no-legend",
            "--no-pager",
        ])
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };
    let mut items = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(unit) = it.next() else { continue };
        if unit.ends_with(".timer") || unit.ends_with(".socket") || unit.ends_with(".path") {
            continue;
        }
        let exec = Command::new("systemctl")
            .args(["--user", "show", "-p", "ExecStart", "--value", unit])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let command = if let Some(i) = exec.find("path=") {
            let rest = &exec[i + 5..];
            let end = rest
                .find(|c: char| c == ' ' || c == ';')
                .unwrap_or(rest.len());
            rest[..end].to_string()
        } else {
            exec
        };
        if command.is_empty() {
            continue;
        }
        items.push(StartupRaw {
            source: StartupSource::ScheduledTask,
            name: unit.to_string(),
            command,
            enabled: Some(true),
            detail: "systemd --user（登录时启动）".to_string(),
            system: false,
        });
    }
    items
}

/// 扫描全部启动项（只读）
pub fn scan() -> Vec<StartupRaw> {
    let mut out = Vec::new();
    scan_dir(
        &user_autostart(),
        StartupSource::UserFolder,
        false,
        &mut out,
    );
    scan_dir(
        &system_autostart(),
        StartupSource::AllUsersFolder,
        true,
        &mut out,
    );
    out.extend(scan_systemd_user());
    out
}

/// 启用或禁用一项。禁用不删原文件：用户目录写覆盖文件 / systemd disable。
pub fn set_enabled(source: StartupSource, name: &str, enable: bool) -> Result<(), String> {
    match source {
        StartupSource::ScheduledTask => {
            let action = if enable { "enable" } else { "disable" };
            let out = Command::new("systemctl")
                .args(["--user", action, name])
                .env("LC_ALL", "C.UTF-8")
                .output()
                .map_err(|e| format!("调用 systemctl 失败: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
        _ => {
            let file = find_desktop_file(name, source)
                .ok_or_else(|| format!("找不到启动项文件：{name}"))?;
            let text = fs::read_to_string(&file).unwrap_or_default();
            let exec = parse_desktop(&text).exec;
            let dir = user_autostart();
            fs::create_dir_all(&dir).map_err(|e| format!("建目录失败: {e}"))?;
            let target = dir.join(file.file_name().unwrap_or_default());
            let content = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec={}\nHidden={}\nX-GNOME-Autostart-enabled={}\n",
                name,
                exec,
                if enable { "false" } else { "true" },
                if enable { "true" } else { "false" }
            );
            fs::write(&target, content).map_err(|e| format!("写覆盖文件失败: {e}"))
        }
    }
}

fn find_desktop_file(name: &str, source: StartupSource) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = match source {
        StartupSource::UserFolder => vec![user_autostart()],
        StartupSource::AllUsersFolder => vec![user_autostart(), system_autostart()],
        _ => vec![user_autostart(), system_autostart()],
    };
    for dir in dirs {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let text = fs::read_to_string(&p).unwrap_or_default();
            let entry = parse_desktop(&text);
            let fname = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if entry.name.eq_ignore_ascii_case(name) || fname.eq_ignore_ascii_case(name) {
                return Some(p);
            }
        }
    }
    None
}
