//! 息屏：立刻关掉显示器（离开座位用）
//!
//! 做法就是 Windows 自己的那条路：给窗口发 `WM_SYSCOMMAND / SC_MONITORPOWER`，
//! lParam = 2 表示「关」，-1 表示「开」，1 表示「省电」。
//! 任何输入（鼠标/键盘）都会把它叫醒，所以这是一个**无损、可逆**的动作。
//!
//! 为什么不用 `SetThreadExecutionState` 或电源计划：那两样是「让系统别睡」，
//! 这里是「立刻睡」，方向相反。不过**叫醒**的时候它反而是最靠谱的一招，
//! 见 [`wake`]。
//!
//! ## 退出息屏靠什么（v1.2）
//!
//! 老版本退出只认界面的按键事件，那条路要求窗口**有焦点**。显示器一关，
//! 窗口很可能就不在前台了，按 ESC 只是把显示器叫亮，键根本到不了程序 ——
//! 这就是「息屏退不出来」的根因。
//!
//! 现在退出走 [`Watch`]：后台线程挂 `WH_KEYBOARD_LL` 低级钩子（系统在输入线程里
//! 直接回调，只要求这个线程泵消息），并叠加 `GetAsyncKeyState` / `GetCursorPos`
//! 轮询。系统级、跟焦点无关，屏幕黑着、窗口被隐藏也照样收得到。
//!
//! 「息屏期间屏蔽键盘」只决定要不要吞掉退出键之外的其他按键 —— 钩子本身**一直挂**，
//! 因为它是唯一不依赖窗口状态、稳定拿到按键的路子。

#![allow(non_snake_case)]

use deskkit_core::config;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type HWND = isize;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type HHOOK = isize;
type HINSTANCE = isize;

const HWND_BROADCAST: HWND = 0xFFFF;
const WM_SYSCOMMAND: u32 = 0x0112;
const SC_MONITORPOWER: usize = 0xF170;
const SM_CMONITORS: i32 = 80;
const SM_CXSCREEN: i32 = 0;
const SMTO_ABORTIFHUNG: u32 = 0x0002;
const MONITOR_OFF: isize = 2;
const MONITOR_ON: isize = -1;

// ---- 全局输入监听用到的常量 ----
const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: u32 = 0x0100;
const WM_SYSKEYDOWN: u32 = 0x0104;
const PM_REMOVE: u32 = 0x0001;
const VK_LBUTTON: i32 = 0x01;
const VK_RBUTTON: i32 = 0x02;
const VK_MBUTTON: i32 = 0x04;
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const ES_CONTINUOUS: u32 = 0x8000_0000;
const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

/// 武装好之后鼠标移动的宽限期：刚松开「息屏」按钮，手还在动，
/// 那一下不算「用户要退出」
const MOUSE_GRACE: Duration = Duration::from_millis(800);

/// `MSG`（x64 布局，按 C 的对齐规则）
#[repr(C)]
struct MSG {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: POINT,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: i32,
    y: i32,
}

