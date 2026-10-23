//! 通过 shell 自己的 IFolderView 设置图标位置
//!
//! 为什么不直接写 explorer 内存：
//!   1) 只改内存时 shell 的文件夹视图完全不知情，下一次重绘就对不上；
//!   2) Win10 1809+ shell 用注册表 `Bags\Desktop\IconLayouts` 保存位置，
//!      内存改写写不进这个值。
//!
//! **「按 F5 图标被打回原位」的真正开关是视图标志 FWF_NOENUMREFRESH（0x00080000）。**
//! 用 `IFolderView2::SetCurrentFolderFlags` 打开它之后，刷新不再重新枚举图标，
//! 位置原地不动；关掉它，同一份布局按一次 F5 就有 63～66 个图标被重排。
//! 2026-10-23 用 SendInput 发真实 F5 做过双向 A/B，唯一变量就是这个标志位。
//!
//! 它只管「刷新」。重启资源管理器后 shell 仍会按自己的记录摆一部分（实测固定 39 项），
//! 所以登录后自动恢复一次是必要的补充，见 `task::LAYOUT_TASK`。
//!
//! 曾经误判：以为「先 SelectItem 选中再定位」才是关键——其实关掉选中步骤后
//! F5 依然 0 差异，那个结论来自一次没真正触发的 PostMessage F5。
//! 也排除过：`SaveViewState`、`SHChangeNotify`、逐个 pidl、清 IconLayouts 缓存、
//! 关网格对齐、删排序方式，全部无效。
//!
//! 链路：IShellWindows → FindWindowSW(SWC_DESKTOP) → IServiceProvider
//!       → QueryService(SID_STopLevelBrowser) → IShellBrowser
//!       → QueryActiveShellView → IShellView → QueryInterface(IFolderView)

use crate::sys::*;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;

pub struct ShellItem {
    pub name: String,
    pub pidl: *mut ITEMIDLIST,
    pub x: i32,
    pub y: i32,
}

pub struct ShellView {
    fv: *mut c_void,
    /// IFolderView2：读写「自动排列 / 对齐网格」这类视图标志，拿不到就是空
    fv2: *mut c_void,
    desk: *mut c_void,
    items: Vec<ShellItem>,
}

impl ShellView {
    /// 拿不到桌面视图时返回 None（例如非交互会话），调用方回退到内存方式
    pub fn try_open() -> Option<Self> {
        unsafe {
            let sw = {
                let mut p: *mut c_void = null_mut();
                let hr = CoCreateInstance(
                    &CLSID_SHELLWINDOWS,
                    std::ptr::null(),
                    CLSCTX_ALL,
                    &IID_ISHELLWINDOWS,
                    &mut p,
                );
                if hr != S_OK || p.is_null() {
                    return None;
                }
                p
            };

            let varloc = VARIANT::from_i32(0); // CSIDL_DESKTOP
            let varroot: VARIANT = std::mem::zeroed();
            let mut hwnd: i32 = 0;
            let mut disp: *mut c_void = null_mut();
            let hr = (vtable::<IShellWindowsVtbl>(sw).find_window_sw)(
                sw,
                &varloc,
                &varroot,
                SWC_DESKTOP,
                &mut hwnd,
                SWFO_NEEDDISPATCH,
                &mut disp,
            );
            release(sw);
            if hr != S_OK || disp.is_null() {
                return None;
            }

            let sp = {
                let mut p: *mut c_void = null_mut();
                let hr =
                    (vtable::<IUnknownVtbl>(disp).query_interface)(disp, &IID_ISERVICEPROVIDER, &mut p);
                release(disp);
                if hr != S_OK || p.is_null() {
                    return None;
                }
                p
            };

            let browser = {
                let mut p: *mut c_void = null_mut();
                let hr = (vtable::<IServiceProviderVtbl>(sp).query_service)(
                    sp,
                    &SID_STOPLEVELBROWSER,
                    &IID_ISHELLBROWSER,
                    &mut p,
                );
                release(sp);
                if hr != S_OK || p.is_null() {
                    return None;
                }
                p
            };

            let view = {
                let mut p: *mut c_void = null_mut();
                let hr = (vtable::<IShellBrowserVtbl>(browser).query_active_shell_view)(browser, &mut p);
                release(browser);
                if hr != S_OK || p.is_null() {
                    return None;
                }
                p
            };

            let fv = {
                let mut p: *mut c_void = null_mut();
                let hr = (vtable::<IUnknownVtbl>(view).query_interface)(view, &IID_IFOLDERVIEW, &mut p);
                if hr != S_OK || p.is_null() {
                    release(view);
                    return None;
                }
                p
            };

            // 顺手拿一份 IFolderView2，失败不影响主流程（老系统可能没有）
            let fv2 = {
                let mut p: *mut c_void = null_mut();
                let hr = (vtable::<IUnknownVtbl>(view).query_interface)(view, &IID_IFOLDERVIEW2, &mut p);
                if hr != S_OK || p.is_null() { null_mut() } else { p }
            };
            release(view);

            let mut desk: *mut c_void = null_mut();
            if SHGetDesktopFolder(&mut desk) != S_OK || desk.is_null() {
                release(fv);
                return None;
            }

            let mut sv = ShellView {
                fv,
                fv2,
                desk,
                items: Vec::new(),
            };
            if sv.reload().is_err() || sv.items.is_empty() {
                return None;
            }
            Some(sv)
        }
    }

