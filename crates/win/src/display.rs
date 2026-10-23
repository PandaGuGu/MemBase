//! 显示器指纹：桌面图标坐标只在同一套显示器配置下才有意义。
//!
//! 做法参考 Icons-Saver：记「虚拟屏边界 + 每块屏的设备名/尺寸/主屏归属/DPI」，
//! 恢复前比对一次，配置变了就明确提示，而不是把图标甩到看不见的地方。
//!
//! 设备名用 `\\.\DISPLAY1` 这种，不用序号——Windows 每次开机可能重新编号。

use crate::sys::*;
use membase_core::layout::{DisplayInfo, MonitorEntry};

struct Ctx {
    items: Vec<MonitorEntry>,
    dpi: u32,
}

extern "system" fn enum_cb(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(data as *mut Ctx) };
    let mut info = MONITORINFOEXW::default();
    info.cb_size = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(hmon, &mut info) };
    if ok != 0 {
        ctx.items.push(MonitorEntry {
            device: from_wide(&info.device),
            rect: [
                info.rc_monitor.left,
                info.rc_monitor.top,
                info.rc_monitor.right,
                info.rc_monitor.bottom,
            ],
            primary: info.flags & MONITORINFOF_PRIMARY != 0,
            dpi: ctx.dpi,
        });
    }
    1
}

/// 当前显示器配置。拿不到详细信息时退化成「只知道总分辨率」。
pub fn fingerprint() -> DisplayInfo {
    // 主屏 DPI：多屏各自不同时这里只能拿主屏的，够用来发现「缩放变了」
    let dpi = unsafe {
        let d = GetDpiForWindow(GetDesktopWindow());
        if d == 0 {
            96
        } else {
            d
        }
    };

    let mut ctx = Ctx {
        items: Vec::new(),
        dpi,
    };
    unsafe {
        EnumDisplayMonitors(
            0,
            std::ptr::null(),
            enum_cb,
            &mut ctx as *mut Ctx as LPARAM,
        );
    }

    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };

    ctx.items
        .sort_by(|a, b| a.device.to_lowercase().cmp(&b.device.to_lowercase()));

    DisplayInfo {
        virtual_screen: [x, y, x + w, y + h],
        monitors: ctx.items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 指纹至少有一块屏() {
        let d = fingerprint();
        assert!(!d.monitors.is_empty(), "EnumDisplayMonitors 没枚举到任何屏");
        assert!(d.width() > 0 && d.height() > 0);
        assert!(d.monitors.iter().any(|m| m.primary), "没有主屏标记");
    }

    #[test]
    fn 同一份配置自己等于自己() {
        let d = fingerprint();
        assert!(d.same_hardware(&d));
    }
}
