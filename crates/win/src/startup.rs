//! 启动项扫描：注册表 Run / 启动文件夹 / 非 Microsoft 计划任务。
//!
//! 三条来源各有各的「关掉」方式，不能一概而论：
//!   * Run 值：**不删值**，改 `Explorer\StartupApproved\...` 里的批准记录
//!     （这就是任务管理器那个开关；删值等于丢失原命令行，撤销不回来）；
//!   * 启动文件夹：同样走 StartupApproved 的 `StartupFolder`；
//!   * 计划任务：`schtasks /change /tn <路径> /disable`。
//!
//! 快捷方式的目标用 shell 自己的 IShellLink 解——`.lnk` 是二进制格式，
//! 自己抠字符串会在中文路径和相对路径上翻车。

use crate::win;
use deskkit_core::startup::{self as core_startup, StartupRaw, StartupSource};
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN32_KEY: &str = r"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_RUN: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APPROVED_RUN32: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32";
const APPROVED_FOLDER: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";

/// 扫描全部启动项（只读）
pub fn scan() -> Vec<StartupRaw> {
    let mut out = Vec::new();
    out.extend(scan_user_run());
    out.extend(scan_machine_run(false));
    out.extend(scan_machine_run(true));
    out.extend(scan_folders());
    out.extend(scan_tasks());
    out
}

fn scan_user_run() -> Vec<StartupRaw> {
    let approved = approved_map(APPROVED_RUN);
    win::enum_user_values(RUN_KEY)
        .into_iter()
        .map(|v| {
            let enabled = approved.get(&v.name.to_lowercase()).copied();
            StartupRaw {
                source: StartupSource::UserRun,
                name: v.name.clone(),
                command: win::expand_env(&v.as_string()),
                enabled,
                detail: format!(r"HKCU\{}", RUN_KEY),
                system: false,
            }
        })
        .collect()
}

fn scan_machine_run(view32: bool) -> Vec<StartupRaw> {
    let (key, approved_key, source) = if view32 {
        (RUN32_KEY, APPROVED_RUN32, StartupSource::MachineRun32)
    } else {
        (RUN_KEY, APPROVED_RUN, StartupSource::MachineRun)
    };
    let approved = approved_map(approved_key);
    win::enum_machine_values(key, view32)
        .into_iter()
        .map(|v| StartupRaw {
            source,
            enabled: approved.get(&v.name.to_lowercase()).copied(),
            name: v.name.clone(),
            command: win::expand_env(&v.as_string()),
            detail: format!(r"HKLM\{}", key),
            system: false,
        })
        .collect()
}

/// StartupApproved 的值是 12 字节二进制：第一个字节奇偶表示禁用/启用
/// （0x02/0x06 = 启用，0x03/0x07 = 禁用），后面是时间戳。
fn approved_map(key: &str) -> std::collections::HashMap<String, bool> {
    let mut m = std::collections::HashMap::new();
    for v in win::enum_user_values(key) {
        if v.kind != 3 {
            continue;
        }
        if let Some(&b) = v.data.first() {
            m.insert(v.name.to_lowercase(), b & 1 == 0);
        }
    }
    m
}

fn scan_folders() -> Vec<StartupRaw> {
    let mut out = Vec::new();
    let approved = approved_map(APPROVED_FOLDER);
    let mut dirs: Vec<(StartupSource, PathBuf, String)> = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push((
            StartupSource::UserFolder,
            Path::new(&appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            "%APPDATA%\\...\\Startup".to_string(),
        ));
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        dirs.push((
            StartupSource::AllUsersFolder,
            Path::new(&pd).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            "%ProgramData%\\...\\Startup".to_string(),
        ));
    }
    for (source, dir, hint) in dirs {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                continue;
            }
            let fname = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if fname.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let lower = fname.to_lowercase();
            let command = if lower.ends_with(".lnk") {
                shortcut_target(&p).unwrap_or_else(|| p.to_string_lossy().to_string())
            } else {
                p.to_string_lossy().to_string()
            };
            out.push(StartupRaw {
                source,
                enabled: approved.get(&lower).copied(),
                name: fname,
                command,
                detail: format!("{}\\{}", hint, p.file_name().unwrap_or_default().to_string_lossy()),
                system: false,
            });
        }
    }
    out
}

fn scan_tasks() -> Vec<StartupRaw> {
    let root = Path::new(r"C:\Windows\System32\Tasks");
    let mut out = Vec::new();
    walk_tasks(root, root, &mut out);
    out
}

fn walk_tasks(root: &Path, dir: &Path, out: &mut Vec<StartupRaw>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            // `\Microsoft\` 下的任务照收，只打标记。
            // 2026-10-22 踩过的坑：VS 安装器的
            // `\Microsoft\VisualStudio\Updates\BackgroundDownload`
            // 一开机就占 339 MB，而「Microsoft 目录一律跳过」让它对四个处置入口全部隐身。
            // 系统任务有 250 多个，噪音靠调用方按「是否真的在跑」过滤，不靠这里一刀切。
            walk_tasks(root, &p, out);
            continue;
        }
        let text = read_task_text(&p);
        let Some(t) = core_startup::parse_task_xml(&text) else {
            continue;
        };
        if t.command.trim().is_empty() {
            continue;
        }
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('/', "\\"))
            .unwrap_or_else(|_| name.clone());
        let command = if t.arguments.is_empty() {
            t.command.clone()
        } else {
            format!("{} {}", t.command, t.arguments)
        };
        out.push(StartupRaw {
            source: StartupSource::ScheduledTask,
            name: format!("\\{}", rel),
            command: win::expand_env(&command),
            enabled: Some(t.enabled),
            detail: format!("任务文件 {}\\{}", dir.display(), name),
            system: rel.to_ascii_lowercase().starts_with("microsoft\\"),
        });
    }
}

