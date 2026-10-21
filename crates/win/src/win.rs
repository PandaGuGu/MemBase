//! 最小 Win32 绑定：只绑定这个工具需要的 API，零外部 crate。
//! 手写 FFI 而不是用 windows-sys，是为了让 `cargo build` 完全离线、几秒编译完，
//! 产出的 exe 也只有几百 KB。
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

pub type HANDLE = *mut c_void;
pub type BOOL = i32;
pub type DWORD = u32;
pub type WCHAR = u16;
pub type ULONG_PTR = usize;
pub type SIZE_T = usize;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

pub const TH32CS_SNAPPROCESS: u32 = 0x0002;

pub const PROCESS_TERMINATE: u32 = 0x0001;
pub const PROCESS_VM_READ: u32 = 0x0010;
pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// GetExitCodeProcess 里表示「还在跑」的哨兵值（不是真退出码）
pub const STILL_ACTIVE: u32 = 259;

pub const SC_MANAGER_CONNECT: u32 = 0x0001;
pub const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x0004;
pub const SERVICE_QUERY_CONFIG: u32 = 0x0001;
pub const SERVICE_QUERY_STATUS: u32 = 0x0004;
pub const SERVICE_STOP: u32 = 0x0020;
pub const SERVICE_START: u32 = 0x0010;
pub const SERVICE_WIN32: u32 = 0x0000_0030;
pub const SERVICE_STATE_ALL: u32 = 0x0000_0003;
pub const SC_ENUM_PROCESS_INFO: u32 = 0;
pub const SERVICE_CONTROL_STOP: u32 = 0x0000_0001;
pub const SC_STATUS_PROCESS_INFO: u32 = 0;
pub const SERVICE_RUNNING: u32 = 0x0000_0004;
pub const SERVICE_STOPPED: u32 = 0x0000_0001;

pub const START_TYPE_BOOT: u32 = 0;
pub const START_TYPE_SYSTEM: u32 = 1;
pub const START_TYPE_AUTO: u32 = 2;
pub const START_TYPE_MANUAL: u32 = 3;
pub const START_TYPE_DISABLED: u32 = 4;

pub const SERVICE_CHANGE_CONFIG: u32 = 0x0002;
/// ChangeServiceConfig 里「这一项不改」的哨兵值
pub const SERVICE_NO_CHANGE: u32 = 0xFFFF_FFFF;
const TOKEN_ELEVATION_CLASS: u32 = 20;

pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

pub const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;

const HKEY_LOCAL_MACHINE: HANDLE = 0x8000_0002isize as HANDLE;
const KEY_READ: u32 = 0x0002_0019;
const REG_DWORD: u32 = 4;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 关窗口用的消息：和用户点标题栏那个 ✕ 是同一条
pub const WM_CLOSE: u32 = 0x0010;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}

