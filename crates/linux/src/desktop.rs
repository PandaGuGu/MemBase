//! 桌面图标布局（Linux / Xfce）。
//!
//! Xfce 的桌面图标由 `xfdesktop` 绘制，位置存在
//! `~/.config/xfce4/desktop/icons.screen0-<宽>x<高>.rc`：每个图标一段，
//! 记的是**网格行列**（row/col），不是像素。这里把行列折算成像素坐标，
//! 和 Windows 的布局模型（`x`/`y`）保持一致；恢复时再折回去。
//!
//! 网格尺寸按 xfconf 的 `/desktop-icons/icon-size` 推算。折算只在
//! 「保存 → 恢复」这一对里自洽，不追求和 xfdesktop 的像素完全一致。

use anyhow::{anyhow, Result};
use membase_core::layout::IconPos;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const MARGIN: i32 = 8;
/// 图标下方的文字区
const LABEL_H: i32 = 24;

fn desktop_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    // XDG_DESKTOP_DIR 优先
    if let Ok(text) = fs::read_to_string(format!("{home}/.config/user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("XDG_DESKTOP_DIR=") {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return PathBuf::from(v.replace("$HOME", &home));
                }
            }
        }
    }
    PathBuf::from(format!("{home}/桌面"))
        .exists()
        .then(|| PathBuf::from(format!("{home}/桌面")))
        .unwrap_or_else(|| PathBuf::from(format!("{home}/Desktop")))
}

fn xfce_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/xfce4/desktop")
}

fn icon_size() -> i32 {
    std::process::Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-p", "/desktop-icons/icon-size"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<i32>()
                .ok()
        })
        .filter(|v| *v > 0)
        .unwrap_or(48)
}

fn cell() -> (i32, i32) {
    let s = icon_size();
    (s + 64, s + LABEL_H + 8)
}

fn screen_size() -> (i32, i32) {
    crate::sys::screen_size()
}

/// 当前分辨率对应的 rc 文件；优先 symlink，其次按尺寸找，最后找任意一个
fn rc_path() -> Option<PathBuf> {
    let dir = xfce_dir();
    let latest = dir.join("icons.screen.latest.rc");
    if latest.exists() {
        return Some(latest);
    }
    let (w, h) = screen_size();
    let exact = dir.join(format!("icons.screen0-{}x{}.rc", w, h));
    if exact.exists() {
        return Some(exact);
    }
    let rd = fs::read_dir(&dir).ok()?;
    let mut best: Option<PathBuf> = None;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("icons.screen") && name.ends_with(".rc") {
            best = Some(e.path());
        }
    }
    best
}

fn read_entries() -> Result<Vec<(String, i32, i32)>> {
    let path = rc_path()
        .ok_or_else(|| anyhow!("找不到 xfdesktop 的图标布局文件（~/.config/xfce4/desktop）"))?;
    let text = fs::read_to_string(&path)?;
    let mut out: Vec<(String, i32, i32)> = Vec::new();
    let mut cur: Option<String> = None;
    let mut row = 0;
    let mut col = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(name) = cur.take() {
                out.push((name, row, col));
            }
            let sec = &line[1..line.len() - 1];
            cur = if sec.starts_with('/') {
                Some(sec.to_string())
            } else {
                None
            };
            row = 0;
            col = 0;
            continue;
        }
        if cur.is_none() {
            continue;
        }
        if let Some(v) = line.strip_prefix("row=") {
            row = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("col=") {
            col = v.trim().parse().unwrap_or(0);
        }
    }
    if let Some(name) = cur {
        out.push((name, row, col));
    }
    Ok(out)
}

fn display_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn write_entries(entries: &[(String, i32, i32)]) -> Result<()> {
    let path = rc_path().ok_or_else(|| anyhow!("找不到 xfdesktop 的图标布局文件"))?;
    let mut text = String::from("[xfdesktop-version-4.10.3+-rcfile_format]\n4.10.3+=true\n");
    for (name, row, col) in entries {
        text.push_str(&format!("\n[{}]\nrow={}\ncol={}\n", name, row, col));
    }
    fs::write(&path, text)?;
    // 让 xfdesktop 重新读一次；失败也不算错（文件已经是新内容）
    let _ = std::process::Command::new("xfdesktop")
        .arg("--reload")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    Ok(())
}

fn desktop_running() -> bool {
    crate::procfs::list().iter().any(|p| p.name == "xfdesktop")
}

pub struct Desktop {
    entries: Vec<(String, i32, i32)>,
    desktop: PathBuf,
    pid: u32,
    hwnd: isize,
}

impl Desktop {
    pub fn open() -> Result<Self> {
        if !desktop_running() {
            return Err(anyhow!(
                "当前会话没有运行 xfdesktop；Linux 的桌面图标布局暂时只支持 Xfce"
            ));
        }
        let entries = read_entries()?;
        let pid = crate::procfs::list()
            .into_iter()
            .find(|p| p.name == "xfdesktop")
            .map(|p| p.pid)
            .unwrap_or(0);
        Ok(Desktop {
            entries,
            desktop: desktop_dir(),
            pid,
            hwnd: 0,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub fn count(&self) -> Result<i32> {
        Ok(self.entries.len() as i32)
    }

    /// Xfce 的自动排列由 xfdesktop 自己的设置控制；这里不假装能改
    pub fn auto_arrange(&self) -> bool {
        false
    }

    pub fn disable_auto_arrange(&self) -> bool {
        false
    }

    pub fn item_name(&self, index: i32) -> Result<String> {
        let e = self
            .entries
            .get(index as usize)
            .ok_or_else(|| anyhow!("图标索引越界: {}", index))?;
        Ok(display_name(&e.0))
    }

    pub fn item_pos(&self, index: i32) -> Result<(i32, i32)> {
        let e = self
            .entries
            .get(index as usize)
            .ok_or_else(|| anyhow!("图标索引越界: {}", index))?;
        let (cw, ch) = cell();
        Ok((MARGIN + e.2 * cw, MARGIN + e.1 * ch))
    }

    pub fn set_pos(&self, index: i32, x: i32, y: i32) -> Result<()> {
        let mut entries = self.entries.clone();
        let (cw, ch) = cell();
        let e = entries
            .get_mut(index as usize)
            .ok_or_else(|| anyhow!("图标索引越界: {}", index))?;
        e.2 = ((x - MARGIN).max(0) + cw / 2) / cw;
        e.1 = ((y - MARGIN).max(0) + ch / 2) / ch;
        write_entries(&entries)
    }

    pub fn snapshot(&self) -> Result<Vec<IconPos>> {
        let (cw, ch) = cell();
        Ok(self
            .entries
            .iter()
            .map(|(path, row, col)| IconPos {
                name: display_name(path),
                x: MARGIN + col * cw,
                y: MARGIN + row * ch,
            })
            .collect())
    }

    pub fn index_map(&self) -> Result<HashMap<String, i32>> {
        let mut m = HashMap::new();
        for (i, (path, _, _)) in self.entries.iter().enumerate() {
            m.insert(display_name(path).to_lowercase(), i as i32);
        }
        Ok(m)
    }

    /// 桌面目录（给 GUI 画预览用）
    pub fn desktop_path(&self) -> &std::path::Path {
        &self.desktop
    }
}
