//! 平台层：Win32 进程 / 服务 / 计划任务 + 桌面图标 ListView / shell IFolderView。
//!
//! 整个 crate 只在 Windows 上编译（非 Windows 平台是空 crate），
//! 所以上层代码用 `#[cfg(windows)]` 引用它即可。
//! 这是整个工程唯一允许出现 `unsafe` 的地方。
#![cfg(windows)]
// win.rs（进程/服务）与 sys.rs（桌面图标）各自声明了少量同名 Win32 函数，
// 句柄类型写法不同（*mut c_void vs isize）、ABI 完全一致，两边故意保持独立。
#![allow(clashing_extern_declarations)]

pub mod actions;
pub mod close;
pub mod compare;
pub mod desktop;
pub mod display;
pub mod gui_icon;
pub mod repair;
pub mod restart;
pub mod screen;
pub mod shellview;
pub mod startup;
pub mod sys;
pub mod snapshot;
pub mod task;
pub mod win;

/// 进程级初始化：DPI 感知 + COM 单线程套间。
/// 桌面布局要走 shell 的 COM 接口，两个都必须先就位，否则坐标会错、接口会拿不到。
pub fn init() {
    sys::set_dpi_aware();
    sys::com_init();
}