impl FILETIME {
    pub fn to_u64(self) -> u64 {
        ((self.dwHighDateTime as u64) << 32) | self.dwLowDateTime as u64
    }
    pub fn from_u64(v: u64) -> FILETIME {
        FILETIME {
            dwLowDateTime: (v & 0xFFFF_FFFF) as u32,
            dwHighDateTime: (v >> 32) as u32,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

impl SYSTEMTIME {
    pub fn fmt(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.wYear, self.wMonth, self.wDay, self.wHour, self.wMinute, self.wSecond
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}

#[repr(C)]
pub struct PROCESSENTRY32W {
    pub dwSize: u32,
    pub cntUsage: u32,
    pub th32ProcessID: u32,
    pub th32DefaultHeapID: ULONG_PTR,
    pub th32ModuleID: u32,
    pub cntThreads: u32,
    pub th32ParentProcessID: u32,
    pub pcPriClassBase: i32,
    pub dwFlags: u32,
    pub szExeFile: [WCHAR; 260],
}

impl Default for PROCESSENTRY32W {
    fn default() -> Self {
        PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            cntUsage: 0,
            th32ProcessID: 0,
            th32DefaultHeapID: 0,
            th32ModuleID: 0,
            cntThreads: 0,
            th32ParentProcessID: 0,
            pcPriClassBase: 0,
            dwFlags: 0,
            szExeFile: [0; 260],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PROCESS_MEMORY_COUNTERS {
    pub cb: u32,
    pub PageFaultCount: u32,
    pub PeakWorkingSetSize: SIZE_T,
    pub WorkingSetSize: SIZE_T,
    pub QuotaPeakPagedPoolUsage: SIZE_T,
    pub QuotaPagedPoolUsage: SIZE_T,
    pub QuotaPeakNonPagedPoolUsage: SIZE_T,
    pub QuotaNonPagedPoolUsage: SIZE_T,
    pub PagefileUsage: SIZE_T,
    pub PeakPagefileUsage: SIZE_T,
    /// PROCESS_MEMORY_COUNTERS_EX 的私有提交；cb 传整个结构大小时系统才填这一项
    pub PrivateUsage: SIZE_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SID_AND_ATTRIBUTES {
    pub Sid: *mut c_void,
    pub Attributes: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TOKEN_USER {
    pub User: SID_AND_ATTRIBUTES,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SERVICE_STATUS {
    pub dwServiceType: u32,
    pub dwCurrentState: u32,
    pub dwControlsAccepted: u32,
    pub dwWin32ExitCode: u32,
    pub dwServiceSpecificExitCode: u32,
    pub dwCheckPoint: u32,
    pub dwWaitHint: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SERVICE_STATUS_PROCESS {
    pub dwServiceType: u32,
    pub dwCurrentState: u32,
    pub dwControlsAccepted: u32,
    pub dwWin32ExitCode: u32,
    pub dwServiceSpecificExitCode: u32,
    pub dwCheckPoint: u32,
    pub dwWaitHint: u32,
    pub dwProcessId: u32,
    pub dwServiceFlags: u32,
}

#[repr(C)]
pub struct ENUM_SERVICE_STATUS_PROCESSW {
    pub lpServiceName: *mut u16,
    pub lpDisplayName: *mut u16,
    pub ServiceStatusProcess: SERVICE_STATUS_PROCESS,
}

#[repr(C)]
pub struct QUERY_SERVICE_CONFIGW {
    pub dwServiceType: u32,
    pub dwStartType: u32,
    pub dwErrorControl: u32,
    pub lpBinaryPathName: *mut u16,
    pub lpLoadOrderGroup: *mut u16,
    pub dwTagId: u32,
    pub lpDependencies: *mut u16,
    pub lpServiceStartName: *mut u16,
    pub lpDisplayName: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEM_INFO {
    pub wProcessorArchitecture: u16,
    pub wReserved: u16,
    pub dwPageSize: u32,
    pub lpMinimumApplicationAddress: *mut c_void,
    pub lpMaximumApplicationAddress: *mut c_void,
    pub dwActiveProcessorMask: usize,
    pub dwNumberOfProcessors: u32,
    pub dwProcessorType: u32,
    pub dwAllocationGranularity: u32,
    pub wProcessorLevel: u16,
    pub wProcessorRevision: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> BOOL;
    fn GetTickCount64() -> u64;
    fn GetSystemTimeAsFileTime(lpSystemTimeAsFileTime: *mut FILETIME);
    fn FileTimeToSystemTime(lpFileTime: *const FILETIME, lpSystemTime: *mut SYSTEMTIME) -> BOOL;
    fn FileTimeToLocalFileTime(lpFileTime: *const FILETIME, lpLocalFileTime: *mut FILETIME) -> BOOL;
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> HANDLE;
    fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
    fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: BOOL, dwProcessId: u32) -> HANDLE;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn QueryFullProcessImageNameW(
        hProcess: HANDLE,
        dwFlags: u32,
        lpExeName: *mut u16,
        lpdwSize: *mut u32,
    ) -> BOOL;
    fn GetProcessTimes(
        hProcess: HANDLE,
        lpCreationTime: *mut FILETIME,
        lpExitTime: *mut FILETIME,
        lpKernelTime: *mut FILETIME,
        lpUserTime: *mut FILETIME,
    ) -> BOOL;
    fn GetProcessHandleCount(hProcess: HANDLE, pdwHandleCount: *mut u32) -> BOOL;
    fn K32GetProcessMemoryInfo(
        Process: HANDLE,
        ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS,
        cb: u32,
    ) -> BOOL;
    fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut u32) -> BOOL;
    fn ProcessIdToSessionId(dwProcessId: u32, pSessionId: *mut u32) -> BOOL;
    fn GetComputerNameW(lpBuffer: *mut u16, nSize: *mut u32) -> BOOL;
    fn GetUserNameW(lpBuffer: *mut u16, pcbBuffer: *mut u32) -> BOOL;
    fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);
    fn GetSystemDirectoryW(lpBuffer: *mut u16, uSize: u32) -> u32;
    fn Sleep(dwMilliseconds: u32);
    fn GetTickCount() -> u32;
    pub fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(
        lpMachineName: *const u16,
        lpDatabaseName: *const u16,
        dwDesiredAccess: u32,
    ) -> HANDLE;
    fn OpenServiceW(hSCManager: HANDLE, lpServiceName: *const u16, dwDesiredAccess: u32)
        -> HANDLE;
    fn EnumServicesStatusExW(
        hSCManager: HANDLE,
        InfoLevel: u32,
        dwServiceType: u32,
        dwServiceState: u32,
        lpServices: *mut u8,
        cbBufSize: u32,
        pcbBytesNeeded: *mut u32,
        lpServicesReturned: *mut u32,
        lpResumeHandle: *mut u32,
        pszGroupName: *const u16,
    ) -> BOOL;
    fn QueryServiceConfigW(
        hService: HANDLE,
        lpServiceConfig: *mut u8,
        cbBufSize: u32,
        pcbBytesNeeded: *mut u32,
    ) -> BOOL;
    fn QueryServiceStatusEx(
        hService: HANDLE,
        InfoLevel: u32,
        lpBuffer: *mut u8,
        cbBufSize: u32,
        pcbBytesNeeded: *mut u32,
    ) -> BOOL;
    fn ControlService(
        hService: HANDLE,
        dwControl: u32,
        lpServiceStatus: *mut SERVICE_STATUS,
    ) -> BOOL;
    fn StartServiceW(
        hService: HANDLE,
        dwNumServiceArgs: u32,
        lpServiceArgVectors: *const *const u16,
    ) -> BOOL;
    fn CloseServiceHandle(hSCObject: HANDLE) -> BOOL;
    fn ChangeServiceConfigW(
        hService: HANDLE,
        dwServiceType: u32,
        dwStartType: u32,
        dwErrorControl: u32,
        lpBinaryPathName: *const u16,
        lpLoadOrderGroup: *const u16,
        lpdwTagId: *mut u32,
        lpDependencies: *const u16,
        lpServiceStartName: *const u16,
        lpPassword: *const u16,
        lpDisplayName: *const u16,
    ) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(
        ProcessHandle: HANDLE,
        DesiredAccess: u32,
        TokenHandle: *mut HANDLE,
    ) -> BOOL;
    fn GetTokenInformation(
        TokenHandle: HANDLE,
        TokenInformationClass: u32,
        TokenInformation: *mut c_void,
        TokenInformationLength: u32,
        ReturnLength: *mut u32,
    ) -> BOOL;
    fn LookupAccountSidW(
        lpSystemName: *const u16,
        Sid: *mut c_void,
        Name: *mut u16,
        cchName: *mut u32,
        ReferencedDomainName: *mut u16,
        cchReferencedDomainName: *mut u32,
        peUse: *mut u32,
    ) -> BOOL;
    fn RegOpenKeyExW(
        hKey: HANDLE,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut HANDLE,
    ) -> i32;
    fn RegQueryValueExW(
        hKey: HANDLE,
        lpValueName: *const u16,
        lpReserved: *mut u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;
    fn RegCloseKey(hKey: HANDLE) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(
        lpEnumFunc: Option<unsafe extern "system" fn(HANDLE, isize) -> BOOL>,
        lParam: isize,
    ) -> BOOL;
    fn GetWindowThreadProcessId(hWnd: HANDLE, lpdwProcessId: *mut u32) -> u32;
    fn IsWindowVisible(hWnd: HANDLE) -> BOOL;
    fn GetWindowTextLengthW(hWnd: HANDLE) -> i32;
    fn PostMessageW(hWnd: HANDLE, Msg: u32, wParam: usize, lParam: isize) -> BOOL;
}

// ---------------- 小工具 ----------------

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub unsafe fn pcwstr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

pub fn from_wide_buf(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn sleep_ms(ms: u32) {
    unsafe {
        Sleep(ms);
    }
}

pub fn memory_status() -> MEMORYSTATUSEX {
    let mut m = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe {
        GlobalMemoryStatusEx(&mut m);
    }
    m
}

pub fn uptime_secs() -> u64 {
    unsafe { GetTickCount64() / 1000 }
}

fn filetime_to_systime_local(ft: FILETIME) -> SYSTEMTIME {
    unsafe {
        let mut local = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        if FileTimeToLocalFileTime(&ft, &mut local) == 0 {
            local = ft;
        }
        let mut st = SYSTEMTIME::default();
        FileTimeToSystemTime(&local, &mut st);
        st
    }
}

/// 把 FILETIME（100ns 整数）转成本地可读时间。0 = 未知，返回空串。
pub fn filetime_local_string(v: u64) -> String {
    if v == 0 {
        return String::new();
    }
    filetime_to_systime_local(FILETIME::from_u64(v)).fmt()
}

pub fn now_local_string() -> String {
    let mut ft = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    unsafe {
        GetSystemTimeAsFileTime(&mut ft);
    }
    filetime_to_systime_local(ft).fmt()
}

pub fn now_utc_string() -> String {
    let mut ft = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    unsafe {
        GetSystemTimeAsFileTime(&mut ft);
    }
    let mut st = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut st);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

/// 开机时刻 = 当前时间 - uptime（100ns 单位换算）
pub fn boot_time_string(uptime_s: u64) -> String {
    let mut ft = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    unsafe {
        GetSystemTimeAsFileTime(&mut ft);
    }
    let ticks = ft.to_u64().saturating_sub(uptime_s * 10_000_000);
    filetime_to_systime_local(FILETIME::from_u64(ticks)).fmt()
}

pub fn computer_name() -> String {
    let mut buf = [0u16; 256];
    let mut size = buf.len() as u32;
    unsafe {
        GetComputerNameW(buf.as_mut_ptr(), &mut size);
    }
    from_wide_buf(&buf)
}

pub fn user_name() -> String {
    let mut buf = [0u16; 256];
    let mut size = buf.len() as u32;
    unsafe {
        if GetUserNameW(buf.as_mut_ptr(), &mut size) == 0 {
            return String::new();
        }
    }
    from_wide_buf(&buf)
}

pub fn logical_processors() -> u32 {
    let mut si = SYSTEM_INFO::default();
    unsafe {
        GetSystemInfo(&mut si);
    }
    si.dwNumberOfProcessors
}

pub fn system_drive() -> String {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return String::new();
    }
    let dir = from_wide_buf(&buf);
    match dir.find(':') {
        Some(i) => format!("{}:", &dir[..i]),
        None => dir,
    }
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

pub fn tick_ms() -> u32 {
    unsafe { GetTickCount() }
}

// ---------------- 进程 ----------------

pub struct ProcHandle(pub HANDLE);

impl Drop for ProcHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub fn open_process(pid: u32, access: u32) -> Option<ProcHandle> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        None
    } else {
        Some(ProcHandle(h))
    }
}

pub fn process_image_path(h: HANDLE) -> String {
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) };
    if ok == 0 {
        return String::new();
    }
    from_wide_buf(&buf[..size as usize])
}

pub fn process_start_time(h: HANDLE) -> String {
    let mut c = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut e = c;
    let mut k = c;
    let mut u = c;
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        return String::new();
    }
    filetime_to_systime_local(c).fmt()
}

pub fn process_handle_count(h: HANDLE) -> u32 {
    let mut n: u32 = 0;
    unsafe {
        GetProcessHandleCount(h, &mut n);
    }
    n
}

pub fn process_memory(h: HANDLE) -> Option<PROCESS_MEMORY_COUNTERS> {
    let mut pmc = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    let ok = unsafe { K32GetProcessMemoryInfo(h, &mut pmc, pmc.cb) };
    if ok == 0 {
        None
    } else {
        Some(pmc)
    }
}

pub fn session_id(pid: u32) -> u32 {
    let mut sid: u32 = 0;
    unsafe {
        ProcessIdToSessionId(pid, &mut sid);
    }
    sid
}

/// 进程累计 CPU 秒数（内核 + 用户）。FILETIME 单位是 100ns。
pub fn process_cpu_seconds(h: HANDLE) -> f64 {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        0.0
    } else {
        (k.to_u64() + u.to_u64()) as f64 / 10_000_000.0
    }
}

/// 进程所属账户，形如 `NT AUTHORITY\SYSTEM` / `DESKTOP\alice`。
/// 打不开令牌（受保护进程、权限不足）时返回空串。
pub fn process_user(h: HANDLE) -> String {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(h, TOKEN_QUERY, &mut token) } == 0 {
        return String::new();
    }

    let mut len: u32 = 0;
    unsafe {
        GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            std::ptr::null_mut(),
            0,
            &mut len,
        );
    }
    if len == 0 {
        unsafe {
            CloseHandle(token);
        }
        return String::new();
    }

    // 按 usize 对齐分配，避免把 TOKEN_USER 读在未对齐地址上
    let words = (len as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buf: Vec<usize> = vec![0; words];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 {
        return String::new();
    }

    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    if sid.is_null() {
        return String::new();
    }

    let mut name = [0u16; 256];
    let mut name_len = name.len() as u32;
    let mut domain = [0u16; 256];
    let mut domain_len = domain.len() as u32;
    let mut sid_type: u32 = 0;
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut sid_type,
        )
    };
    if ok == 0 {
        return String::new();
    }

    let n = from_wide_buf(&name[..(name_len as usize).min(name.len())]);
    let d = from_wide_buf(&domain[..(domain_len as usize).min(domain.len())]);
    if d.is_empty() {
        n
    } else {
        format!("{d}\\{n}")
    }
}

pub fn terminate_process(pid: u32) -> Result<(), String> {
    match open_process(pid, PROCESS_TERMINATE) {
        Some(h) => {
            let ok = unsafe { TerminateProcess(h.0, 1) };
            if ok == 0 {
                Err(format!("TerminateProcess 失败，错误码 {}", last_error()))
            } else {
                Ok(())
            }
        }
        None => Err(format!("打不开进程（权限不足或已退出），错误码 {}", last_error())),
    }
}

/// 在已有句柄上读创建时间，避免为了取时间再开一次进程
pub fn process_created(h: HANDLE) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        None
    } else {
        Some(c.to_u64())
    }
}

