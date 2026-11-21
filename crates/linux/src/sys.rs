//! 与 `membase_win::sys` 对齐的最小集合：屏幕尺寸 / DPI / folder-view 常量。
//! Linux 没有 IFolderView，相关常量只为了上层代码能原样编译。

use x11rb::connection::Connection;
use x11rb::rust_connection::RustConnection;

/// 虚拟屏尺寸（像素）。拿不到 X11 时退回 1920×1080。
pub fn screen_size() -> (i32, i32) {
    if let Ok((conn, screen)) = RustConnection::connect(None) {
        if let Some(root) = conn.setup().roots.get(screen) {
            return (root.width_in_pixels as i32, root.height_in_pixels as i32);
        }
    }
    (1920, 1080)
}

/// X11 下 DPI 缩放由 egui / winit 自己处理，平台层不再二次缩放
pub fn dpi_scale() -> f32 {
    1.0
}

pub fn set_dpi_aware() {}

pub fn com_init() {}

pub const FWF_AUTOARRANGE: u32 = 0x0000_0001;
pub const FWF_SNAPTOGRID: u32 = 0x0000_0004;
pub const FWF_NOENUMREFRESH: u32 = 0x0008_0000;