/// `KBDLLHOOKSTRUCT`：低级键盘钩子回调拿到的结构
#[repr(C)]
struct KbdHook {
    vk: u32,
    scan: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[link(name = "user32")]
extern "system" {
    fn SendMessageTimeoutW(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> LRESULT;
    fn GetForegroundWindow() -> HWND;
    fn mouse_event(flags: u32, dx: i32, dy: i32, data: u32, extra: usize);
    fn GetSystemMetrics(index: i32) -> i32;
    fn MessageBeep(kind: u32);
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetCursorPos(pt: *mut POINT) -> i32;
    fn ShowCursor(show: i32) -> i32;
    fn SetWindowsHookExW(id: i32, proc: HOOKPROC, module: HINSTANCE, thread: u32) -> HHOOK;
    fn CallNextHookEx(hook: HHOOK, code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    fn UnhookWindowsHookEx(hook: HHOOK) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
}

#[link(name = "kernel32")]
extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
}

type HOOKPROC = Option<extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT>;

/// 显示器台数（含扩展屏）
pub fn monitor_count() -> u32 {
    (unsafe { GetSystemMetrics(SM_CMONITORS) }).max(0) as u32
}

/// 默认息屏时长（秒）：0 表示不自动退出，一直等用户按退出键
pub const DEFAULT_SECS: u64 = 60 * 60;
/// 上限 24 小时，免得手滑打个天文数字
pub const MAX_SECS: u64 = 24 * 60 * 60;

fn cfg_path(name: &str) -> PathBuf {
    config::config_path().with_file_name(name)
}

fn write_cfg(name: &str, value: &str) -> Result<(), String> {
    let p = cfg_path(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败: {e}"))?;
    }
    std::fs::write(p, value).map_err(|e| format!("写入失败: {e}"))
}

/// 开关型配置：「1」/「0」，缺省走兜底值
fn read_bool(name: &str, fallback: bool) -> bool {
    match std::fs::read_to_string(cfg_path(name)) {
        Ok(t) => match t.trim() {
            "1" | "true" | "on" | "yes" | "开" => true,
            "0" | "false" | "off" | "no" | "关" => false,
            _ => fallback,
        },
        Err(_) => fallback,
    }
}

fn write_bool(name: &str, v: bool) -> Result<(), String> {
    write_cfg(name, if v { "1" } else { "0" })
}

/// 读取默认息屏时长（秒）。老版本存的是分钟，这里顺手迁移过来。
pub fn default_secs() -> u64 {
    if let Ok(t) = std::fs::read_to_string(cfg_path("screen_off_seconds.txt")) {
        if let Ok(n) = t.trim().parse::<u64>() {
            return n.min(MAX_SECS);
        }
    }
    // 兼容：上一版只有「分钟」
    if let Ok(t) = std::fs::read_to_string(cfg_path("screen_off_minutes.txt")) {
        if let Ok(m) = t.trim().parse::<u64>() {
            return (m * 60).min(MAX_SECS);
        }
    }
    DEFAULT_SECS
}

/// 写默认息屏时长（界面上的时/分/秒改一下就落盘，命令行同一个文件）
pub fn set_default_secs(n: u64) -> Result<(), String> {
    write_cfg("screen_off_seconds.txt", &n.min(MAX_SECS).to_string())
}

/// 鼠标移动也算退出（默认开：给用户留一条一定走得通的路）
pub fn mouse_exit() -> bool {
    read_bool("screen_mouse_exit.txt", true)
}

pub fn set_mouse_exit(v: bool) -> Result<(), String> {
    write_bool("screen_mouse_exit.txt", v)
}

/// 息屏期间屏蔽键盘，只放行退出键（默认关）
pub fn block_keys() -> bool {
    read_bool("screen_block_keys.txt", false)
}

pub fn set_block_keys(v: bool) -> Result<(), String> {
    write_bool("screen_block_keys.txt", v)
}

/// 息屏时要不要**顺手把显示器电源也关掉**。
///
/// 默认 **关**：只盖一层全屏纯黑窗口，显示器始终亮着。
/// 原因很实在：一旦真发了 `SC_MONITORPOWER`，能不能再叫醒就全看驱动和电源管理，
/// 有的机器叫不醒 —— 那就变成「屏幕上什么都没有、按什么键都不亮」的死局。
/// 息屏助手默认也是这样（它那个「同时关闭显示器」是选项，不是默认行为）。
pub fn power_off() -> bool {
    read_bool("screen_power_off.txt", false)
}

pub fn set_power_off(v: bool) -> Result<(), String> {
    write_bool("screen_power_off.txt", v)
}

/// 退出息屏用的键。默认 ESC，可以改成空格 / 回车 / 任意键
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitKey {
    Esc,
    Space,
    Enter,
    Any,
}

impl ExitKey {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitKey::Esc => "esc",
            ExitKey::Space => "space",
            ExitKey::Enter => "enter",
            ExitKey::Any => "any",
        }
    }

    /// 界面上显示的名字
    pub fn label(self) -> &'static str {
        match self {
            ExitKey::Esc => "ESC",
            ExitKey::Space => "空格",
            ExitKey::Enter => "回车",
            ExitKey::Any => "任意键",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "esc" | "escape" | "27" => Some(ExitKey::Esc),
            "space" | "空格" | "32" => Some(ExitKey::Space),
            "enter" | "return" | "回车" | "13" => Some(ExitKey::Enter),
            "any" | "anykey" | "任意" | "任意键" => Some(ExitKey::Any),
            _ => None,
        }
    }

    pub fn all() -> [ExitKey; 4] {
        [ExitKey::Esc, ExitKey::Space, ExitKey::Enter, ExitKey::Any]
    }

    /// 对应的虚拟键码；`Any` 没有单一键码（-1）
    fn vk(self) -> i32 {
        match self {
            ExitKey::Esc => 0x1B,
            ExitKey::Space => 0x20,
            ExitKey::Enter => 0x0D,
            ExitKey::Any => -1,
        }
    }
}