/// 进程创建时间（FILETIME 100ns 整数）。
///
/// 和 PID 一起构成进程身份：PID 会被系统回收再分配，只认 PID 的话，
/// 「看快照 → 复核 → 动手」之间目标一旦退出，就可能打到复用同一 PID 的新进程上。
pub fn process_create_time(pid: u32) -> Option<u64> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    process_created(h.0)
}

/// 进程还在，并且还是「刚才看到的那个」。
/// `created == 0` 表示调用方没拿到创建时间（老快照 / 权限不足），此时退回只看 PID。
pub fn process_alive_as(pid: u32, created: u64) -> bool {
    // 光「打得开进程」不等于「还活着」：进程已经在收尾（窗口已关、线程在退）
    // 时对象还在，句柄也还打得开，但 TerminateProcess 会返回 ERROR_ACCESS_DENIED。
    // 所以先核身份，再看退出码是不是 STILL_ACTIVE。
    let now = match process_create_time(pid) {
        None => return false,
        Some(v) => v,
    };
    if created != 0 && now != created {
        return false;
    }
    match process_exit_code(pid) {
        Some(code) => code == STILL_ACTIVE,
        None => true, // 读不到退出码就按「还在」算，宁可多等一轮
    }
}

/// 进程退出码；已在收尾的进程会返回 259 以外的值。拿不到返回 None。
pub fn process_exit_code(pid: u32) -> Option<u32> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut code = 0u32;
    if unsafe { GetExitCodeProcess(h.0, &mut code) } == 0 {
        None
    } else {
        Some(code)
    }
}

