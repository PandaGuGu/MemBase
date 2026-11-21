//! 桌面 shell 视图（Linux）：没有 IShellWindows / IFolderView 等价物。
//!
//! Xfce 的图标位置直接落在 rc 文件里，`desktop.rs` 已经能读写，
//! 所以这里统一返回「不可用」，让 `actions::restore` 走文件方式。

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::ffi::c_void;

pub struct ShellItem {
    pub name: String,
    pub pidl: *mut c_void,
    pub x: i32,
    pub y: i32,
}

pub struct ShellView {
    items: Vec<ShellItem>,
}

impl ShellView {
    pub fn try_open() -> Option<Self> {
        None
    }

    pub fn auto_arrange_state(&self) -> Option<bool> {
        None
    }

    pub fn current_view_mode(&self) -> Option<u32> {
        None
    }

    pub fn folder_flags(&self) -> Option<u32> {
        None
    }

    pub fn set_folder_flags(&self, _mask: u32, _flags: u32) -> bool {
        false
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[ShellItem] {
        &self.items
    }

    pub fn reload(&mut self) -> Result<()> {
        bail!("Linux 没有 shell 视图接口")
    }

    pub fn apply(
        &mut self,
        _want: &HashMap<String, (i32, i32)>,
        _max_rounds: usize,
    ) -> Result<(usize, Vec<String>, usize)> {
        bail!("Linux 没有 shell 视图接口")
    }
}