    /// shell 视图自己认为的「自动排列」状态（S_OK = 开着）
    pub fn auto_arrange_state(&self) -> Option<bool> {
        let hr = unsafe { (vtable::<IFolderViewVtbl>(self.fv).get_auto_arrange)(self.fv) };
        Some(hr == S_OK)
    }

    /// 当前视图模式（低 4 位是 ICON/DETAILS…，高位是 FVM_AUTOARRANGE 等标志）
    pub fn current_view_mode(&self) -> Option<u32> {
        let mut mode: u32 = 0;
        let hr = unsafe {
            (vtable::<IFolderViewVtbl>(self.fv).get_current_view_mode)(self.fv, &mut mode)
        };
        if hr == S_OK { Some(mode) } else { None }
    }

    /// 读 shell 视图的文件夹标志（与 Bags 里的 FFlags 同一套位）。
    /// 拿不到 IFolderView2 时返回 None。
    pub fn folder_flags(&self) -> Option<u32> {
        if self.fv2.is_null() {
            return None;
        }
        let mut flags: u32 = 0;
        let hr = unsafe {
            (vtable::<IFolderView2Vtbl>(self.fv2).get_current_folder_flags)(self.fv2, &mut flags)
        };
        if hr == S_OK {
            Some(flags)
        } else {
            None
        }
    }