/// 校验身份后终止：校验和 TerminateProcess 用**同一个句柄**，
/// 中间不留 PID 被复用后误伤的窗口。
pub fn terminate_process_checked(pid: u32, created: u64) -> Result<(), String> {
    let h = match open_process(
        pid,
        PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
    ) {
        Some(h) => h,
        None => {
            return Err(format!(
                "打不开进程（权限不足或已退出），错误码 {}",
                last_error()
            ))
        }
    };
    if created != 0 {
        let cur = match process_created(h.0) {
            Some(v) => v,
            None => {
                return Err(format!("读不到创建时间，错误码 {}", last_error()));
            }
        };
        if cur != created {
            return Err("PID 已被系统复用给别的进程，放弃".into());
        }
    }
    let ok = unsafe { TerminateProcess(h.0, 1) };
    if ok == 0 {
        Err(format!("TerminateProcess 失败，错误码 {}", last_error()))
    } else {
        Ok(())
    }
}

struct CloseEnumCtx {
    pids: Vec<u32>,
    hits: std::collections::HashMap<u32, usize>,
}

unsafe extern "system" fn close_enum_proc(hwnd: HANDLE, lparam: isize) -> BOOL {
    let ctx = &mut *(lparam as *mut CloseEnumCtx);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || !ctx.pids.contains(&pid) {
        return 1;
    }
    // 只认用户看得见、有标题的顶层窗口：托盘隐藏窗口不参与，
    // 否则会把「关到托盘」的程序硬拽出来，或者在无 UI 的宿主进程上乱发消息。
    if IsWindowVisible(hwnd) == 0 || GetWindowTextLengthW(hwnd) <= 0 {
        return 1;
    }
    if PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
        *ctx.hits.entry(pid).or_insert(0) += 1;
    }
    1
}

