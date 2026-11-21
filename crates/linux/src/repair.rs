//! 修复「每次重启固定错乱同一批图标」（Linux / Xfce）。
//!
//! Xfce 没有 `IconLayouts` 注册表值；对应物就是 xfdesktop 的
//! `icons.screen*.rc`。这份文件写坏时会出现「怎么摆都弹回去」，
//! 备份后删掉，xfdesktop 会按默认顺序重建。

use anyhow::{Context, Result};
use membase_core::layout_store as store;
use std::fs;
use std::path::PathBuf;

pub struct RepairInfo {
    pub backup: PathBuf,
    pub bytes: usize,
}

fn xfce_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/xfce4/desktop")
}

fn rc_file() -> Option<PathBuf> {
    let dir = xfce_dir();
    let latest = dir.join("icons.screen.latest.rc");
    if latest.exists() {
        return Some(latest);
    }
    let (w, h) = crate::sys::screen_size();
    let exact = dir.join(format!("icons.screen0-{}x{}.rc", w, h));
    if exact.exists() {
        return Some(exact);
    }
    let rd = fs::read_dir(&dir).ok()?;
    rd.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().starts_with("icons.screen"))
            .unwrap_or(false)
    })
}

/// 读取当前布局文件原始字节；不存在则返回 None
pub fn read_icon_layouts() -> Result<Option<Vec<u8>>> {
    match rc_file() {
        Some(p) => Ok(Some(fs::read(&p)?)),
        None => Ok(None),
    }
}

pub fn backup_path() -> PathBuf {
    store::root_dir().join("xfdesktop-icons.rc.bak")
}

/// 备份 + 删除。返回 None 表示本来就没有布局文件（已经是干净状态）
pub fn repair() -> Result<Option<RepairInfo>> {
    let Some(blob) = read_icon_layouts()? else {
        return Ok(None);
    };
    let backup = backup_path();
    let _ = fs::create_dir_all(store::root_dir());
    fs::write(&backup, &blob).with_context(|| format!("备份失败: {}", backup.display()))?;

    let path = rc_file().context("找不到 xfdesktop 布局文件")?;
    fs::remove_file(&path).with_context(|| format!("删除失败: {}", path.display()))?;
    let _ = std::process::Command::new("xfdesktop")
        .arg("--reload")
        .spawn();

    Ok(Some(RepairInfo {
        backup,
        bytes: blob.len(),
    }))
}

/// Xfce 没有 FFlags 概念
pub fn read_fflags() -> Option<u32> {
    None
}

pub fn write_fflags(_v: u32) -> Result<()> {
    Ok(())
}

/// 仅做诊断：缓存是否存在、多大
pub fn describe_cache() -> String {
    match read_icon_layouts() {
        Ok(Some(b)) => format!("{} 字节", b.len()),
        Ok(None) => "不存在（干净）".to_string(),
        Err(e) => format!("读取失败: {}", e),
    }
}