pub fn exit_key() -> ExitKey {
    std::fs::read_to_string(cfg_path("screen_exit_key.txt"))
        .ok()
        .and_then(|t| ExitKey::parse(&t))
        .unwrap_or(ExitKey::Esc)
}

pub fn set_exit_key(k: ExitKey) -> Result<(), String> {
    write_cfg("screen_exit_key.txt", k.as_str())
}

/// 秒 -> `1:05:09`（小时不补零，分秒补零）
pub fn fmt_hms(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

/// `1:05:09` / `5m` / `90s` / `2h` / 纯数字（秒）→ 秒数
pub fn parse_duration(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let num = |s: &str| s.trim().parse::<f64>().ok();
    if t.contains(':') {
        let parts: Vec<&str> = t.split(':').collect();
        if parts.len() > 3 {
            return None;
        }
        let mut secs = 0f64;
        for p in &parts {
            secs = secs * 60.0 + num(p)?;
        }
        return Some((secs.max(0.0) as u64).min(MAX_SECS));
    }
    let (value, mult) = if let Some(v) = t.strip_suffix("ms").and_then(num) {
        return Some((v / 1000.0).max(0.0) as u64);
    } else if let Some(v) = t.strip_suffix('h').and_then(num) {
        (v, 3600.0)
    } else if let Some(v) = t.strip_suffix('m').and_then(num) {
        (v, 60.0)
    } else if let Some(v) = t.strip_suffix('s').and_then(num) {
        (v, 1.0)
    } else {
        (num(&t)?, 1.0)
    };
    Some(((value * mult).max(0.0) as u64).min(MAX_SECS))
}

/// 上次息屏时间（本程序记的，纯文本，读不到就是没有）
pub fn last_off_path() -> PathBuf {
    config::config_path().with_file_name("last_screen_off.txt")
}

pub fn last_off() -> Option<String> {
    std::fs::read_to_string(last_off_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn record_off() {
    let p = last_off_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 本地时间 + 好读的格式：这是给人看的，不是给机器比的
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = std::fs::write(p, now);
}

/// 发一次 SC_MONITORPOWER。返回是否至少有一个窗口收下了这条消息。
fn send_power(lparam: isize) -> bool {
    let mut out: usize = 0;
    // 广播给所有顶层窗口（explorer / 桌面），这是各家工具通用的做法；
    // 用 SendMessageTimeout 而不是 SendMessage，免得有卡死的窗口把我们也拖住
    let r = unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SYSCOMMAND,
            SC_MONITORPOWER,
            lparam,
            SMTO_ABORTIFHUNG,
            1000,
            &mut out,
        )
    };
    if r != 0 {
        return true;
    }
    // 有些系统上广播没人接，退一步直接发给前台窗口
    let fg = unsafe { GetForegroundWindow() };
    if fg != 0 {
        let r2 = unsafe {
            SendMessageTimeoutW(
                fg,
                WM_SYSCOMMAND,
                SC_MONITORPOWER,
                lparam,
                SMTO_ABORTIFHUNG,
                1000,
                &mut out,
            )
        };
        return r2 != 0;
    }
    false
}

/// 给光标来一下「真实」的抖动：+N 再 -N，人眼看不出来，但驱动会当成硬件输入。
/// `mouse_event` 合成的移动本身不一定会点亮显示器，所以这里只是补一刀。
fn nudge_cursor() {
    let mut p = POINT::default();
    let ok = unsafe { GetCursorPos(&mut p) } != 0;
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    // 贴着右边缘时往左抖，免得 +2 被夹掉、回退时反而一路向左漂
    let dx = if ok && w > 0 && p.x >= w - 4 { -2 } else { 2 };
    unsafe {
        mouse_event(MOUSEEVENTF_MOVE, dx, 0, 0, 0);
        std::thread::sleep(Duration::from_millis(25));
        mouse_event(MOUSEEVENTF_MOVE, -dx, 0, 0, 0);
    }
}

/// 立刻关屏（不锁屏 —— 这是用户明确要的行为：只黑屏，不动会话）
pub fn off() -> Result<(), String> {
    // 给合上盖子/远程桌面这类不支持关屏的环境一点预期：没发出去就如实报错
    if !send_power(MONITOR_OFF) {
        return Err("关屏消息没有窗口接收（远程桌面 / 虚拟显示环境常见），显示器未被关闭".into());
    }
    record_off();
    // 一声轻提示，确认命令真的按下去了（关屏后看不到界面，靠声音确认）
    unsafe { MessageBeep(0) };
    Ok(())
}

/// 叫醒显示器。
///
/// 只有真把显示器电源关过（[`off`]）才需要它。三条路一起走，因为单独任何一条都可能不灵：
///   1. `SetThreadExecutionState(ES_DISPLAY_REQUIRED)` —— 重置显示器空闲计时器，
///      这是系统认可的「我要屏幕亮着」；
///   2. 广播 `SC_MONITORPOWER / -1` —— 关屏那条路的反向操作；
///   3. 光标 ±2px 抖动 —— 有些驱动只认硬件级的输入事件。
///
/// 注意：这套并不能保证一定叫得醒（取决于显卡驱动 / 电源管理），
/// 所以界面侧默认 **不关** 显示器电源，见 [`power_off`]。
pub fn wake() {
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED);
    }
    send_power(MONITOR_ON);
    nudge_cursor();
    // 让它先真的亮起来，再把「要求亮屏」的占用还回去
    std::thread::sleep(Duration::from_millis(120));
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS);
    }
}