/// 给这些进程的可见顶层窗口发 WM_CLOSE —— 和用户点 ✕ 完全一样。
/// 返回 pid → 实际发出消息的窗口数；没有窗口的进程不会出现在结果里。
pub fn request_close_for(pids: &[u32]) -> std::collections::HashMap<u32, usize> {
    let mut ctx = CloseEnumCtx {
        pids: pids.to_vec(),
        hits: std::collections::HashMap::new(),
    };
    if ctx.pids.is_empty() {
        return ctx.hits;
    }
    unsafe {
        EnumWindows(Some(close_enum_proc), &mut ctx as *mut CloseEnumCtx as isize);
    }
    ctx.hits
}

pub fn snapshot_processes() -> Vec<PROCESSENTRY32W> {
    let mut out = Vec::new();
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap.is_null() || snap == INVALID_HANDLE_VALUE {
        return out;
    }
    let mut pe = PROCESSENTRY32W::default();
    let mut ok = unsafe { Process32FirstW(snap, &mut pe) };
    while ok != 0 {
        out.push(pe);
        pe = PROCESSENTRY32W::default();
        ok = unsafe { Process32NextW(snap, &mut pe) };
    }
    unsafe {
        CloseHandle(snap);
    }
    out
}

// ---------------- 服务 ----------------

pub struct ScHandle(pub HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseServiceHandle(self.0);
            }
        }
    }
}

pub fn open_scm() -> Option<ScHandle> {
    let h = unsafe {
        OpenSCManagerW(
            std::ptr::null(),
            std::ptr::null(),
            SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE,
        )
    };
    if h.is_null() {
        None
    } else {
        Some(ScHandle(h))
    }
}

pub fn open_service(scm: HANDLE, name: &str, access: u32) -> Option<ScHandle> {
    let wname = wide(name);
    let h = unsafe { OpenServiceW(scm, wname.as_ptr(), access) };
    if h.is_null() {
        None
    } else {
        Some(ScHandle(h))
    }
}

pub struct SvcEntry {
    pub name: String,
    pub display: String,
    pub state: u32,
    pub pid: u32,
}

