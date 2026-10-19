//! 平台层：Win32 的进程 / 服务 / 计划任务封装。
//!
//! 整个 crate 只在 Windows 上编译（非 Windows 平台是空 crate），
//! 所以上层代码用 `#[cfg(windows)]` 引用它即可。
//! 这是整个工程唯一允许出现 `unsafe` 的地方。
#![cfg(windows)]

pub mod compare;
pub mod snapshot;
pub mod win;