// ---------------------------------------------------------------------------
// 息屏时的鼠标指针
// ---------------------------------------------------------------------------

/// 我们自己把系统光标计数压下去了几次。
/// `ShowCursor` 是引用计数式的，只调一次未必压得过别处留下的正计数，
/// 所以藏的时候一路减到负数，记下减了几次，亮屏时原样加回去。
static CURSOR_HIDDEN: AtomicI32 = AtomicI32::new(0);

/// 藏起鼠标指针（息屏时用）。
///
/// 必须在窗口线程上调：`ShowCursor` 的光标归当前线程的窗口管。
/// egui/winit 的 `CursorVisible(false)` 单发一次不可靠 —— 后续帧里
/// 光标图标一刷新就可能被 `ShowCursor(TRUE)` 顶回来，这里直接压计数。
pub fn hide_cursor() {
    let mut n = 0;
    // 上限兜底：计数万一被别的软件顶乱了也别在这儿转死
    for _ in 0..16 {
        n += 1;
        if unsafe { ShowCursor(0) } < 0 {
            break;
        }
    }
    CURSOR_HIDDEN.fetch_add(n, Ordering::SeqCst);
}

/// 退出息屏时把指针还回来：只恢复我们自己压下去的次数，不动别处的计数
pub fn show_cursor() {
    let n = CURSOR_HIDDEN.swap(0, Ordering::SeqCst);
    for _ in 0..n {
        unsafe { ShowCursor(1) };
    }
}

/// 只是探测：不关屏，看看消息通道通不通（给 `--dry-run` 与状态页用）
pub fn probe() -> bool {
    let mut out: usize = 0;
    let r = unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SYSCOMMAND,
            SC_MONITORPOWER,
            -1, // -1 = 强制「开」，不改变现状，只是探路
            SMTO_ABORTIFHUNG,
            300,
            &mut out,
        )
    };
    r != 0
}

// ---------------------------------------------------------------------------
// 全局输入监听：息屏期间「怎么退出来」全靠它
// ---------------------------------------------------------------------------

/// 是「什么」把人叫回来的
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    /// 按了设定的退出键
    Key,
    /// 鼠标动了（可关）
    Mouse,
    /// 按了鼠标键（左/右/中）
    Click,
}

impl Hit {
    pub fn label(self) -> &'static str {
        match self {
            Hit::Key => "退出键",
            Hit::Mouse => "鼠标移动",
            Hit::Click => "鼠标点击",
        }
    }
}

const HIT_NONE: u8 = 0;
const HIT_KEY: u8 = 1;
const HIT_MOUSE: u8 = 2;
const HIT_CLICK: u8 = 3;

