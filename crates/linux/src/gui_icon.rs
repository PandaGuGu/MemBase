//! 桌面图标与壁纸（Linux）。
//!
//! 图标来源：
//!   1. 桌面上的 `.desktop` 启动器里的 `Icon=` 名字 → 在 XDG 图标主题里找 PNG；
//!   2. 桌面上的图片文件直接解码；
//!   3. 其余回退到 `color_of` 的纯色块（GUI 自己画）。
//!
//! 拿不到图标不影响可用性——Windows 版也是同样的回退策略。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

pub struct IconProvider {
    bare: Option<HashMap<String, PathBuf>>,
    cache: HashMap<String, Option<Rgba>>,
}

impl Default for IconProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl IconProvider {
    pub fn new() -> Self {
        IconProvider {
            bare: None,
            cache: HashMap::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.bare = None;
        self.cache.clear();
    }

    pub fn get(&mut self, name: &str) -> Option<Rgba> {
        if name.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.get(name) {
            return hit.clone();
        }
        let mut img = None;
        for dir in desktop_dirs() {
            let p = dir.join(name);
            if p.exists() {
                img = load_path(&p);
                break;
            }
        }
        if img.is_none() {
            if let Some(p) = self.bare_index().get(&name.to_lowercase()).cloned() {
                img = load_path(&p);
            }
        }
        self.cache.insert(name.to_string(), img.clone());
        img
    }

    fn bare_index(&mut self) -> &HashMap<String, PathBuf> {
        if self.bare.is_none() {
            let mut map = HashMap::new();
            for dir in desktop_dirs() {
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for e in rd.flatten() {
                    let path = e.path();
                    if let Some(stem) = path.file_stem() {
                        map.entry(stem.to_string_lossy().to_lowercase())
                            .or_insert_with(|| path.clone());
                    }
                    if let Some(fname) = path.file_name() {
                        map.entry(fname.to_string_lossy().to_lowercase())
                            .or_insert_with(|| path.clone());
                    }
                }
            }
            self.bare = Some(map);
        }
        self.bare.as_ref().unwrap()
    }
}

pub fn desktop_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut v = Vec::new();
    if let Ok(text) = std::fs::read_to_string(format!("{home}/.config/user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(val) = line.strip_prefix("XDG_DESKTOP_DIR=") {
                let p = val.trim().trim_matches('"').replace("$HOME", &home);
                if !p.is_empty() {
                    v.push(PathBuf::from(p));
                }
            }
        }
    }
    v.push(PathBuf::from(&home).join("Desktop"));
    v.push(PathBuf::from(&home).join("桌面"));
    v.retain(|p| p.is_dir());
    v.dedup();
    v
}

#[cfg(feature = "preview")]
fn desktop_icon_name(path: &Path) -> Option<String> {
    if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("Icon="))
        .map(|v| v.trim().to_string())
}

#[cfg(feature = "preview")]
fn icon_theme_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = vec![
        PathBuf::from(&home).join(".local/share/icons"),
        PathBuf::from(&home).join(".icons"),
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/usr/share/pixmaps"),
    ];
    dirs.retain(|p| p.is_dir());
    dirs
}

#[cfg(feature = "preview")]
fn find_theme_icon(name: &str) -> Option<PathBuf> {
    if Path::new(name).is_absolute() && Path::new(name).exists() {
        return Some(PathBuf::from(name));
    }
    let candidates = ["png", "jpg", "jpeg", "bmp"];
    for base in icon_theme_dirs() {
        // 常见的两档尺寸，找得到就先用
        for size in [
            "48x48/apps",
            "64x64/apps",
            "32x32/apps",
            "scalable/apps",
            "apps",
        ] {
            for ext in candidates {
                let p = base.join(size).join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for theme in rd.flatten() {
            let tdir = theme.path();
            if !tdir.is_dir() {
                continue;
            }
            let Ok(sub) = std::fs::read_dir(&tdir) else {
                continue;
            };
            for s in sub.flatten() {
                let sp = s.path();
                for ext in candidates {
                    let p = sp.join("apps").join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

#[cfg(feature = "preview")]
fn load_path(path: &Path) -> Option<Rgba> {
    if let Some(icon) = desktop_icon_name(path) {
        if let Some(p) = find_theme_icon(&icon) {
            return decode(&p, 96);
        }
    }
    decode(path, 96)
}

#[cfg(not(feature = "preview"))]
fn load_path(_path: &Path) -> Option<Rgba> {
    None
}

#[cfg(feature = "preview")]
fn decode(path: &Path, max: u32) -> Option<Rgba> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max {
        img.thumbnail(max, u32::MAX)
    } else {
        img
    };
    let img = img.to_rgba8();
    Some(Rgba {
        w: img.width(),
        h: img.height(),
        data: img.into_raw(),
    })
}

/// 当前桌面壁纸路径（Xfce / GNOME / KDE 各取一次）
pub fn wallpaper_path() -> Option<PathBuf> {
    if std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.to_ascii_lowercase().contains("xfce"))
        .unwrap_or(false)
    {
        if let Some(p) = xfce_wallpaper() {
            return Some(p);
        }
    }
    if let Ok(out) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", "picture-uri"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_string();
        let p = PathBuf::from(s.trim_start_matches("file://"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn xfce_wallpaper() -> Option<PathBuf> {
    let out = Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-l"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for prop in text.lines().filter(|l| l.ends_with("/last-image")) {
        if let Ok(v) = Command::new("xfconf-query")
            .args(["-c", "xfce4-desktop", "-p", prop])
            .output()
        {
            let s = String::from_utf8_lossy(&v.stdout).trim().to_string();
            let p = PathBuf::from(&s);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 读壁纸并缩到 max_w 宽（预览用）
#[cfg(feature = "preview")]
pub fn wallpaper_rgba(max_w: u32) -> Option<Rgba> {
    let path = wallpaper_path()?;
    decode(&path, max_w)
}

#[cfg(not(feature = "preview"))]
pub fn wallpaper_rgba(_max_w: u32) -> Option<Rgba> {
    None
}

/// 找不到图标时的兜底色（按名字散列到固定色相）
pub fn color_of(name: &str) -> (u8, u8, u8) {
    let mut h: i64 = 0;
    for c in name.chars() {
        h = (h * 31 + c as i64) & 0x7FFF_FFFF;
    }
    let hue = (h % 360) as f32;
    hsv_to_rgb(hue, 0.55, 0.62)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
