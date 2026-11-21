//! 平台层（Linux）：`/proc` 进程 / systemd 服务 / XDG 启动项 /
//! systemd timer 定时采集 / X11 息屏 / Xfce 桌面图标布局。
//!
//! 与 `membase-win` 保持同样的模块名和函数签名，上层的 CLI 与 GUI
//! 只需要把平台别名换一下就能在 Linux 上编译；Windows 专用的注册表、
//! Appx、shell COM 接口在这里有等价的空实现或明确报错。
#![cfg(target_os = "linux")]

pub mod actions;
pub mod close;
pub mod compare;
pub mod desktop;
pub mod display;
pub mod gui_icon;
pub mod procfs;
pub mod repair;
pub mod restart;
pub mod screen;
pub mod shellview;
pub mod snapshot;
pub mod startup;
pub mod sys;
pub mod task;
pub mod win;

/// 进程级初始化。X11 连接是惰性的，这里只有会话相关的一次性探测。
pub fn init() {
    // 让子进程（systemctl / xdg-open / xset）使用 UTF-8 输出
    std::env::set_var("LC_ALL", "C.UTF-8");
}