// 监听状态放静态里：一个进程同时只会有一次息屏，钩子回调也只能是无捕获函数，
// 拿不到 Arc（低级钩子是系统在我们的线程里直接调回来的）。
static W_HIT: AtomicU8 = AtomicU8::new(HIT_NONE);
static W_STOP: AtomicBool = AtomicBool::new(false);
static W_ALIVE: AtomicBool = AtomicBool::new(false);
/// 退出键的虚拟键码，-1 = 任意键
static W_EXIT_VK: AtomicI32 = AtomicI32::new(0x1B);
/// 是否吞掉退出键以外的按键
static W_BLOCK: AtomicBool = AtomicBool::new(false);

fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

/// 自上次调用以来是否按过（快按快放也不会漏）
fn key_tapped(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x0001) != 0 }
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) };
    p
}

/// 低级键盘钩子：**一直挂着**（不挂钩子就只能靠轮询，而轮询的低位「按下过」标记
/// 并不可靠），退出键记一次命中并吞掉，别的键只在「屏蔽键盘」打开时吞。
extern "system" fn kbd_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let msg = wparam as u32;
    if code >= 0 && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && lparam != 0 {
        let vk = unsafe { (*(lparam as *const KbdHook)).vk as i32 };
        let want = W_EXIT_VK.load(Ordering::Relaxed);
        let is_exit = if want < 0 {
            (0x08..=0xFE).contains(&vk)
        } else {
            vk == want
        };
        if is_exit {
            W_HIT.store(HIT_KEY, Ordering::SeqCst);
            // 息屏期间的退出键也不该漏进别的窗口：按下的那一下只归我们
            return 1;
        } else if W_BLOCK.load(Ordering::Relaxed) {
            return 1; // 吞掉
        }
    }
    unsafe { CallNextHookEx(0, code, wparam, lparam) }
}

/// 命中时立刻叫醒界面用的回调。
///
/// 界面那边不能指望「定时器每 100ms 一定会醒」—— 显示器关着的时候更是没准；
/// 所以命中就由监听线程主动去戳界面一下（egui 里就是 `ctx.request_repaint()`）。
pub type OnHit = Arc<dyn Fn() + Send + Sync>;