pub fn enum_services(scm: HANDLE) -> Vec<SvcEntry> {
    let mut result = Vec::new();
    let mut resume: u32 = 0;
    // 用 u64 分配，保证结构体读取时是按 8 字节对齐的
    let mut words: Vec<u64> = vec![0u64; 256 * 1024 / 8];

    loop {
        let mut needed: u32 = 0;
        let mut returned: u32 = 0;
        let bytes = (words.len() * 8) as u32;
        let ok = unsafe {
            EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                words.as_mut_ptr() as *mut u8,
                bytes,
                &mut needed,
                &mut returned,
                &mut resume,
                std::ptr::null(),
            )
        };

        unsafe {
            let items = words.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;
            for i in 0..returned as usize {
                let it = &*items.add(i);
                result.push(SvcEntry {
                    name: pcwstr_to_string(it.lpServiceName),
                    display: pcwstr_to_string(it.lpDisplayName),
                    state: it.ServiceStatusProcess.dwCurrentState,
                    pid: it.ServiceStatusProcess.dwProcessId,
                });
            }
        }

        if ok != 0 {
            break; // 枚举完成
        }
        if last_error() == ERROR_MORE_DATA && needed > bytes {
            words = vec![0u64; (needed as usize + 7) / 8];
            continue;
        }
        break;
    }
    result
}

pub struct SvcConfig {
    pub start_type: u32,
    pub binary_path: String,
    pub account: String,
}

pub fn query_service_config(svc: HANDLE) -> Option<SvcConfig> {
    unsafe {
        let mut needed: u32 = 0;
        let mut words: Vec<u64> = vec![0u64; 8192 / 8];
        let mut bytes = (words.len() * 8) as u32;
        let mut ok = QueryServiceConfigW(svc, words.as_mut_ptr() as *mut u8, bytes, &mut needed);
        if ok == 0 && last_error() == ERROR_INSUFFICIENT_BUFFER && needed > bytes {
            words = vec![0u64; (needed as usize + 7) / 8];
            bytes = (words.len() * 8) as u32;
            ok = QueryServiceConfigW(svc, words.as_mut_ptr() as *mut u8, bytes, &mut needed);
        }
        if ok == 0 {
            return None;
        }
        let cfg = &*(words.as_ptr() as *const QUERY_SERVICE_CONFIGW);
        Some(SvcConfig {
            start_type: cfg.dwStartType,
            binary_path: pcwstr_to_string(cfg.lpBinaryPathName),
            account: pcwstr_to_string(cfg.lpServiceStartName),
        })
    }
}

pub fn service_state_name(state: u32) -> &'static str {
    match state {
        SERVICE_STOPPED => "Stopped",
        2 => "StartPending",
        3 => "StopPending",
        SERVICE_RUNNING => "Running",
        5 => "ContinuePending",
        6 => "PausePending",
        7 => "Paused",
        _ => "Unknown",
    }
}

pub fn start_mode_name(t: u32) -> &'static str {
    match t {
        START_TYPE_BOOT => "Boot",
        START_TYPE_SYSTEM => "System",
        START_TYPE_AUTO => "Auto",
        START_TYPE_MANUAL => "Manual",
        START_TYPE_DISABLED => "Disabled",
        _ => "Unknown",
    }
}

/// 单个服务当前的启动类型（`Auto` / `Manual` / `Disabled`…）。服务不存在时 None。
pub fn service_start_mode(name: &str) -> Option<String> {
    let scm = open_scm()?;
    let svc = open_service(scm.0, name, SERVICE_QUERY_CONFIG)?;
    let cfg = query_service_config(svc.0)?;
    Some(start_mode_name(cfg.start_type).to_string())
}

pub fn stop_service(name: &str) -> Result<(), String> {
    let scm = match open_scm() {
        Some(s) => s,
        None => return Err(format!("打不开服务管理器，错误码 {}", last_error())),
    };
    let svc = match open_service(scm.0, name, SERVICE_STOP | SERVICE_QUERY_STATUS) {
        Some(s) => s,
        None => return Err(format!("打不开服务，错误码 {}", last_error())),
    };
    let mut status = SERVICE_STATUS::default();
    let ok = unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) };
    if ok == 0 {
        return Err(format!("停止失败，错误码 {}", last_error()));
    }
    Ok(())
}

pub fn start_service(name: &str) -> Result<(), String> {
    let scm = match open_scm() {
        Some(s) => s,
        None => return Err(format!("打不开服务管理器，错误码 {}", last_error())),
    };
    let svc = match open_service(scm.0, name, SERVICE_START | SERVICE_QUERY_STATUS) {
        Some(s) => s,
        None => return Err(format!("打不开服务，错误码 {}", last_error())),
    };
    let ok = unsafe { StartServiceW(svc.0, 0, std::ptr::null()) };
    if ok == 0 {
        let err = last_error();
        // 1056 = ERROR_SERVICE_ALREADY_RUNNING
        if err == 1056 {
            return Ok(());
        }
        return Err(format!("启动失败，错误码 {}", err));
    }
    Ok(())
}