    /// 改 shell 视图的文件夹标志：mask 里为 1 的位按 flags 覆盖
    pub fn set_folder_flags(&self, mask: u32, flags: u32) -> bool {
        if self.fv2.is_null() {
            return false;
        }
        let hr = unsafe {
            (vtable::<IFolderView2Vtbl>(self.fv2).set_current_folder_flags)(self.fv2, mask, flags)
        };
        hr == S_OK
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[ShellItem] {
        &self.items
    }

    /// 名字取自 shell 的显示名，与布局里保存的完全一致
    pub fn reload(&mut self) -> Result<()> {
        unsafe {
            self.free_items();
            let mut n: i32 = 0;
            let hr = (vtable::<IFolderViewVtbl>(self.fv).item_count)(self.fv, 0, &mut n);
            if hr != S_OK {
                bail!("枚举桌面条目失败（0x{:08X}）", hr);
            }

            let mut strret = STRRET::default();
            let fv = vtable::<IFolderViewVtbl>(self.fv);

            for i in 0..n {
                let mut pidl: *mut ITEMIDLIST = null_mut();
                if (fv.item)(self.fv, i, &mut pidl) != S_OK || pidl.is_null() {
                    continue;
                }
                let mut pt = POINT::default();
                if (fv.get_item_position)(self.fv, pidl, &mut pt) != S_OK {
                    CoTaskMemFree(pidl as *const c_void);
                    continue;
                }
                let name = self
                    .display_name(pidl, &mut strret)
                    .unwrap_or_else(|| format!("#{}", i));
                self.items.push(ShellItem {
                    name,
                    pidl,
                    x: pt.x,
                    y: pt.y,
                });
            }
            Ok(())
        }
    }

    unsafe fn display_name(&self, pidl: *const ITEMIDLIST, strret: &mut STRRET) -> Option<String> {
        *strret = STRRET::default();
        let hr = (vtable::<IShellFolderVtbl>(self.desk).get_display_name_of)(
            self.desk,
            pidl,
            SHGDN_INFOLDER,
            strret,
        );
        if hr != S_OK {
            return None;
        }
        let mut buf = [0u16; 300];
        let hr2 = StrRetToBufW(strret, pidl, buf.as_mut_ptr(), buf.len() as u32);
        // pOleStr 是 shell 分配的，取完要还回去
        if strret.u_type == 0 {
            let p = *(strret.data.as_ptr() as *const *const c_void);
            if !p.is_null() {
                CoTaskMemFree(p);
            }
        }
        if hr2 != S_OK {
            return None;
        }
        Some(from_wide(&buf))
    }

    /// 批量设置位置。返回 (移动数, 最终仍不一致的图标名, 轮数)。
    /// shell 的网格吸附是异步的，写完必须读回核对，不一致就再来一轮。
    pub fn apply(
        &mut self,
        want: &HashMap<String, (i32, i32)>,
        max_rounds: usize,
    ) -> Result<(usize, Vec<String>, usize)> {
        let mut moved = 0usize;
        let mut rounds = 0usize;

        for round in 0..max_rounds {
            self.reload()?;

            // 先把要动的图标选中，贴近「用户框选后拖动」的交互路径。
            // 注意：对「刷新是否重排」没有决定性作用（那是 FWF_NOENUMREFRESH 管的），
            // 保留它只是为了在其它 Windows 版本上更接近真实交互。
            if round == 0 && std::env::var("MEMBASE_NO_SELECT").is_err() {
                let dbg = std::env::var("MEMBASE_DEBUG").is_ok();
                let (mut ok, mut fail) = (0usize, 0usize);
                for (idx, it) in self.items.iter().enumerate() {
                    if want.contains_key(&it.name.to_lowercase()) {
                        unsafe {
                            let hr = (vtable::<IFolderViewVtbl>(self.fv).select_item)(
                                self.fv,
                                idx as i32,
                                SVSI_SELECT,
                            );
                            if hr == S_OK {
                                ok += 1;
                            } else {
                                fail += 1;
                                if dbg {
                                    eprintln!("[dbg] select {idx} {} hr=0x{hr:08X}", it.name);
                                }
                            }
                        }
                    }
                }
                if dbg {
                    eprintln!("[dbg] 选中成功 {ok} 失败 {fail}");
                }
            }

            let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
            let mut pts: Vec<POINT> = Vec::new();
            for it in &self.items {
                let Some(&(x, y)) = want.get(&it.name.to_lowercase()) else {
                    continue;
                };
                if it.x == x && it.y == y {
                    continue;
                }
                pidls.push(it.pidl as *const ITEMIDLIST);
                pts.push(POINT::new(x, y));
            }
            if pidls.is_empty() {
                break;
            }

            rounds += 1;
            if round == 0 {
                moved = pidls.len();
            }
            let hr = unsafe {
                (vtable::<IFolderViewVtbl>(self.fv).select_and_position_items)(
                    self.fv,
                    pidls.len() as u32,
                    pidls.as_ptr(),
                    pts.as_ptr(),
                    SVSI_POSITIONITEM,
                )
            };
            if std::env::var("MEMBASE_DEBUG").is_ok() {
                eprintln!("[dbg] position {} 项 hr=0x{:08X}", pidls.len(), hr);
            }
            if hr != S_OK {
                bail!("设置图标位置失败（0x{:08X}）", hr);
            }
            std::thread::sleep(std::time::Duration::from_millis(160));
        }

        // 摆完取消选中，别把桌面留成一片蓝
        if std::env::var("MEMBASE_DEBUG_HOLD").is_ok() {
            eprintln!("[dbg] 保持选中 6 秒，等待外部观察");
            std::thread::sleep(std::time::Duration::from_secs(6));
        }
        unsafe {
            let _ = (vtable::<IFolderViewVtbl>(self.fv).select_item)(
                self.fv,
                -1,
                SVSI_DESELECTOTHERS,
            );
        }

        self.reload()?;
        let mut failed = Vec::new();
        for it in &self.items {
            let Some(&(x, y)) = want.get(&it.name.to_lowercase()) else {
                continue;
            };
            if it.x != x || it.y != y {
                failed.push(it.name.clone());
            }
        }
        Ok((moved, failed, rounds.max(1)))
    }

    fn free_items(&mut self) {
        for it in self.items.drain(..) {
            if !it.pidl.is_null() {
                unsafe { CoTaskMemFree(it.pidl as *const c_void) };
            }
        }
    }
}

impl Drop for ShellView {
    fn drop(&mut self) {
        unsafe {
            self.free_items();
            if !self.desk.is_null() {
                release(self.desk);
                self.desk = null_mut();
            }
            if !self.fv2.is_null() {
                release(self.fv2);
                self.fv2 = null_mut();
            }
            if !self.fv.is_null() {
                release(self.fv);
                self.fv = null_mut();
            }
        }
    }
}