fn watch_loop(key: ExitKey, mouse: bool, block: bool, on_hit: Option<OnHit>) {
    W_ALIVE.store(true, Ordering::SeqCst);
    W_EXIT_VK.store(key.vk(), Ordering::SeqCst);
    W_BLOCK.store(block, Ordering::SeqCst);

    // 钩子一直挂：它是唯一不依赖窗口状态、稳定收到按键的路子；
    // 挂不上也不影响退出（还有下面的轮询兜底）
    let hook: HHOOK = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(kbd_proc), 0, 0) };

    // 进入息屏前就按着的键不算数（点「息屏」那一下鼠标就是按着的），等松手再武装
    let mut arming: Vec<i32> = (VK_LBUTTON..=0xFE).filter(|&vk| key_down(vk)).collect();
    let mut origin = cursor();
    // 武装完成的时刻；在那之后的一小段时间里只跟着光标挪基准点（见 MOUSE_GRACE）
    let mut armed_at: Option<Instant> = None;

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // 命中后由监听线程自己先把显示器叫醒：界面万一卡了一帧，也不至于屏幕还黑着
    let mut woke = false;
    while !W_STOP.load(Ordering::SeqCst) {
        // 挂了钩子就必须泵消息，否则系统会把整机输入卡在我们这儿
        if hook != 0 {
            unsafe {
                while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        if !arming.is_empty() {
            arming.retain(|&vk| key_down(vk));
            if arming.is_empty() {
                origin = cursor();
                armed_at = Some(Instant::now());
            }
        } else {
            let vk = W_EXIT_VK.load(Ordering::Relaxed);
            let key_hit = if vk < 0 {
                (0x08..=0xFE).any(key_tapped)
            } else {
                key_tapped(vk)
            };
            if key_hit {
                W_HIT.store(HIT_KEY, Ordering::SeqCst);
            }
            if key_tapped(VK_LBUTTON) || key_tapped(VK_RBUTTON) || key_tapped(VK_MBUTTON) {
                W_HIT.store(HIT_CLICK, Ordering::SeqCst);
            }
            if mouse {
                let p = cursor();
                if armed_at.map(|t| t.elapsed() < MOUSE_GRACE).unwrap_or(true) {
                    origin = p;
                } else if (p.x - origin.x).abs() + (p.y - origin.y).abs() > 3 {
                    W_HIT.store(HIT_MOUSE, Ordering::SeqCst);
                }
            }
        }
        // 钩子回调里不能做重活（会卡住整机输入），叫醒放到这里统一做
        if !woke && W_HIT.load(Ordering::SeqCst) != HIT_NONE {
            woke = true;
            // 先戳界面（让它马上处理退出），再把显示器叫醒
            if let Some(cb) = &on_hit {
                cb();
            }
            wake();
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    if hook != 0 {
        unsafe { UnhookWindowsHookEx(hook) };
    }
    W_BLOCK.store(false, Ordering::SeqCst);
    W_ALIVE.store(false, Ordering::SeqCst);
}

/// 息屏期间的后台监听。
///
/// `start` 之后每帧问一次 [`Watch::hit`]；命中就用 [`Watch::stop`] 收摊。
/// 线程自己挂了（`alive()` 为 false）说明「再也收不到退出了」，
/// 调用方应当立刻退出息屏，别把人关在黑屏里。
pub struct Watch {
    th: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// `mouse` = 鼠标移动也算退出；`block` = 息屏期间屏蔽键盘（只留退出键）。
    /// `on_hit` 会在命中那一瞬被监听线程调用（界面用它立刻重画，别等定时器）。
    pub fn start(
        key: ExitKey,
        mouse: bool,
        block: bool,
        on_hit: Option<OnHit>,
    ) -> Result<Watch, String> {
        W_HIT.store(HIT_NONE, Ordering::SeqCst);
        W_STOP.store(false, Ordering::SeqCst);
        // 先认「活着」：界面第一帧就会查 alive()，不能因为线程晚一步进循环
        // 就被判成「监听没了」而立刻退出息屏
        W_ALIVE.store(true, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        // 用一个「自己会退出」的线程，避免 join 卡死界面
        let th = match std::thread::Builder::new()
            .name("deskkit-blackout-watch".into())
            .spawn(move || {
                watch_loop(key, mouse, block, on_hit);
                flag.store(true, Ordering::SeqCst);
            }) {
            Ok(th) => th,
            Err(e) => {
                W_ALIVE.store(false, Ordering::SeqCst);
                return Err(format!("监听线程起不来: {e}"));
            }
        };
        Ok(Watch {
            th: Some(th),
            stop,
        })
    }

    /// 有没有人按了退出（一次就够，命中后一直返回同一个原因）
    pub fn hit(&self) -> Option<Hit> {
        match W_HIT.load(Ordering::SeqCst) {
            HIT_KEY => Some(Hit::Key),
            HIT_MOUSE => Some(Hit::Mouse),
            HIT_CLICK => Some(Hit::Click),
            _ => None,
        }
    }

    /// 监听线程是否还活着
    pub fn alive(&self) -> bool {
        !self.stop.load(Ordering::SeqCst) && W_ALIVE.load(Ordering::SeqCst)
    }

    pub fn stop(&mut self) {
        W_STOP.store(true, Ordering::SeqCst);
        if let Some(th) = self.th.take() {
            let _ = th.join();
        }
        W_BLOCK.store(false, Ordering::SeqCst);
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 界面自检要跳过「真关屏」时用
pub fn blackout_test_mode() -> bool {
    std::env::var("DESKKIT_BLACKOUT_TEST").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_text_forms() {
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("1:05:09"), Some(3909));
        assert_eq!(parse_duration("330"), Some(330));
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("随便写点啥"), None);
    }

    #[test]
    fn hms_pads_only_minutes_and_seconds() {
        assert_eq!(fmt_hms(3909), "1:05:09");
        assert_eq!(fmt_hms(59), "0:00:59");
    }

    #[test]
    fn exit_key_roundtrip() {
        for k in ExitKey::all() {
            assert_eq!(ExitKey::parse(k.as_str()), Some(k));
            assert!(k.vk() != 0);
        }
        // 任意键用 -1 表示，不能和真实键码撞
        assert_eq!(ExitKey::Any.vk(), -1);
    }

    #[test]
    fn watch_thread_comes_up_and_stops() {
        // 界面那边就靠这个线程收退出键，起不来或停不下来都会把人关在黑屏里
        let mut w = Watch::start(ExitKey::Esc, true, false, None).expect("监听线程应当能起来");
        assert!(w.alive(), "刚起来就该是活的");
        assert!(w.hit().is_none(), "没人按键就不该报退出");
        w.stop();
        assert!(!w.alive(), "停掉之后不该还报活");
    }
}