/// 当前进程是否已提权（UAC 提升过）。
///
/// 采集口径依赖这个：没有提权就拿不到别的账户进程的内存，
/// 出来的基准里会有一堆 `工作集=未知`，那种快照不能用来做对比。
pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev: u32 = 0;
        let mut len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION_CLASS,
            &mut elev as *mut u32 as *mut c_void,
            4,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elev != 0
    }
}

/// 改服务的启动类型。要"下次开机别再起来"，这是唯一真的办法——
/// 只 `ControlService(STOP)` 的话重启就回来了。
pub fn set_service_start_mode(name: &str, start_type: u32) -> Result<(), String> {
    let scm = match open_scm() {
        Some(s) => s,
        None => return Err(format!("打不开服务管理器，错误码 {}", last_error())),
    };
    let svc = match open_service(scm.0, name, SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG) {
        Some(s) => s,
        None => return Err(format!("打不开服务，错误码 {}", last_error())),
    };
    let ok = unsafe {
        ChangeServiceConfigW(
            svc.0,
            SERVICE_NO_CHANGE,
            start_type,
            SERVICE_NO_CHANGE,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        let err = last_error();
        // 5 = ERROR_ACCESS_DENIED：没提权改不动服务配置
        if err == 5 {
            return Err("拒绝访问：需要以管理员身份运行".to_string());
        }
        return Err(format!("修改启动类型失败，错误码 {}", err));
    }
    Ok(())
}

/// 禁止开机自启（`start= disabled`）
pub fn disable_service_start(name: &str) -> Result<(), String> {
    set_service_start_mode(name, START_TYPE_DISABLED)
}

/// 恢复成手动启动，作为「撤销禁用」的安全落点
pub fn enable_service_start(name: &str) -> Result<(), String> {
    set_service_start_mode(name, START_TYPE_MANUAL)
}

pub fn service_status(svc: HANDLE) -> Option<SERVICE_STATUS_PROCESS> {    let mut buf = [0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
    let mut needed: u32 = 0;
    let ok = unsafe {
        QueryServiceStatusEx(
            svc,
            SC_STATUS_PROCESS_INFO,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(unsafe { *(buf.as_ptr() as *const SERVICE_STATUS_PROCESS) })
}

// ---------------- 权限 / 自身保护 / 外壳 ----------------

/// 是否以管理员权限运行。
///
/// 直接读令牌的提权标志，不用 `IsUserAnAdmin`：那个在 UAC 过滤令牌下的行为
/// 更容易误判，而「能不能读别的进程内存」完全由提权状态决定。
pub fn is_admin() -> bool {
    is_elevated()
}

/// 把当前控制台切到 UTF-8，否则中文在 cmd/老版 PowerShell 里会花屏
pub fn console_utf8() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleOutputCP(wCodePageID: u32) -> BOOL;
        fn SetConsoleCP(wCodePageID: u32) -> BOOL;
    }
    unsafe {
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

/// 界面版带参数启动时，把标准输入输出接回调用它的控制台。
///
/// GUI 子系统进程默认没有控制台；`AttachConsole` 之后标准句柄未必可用，
/// 所以显式打开 `CONOUT$` / `CONIN$` 再 `SetStdHandle`。
/// 双击启动（没有父控制台）时静默返回，不影响界面。
pub fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> BOOL;
        fn GetStdHandle(nStdHandle: u32) -> HANDLE;
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *mut c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: HANDLE,
        ) -> HANDLE;
        fn SetStdHandle(nStdHandle: u32, nHandle: HANDLE) -> BOOL;
    }

    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let valid = |h: HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
        let open = |name: &str| {
            let w = wide(name);
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        // 已经有有效的标准句柄（重定向到文件 / 管道）就别动，否则会把输出抢回控制台
        if !valid(GetStdHandle(STD_OUTPUT_HANDLE)) {
            let out = open("CONOUT$");
            if valid(out) {
                SetStdHandle(STD_OUTPUT_HANDLE, out);
                SetStdHandle(STD_ERROR_HANDLE, out);
            }
        }
        if !valid(GetStdHandle(STD_INPUT_HANDLE)) {
            let inp = open("CONIN$");
            if valid(inp) {
                SetStdHandle(STD_INPUT_HANDLE, inp);
            }
        }
    }
}

pub struct OsInfo {
    pub caption: String,
    pub version: String,
    pub build: String,
}

fn reg_string(key: &str, name: &str) -> Option<String> {
    let key_w = wide(key);
    let name_w = wide(name);
    let mut hkey: HANDLE = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, key_w.as_ptr(), 0, KEY_READ, &mut hkey) } != 0 {
        return None;
    }
    let mut len: u32 = 0;
    let mut typ: u32 = 0;
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut typ,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if r != 0 || len == 0 {
        unsafe {
            RegCloseKey(hkey);
        }
        return None;
    }
    let mut buf: Vec<u16> = vec![0; (len as usize + 1) / 2 + 1];
    let mut got = (buf.len() * 2) as u32;
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut typ,
            buf.as_mut_ptr() as *mut u8,
            &mut got,
        )
    };
    unsafe {
        RegCloseKey(hkey);
    }
    if r != 0 {
        return None;
    }
    let s = from_wide_buf(&buf[..(got as usize / 2).min(buf.len())]);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn reg_dword(key: &str, name: &str) -> Option<u32> {
    let key_w = wide(key);
    let name_w = wide(name);
    let mut hkey: HANDLE = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, key_w.as_ptr(), 0, KEY_READ, &mut hkey) } != 0 {
        return None;
    }
    let mut val: u32 = 0;
    let mut len: u32 = 4;
    let mut typ: u32 = 0;
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut typ,
            &mut val as *mut u32 as *mut u8,
            &mut len,
        )
    };
    unsafe {
        RegCloseKey(hkey);
    }
    if r == 0 && typ == REG_DWORD {
        Some(val)
    } else {
        None
    }
}