/// 任务文件是 UTF-16LE（带 BOM），也兼容 UTF-8
#[allow(clippy::chunks_exact_to_as_chunks)] // as_chunks 要 Rust 1.88+，本工程 MSRV 是 1.75
fn read_task_text(p: &Path) -> String {
    let bytes = fs::read(p).unwrap_or_default();
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    }
}

// ---------------- 启用 / 禁用 ----------------

/// 启用或禁用一项。禁用不改原值，只改批准记录（可逆）。
pub fn set_enabled(source: StartupSource, name: &str, enable: bool) -> Result<(), String> {
    match source {
        StartupSource::UserRun | StartupSource::MachineRun => {
            write_approved(APPROVED_RUN, name, enable)
        }
        StartupSource::MachineRun32 => write_approved(APPROVED_RUN32, name, enable),
        StartupSource::UserFolder | StartupSource::AllUsersFolder => {
            write_approved(APPROVED_FOLDER, name, enable)
        }
        StartupSource::ScheduledTask => {
            let arg = if enable { "/enable" } else { "/disable" };
            let out = std::process::Command::new("schtasks")
                .args(["/change", "/tn", name, arg])
                .output()
                .map_err(|e| format!("调 schtasks 失败: {}", e))?;
            if !out.status.success() {
                let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(format!(
                    "schtasks {} 失败: {}",
                    arg,
                    if msg.is_empty() {
                        String::from_utf8_lossy(&out.stdout).trim().to_string()
                    } else {
                        msg
                    }
                ));
            }
            Ok(())
        }
    }
}

fn write_approved(key: &str, name: &str, enable: bool) -> Result<(), String> {
    // 12 字节：首字节 0x02 启用 / 0x03 禁用，其余留给系统写时间戳
    let mut bytes = [0u8; 12];
    bytes[0] = if enable { 0x02 } else { 0x03 };
    win::set_user_reg_binary(key, name, &bytes)
}

// ---------------- 快捷方式目标（IShellLink） ----------------

pub const CLSID_SHELLLINK: crate::sys::GUID = crate::sys::GUID {
    data1: 0x0002_1401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
pub const IID_ISHELLLINKW: crate::sys::GUID = crate::sys::GUID {
    data1: 0x0002_14F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
pub const IID_IPERSISTFILE: crate::sys::GUID = crate::sys::GUID {
    data1: 0x0000_010B,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[repr(C)]
struct IShellLinkWVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const crate::sys::GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    /// 第 4 个槽：GetPath(pszFile, cch, pfd, fFlags)
    get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> i32,
}

#[repr(C)]
struct IPersistFileVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const crate::sys::GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut crate::sys::GUID) -> i32,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> i32,
    load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
}

/// 读 `.lnk` 的目标路径；解不出来返回 None（不猜）
pub fn shortcut_target(lnk: &Path) -> Option<String> {
    crate::sys::com_init();
    unsafe {
        let mut link: *mut c_void = std::ptr::null_mut();
        let hr = crate::sys::CoCreateInstance(
            &CLSID_SHELLLINK,
            std::ptr::null(),
            1, // CLSCTX_INPROC_SERVER
            &IID_ISHELLLINKW,
            &mut link,
        );
        if hr < 0 || link.is_null() {
            return None;
        }

        let mut pf: *mut c_void = std::ptr::null_mut();
        let hr = (crate::sys::vtable::<IShellLinkWVtbl>(link).query_interface)(
            link,
            &IID_IPERSISTFILE,
            &mut pf,
        );
        if hr < 0 || pf.is_null() {
            crate::sys::release(link);
            return None;
        }

        let w = win::wide(&lnk.to_string_lossy());
        // STGM_READ = 0
        let hr = (crate::sys::vtable::<IPersistFileVtbl>(pf).load)(pf, w.as_ptr(), 0);
        let mut out = None;
        if hr >= 0 {
            let mut buf = [0u16; 1024];
            let hr = (crate::sys::vtable::<IShellLinkWVtbl>(link).get_path)(
                link,
                buf.as_mut_ptr(),
                buf.len() as i32,
                std::ptr::null_mut(),
                0,
            );
            if hr >= 0 {
                let n = buf.iter().position(|c| *c == 0).unwrap_or(0);
                if n > 0 {
                    out = Some(String::from_utf16_lossy(&buf[..n]));
                }
            }
        }
        crate::sys::release(pf);
        crate::sys::release(link);
        out.map(|s| win::expand_env(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 启动项扫描不炸且来源合法() {
        let items = scan();
        // 至少应该有计划任务或 Run 值；纯粹防 panic 和明显坏数据
        for it in &items {
            assert!(!it.name.is_empty(), "名字不该为空");
            assert!(
                matches!(
                    it.source,
                    StartupSource::UserRun
                        | StartupSource::MachineRun
                        | StartupSource::MachineRun32
                        | StartupSource::UserFolder
                        | StartupSource::AllUsersFolder
                        | StartupSource::ScheduledTask
                ),
                "来源非法"
            );
        }
    }

    #[test]
    fn 批准记录字节符合约定() {
        // 只验编码规则，不落盘
        let mut b = [0u8; 12];
        b[0] = 0x02;
        assert!(b[0] & 1 == 0, "0x02 表示启用");
        b[0] = 0x03;
        assert!(b[0] & 1 == 1, "0x03 表示禁用");
    }
}