/// Win11 的注册表 ProductName 仍常写「Windows 10」，按 build 号纠正一次。
fn caption_for(product: &str, build: u32) -> String {
    if build >= 22000 {
        product.replacen("Windows 10", "Windows 11", 1)
    } else {
        product.to_string()
    }
}

/// 读注册表里的系统版本信息。读不到的项返回空串。
pub fn os_info() -> OsInfo {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let product = reg_string(KEY, "ProductName").unwrap_or_default();
    let display = reg_string(KEY, "DisplayVersion").unwrap_or_default();
    let build_no = reg_string(KEY, "CurrentBuildNumber")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let ubr = reg_dword(KEY, "UBR").unwrap_or(0);

    OsInfo {
        caption: caption_for(&product, build_no),
        version: if display.is_empty() {
            if build_no > 0 {
                format!("10.0.{build_no}")
            } else {
                String::new()
            }
        } else {
            display
        },
        build: match (build_no, ubr) {
            (0, _) => String::new(),
            (b, 0) => b.to_string(),
            (b, u) => format!("{b}.{u}"),
        },
    }
}

/// 打开资源管理器到指定目录
pub fn open_folder(path: &str) {
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: HANDLE,
            lpOperation: *const u16,
            lpFile: *const u16,
            lpParameters: *const u16,
            lpDirectory: *const u16,
            nShowCmd: i32,
        ) -> HANDLE;
    }
    let op = wide("open");
    let file = wide(path);
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
    }
}

/// 保护集合 = 自身父进程链 + 「启动本程序的界面进程」的整棵子进程树。
/// 目的：绝不把工具自己和编辑器/宿主（以及它们的子进程）一起干掉。
pub fn protected_pids() -> Vec<u32> {
    let procs = snapshot_processes();
    let mut parent: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut names: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for p in &procs {
        parent.insert(p.th32ProcessID, p.th32ParentProcessID);
        let raw = from_wide_buf(&p.szExeFile).to_ascii_lowercase();
        names.insert(p.th32ProcessID, raw.trim_end_matches(".exe").to_string());
    }

    let mut out: Vec<u32> = Vec::new();

    // 0) 按名字保护本工具自己的所有实例（用户可能同时开着桌面版和命令行版，
    //    它们不在彼此的父进程链里，但仍然绝不该被清理）
    for (&pid, n) in names.iter() {
        if n == "deskkit" || n == "membase" {
            if !out.contains(&pid) {
                out.push(pid);
            }
        }
    }

    // 1) 父进程链
    let mut chain: Vec<u32> = Vec::new();
    let mut cur = std::process::id();
    for _ in 0..40 {
        if cur == 0 {
            break;
        }
        chain.push(cur);
        match parent.get(&cur) {
            Some(&pp) if pp > 0 && !chain.contains(&pp) => cur = pp,
            _ => break,
        }
    }
    out.extend(chain.iter().copied());

    // 2) 找链上最高的「外部应用」，保护它整棵子树
    let mut root: u32 = 0;
    for &pid in &chain {
        let n = names.get(&pid).cloned().unwrap_or_default();
        if n.eq_ignore_ascii_case("explorer")
            || n.eq_ignore_ascii_case("cmd")
            || n.eq_ignore_ascii_case("conhost")
        {
            continue;
        }
        if let Some(h) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
            let p = process_image_path(h.0).to_ascii_lowercase();
            if p.is_empty() || p.contains(":\\windows\\") {
                continue;
            }
            root = pid; // 越靠上越优先
        }
    }
    if root > 0 {
        let mut children: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        for (&pid, &pp) in parent.iter() {
            children.entry(pp).or_default().push(pid);
        }
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root);
        let mut seen = std::collections::HashSet::new();
        while let Some(p) = queue.pop_front() {
            if !seen.insert(p) {
                continue;
            }
            if !out.contains(&p) {
                out.push(p);
            }
            if let Some(ch) = children.get(&p) {
                for &c in ch {
                    queue.push_back(c);
                }
            }
        }
    }
    out
}


