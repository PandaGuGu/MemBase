//! 息屏（Linux / X11）：DPMS 关显示器 + 全局按键/鼠标监听。
//!
//! 与 Windows 版的差别：Linux 桌面没有 `SC_MONITORPOWER` 广播，
//! 关屏走 `xset dpms force off`（X11 标准扩展，Xfce/GNOME/KDE 都支持）；
//! 界面上的全屏黑窗由 egui 自己画，这一层只负责电源和「怎么退出来」。

use membase_core::config;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub fn monitor_count() -> u32 {
    let out = std::process::Command::new("xrandr")
        .arg("--listmonitors")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string());
    if let Some(text) = out {
        if let Some(n) = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u32>().ok())
        {
            return n.max(1);
        }
    }
    1
}

pub const DEFAULT_SECS: u64 = 60 * 60;
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

pub fn default_secs() -> u64 {
    if let Ok(t) = std::fs::read_to_string(cfg_path("screen_off_seconds.txt")) {
        if let Ok(n) = t.trim().parse::<u64>() {
            return n.min(MAX_SECS);
        }
    }
    DEFAULT_SECS
}

pub fn set_default_secs(n: u64) -> Result<(), String> {
    write_cfg("screen_off_seconds.txt", &n.min(MAX_SECS).to_string())
}

pub fn mouse_exit() -> bool {
    read_bool("screen_mouse_exit.txt", true)
}

pub fn set_mouse_exit(v: bool) -> Result<(), String> {
    write_bool("screen_mouse_exit.txt", v)
}

pub fn block_keys() -> bool {
    read_bool("screen_block_keys.txt", false)
}

pub fn set_block_keys(v: bool) -> Result<(), String> {
    write_bool("screen_block_keys.txt", v)
}

/// 息屏时是否真的关显示器（DPMS）。默认关：只画全屏黑窗，一定退得出来。
pub fn power_off() -> bool {
    read_bool("screen_power_off.txt", false)
}

pub fn set_power_off(v: bool) -> Result<(), String> {
    write_bool("screen_power_off.txt", v)
}

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
            "esc" | "escape" => Some(ExitKey::Esc),
            "space" | "空格" => Some(ExitKey::Space),
            "enter" | "return" | "回车" => Some(ExitKey::Enter),
            "any" | "任意" | "任意键" => Some(ExitKey::Any),
            _ => None,
        }
    }
    pub fn all() -> [ExitKey; 4] {
        [ExitKey::Esc, ExitKey::Space, ExitKey::Enter, ExitKey::Any]
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

pub fn fmt_hms(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

/// `3600` / `90s` / `5m` / `1h` / `1:05:09` / `0`
pub fn parse_duration(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    if t.contains(':') {
        let parts: Vec<&str> = t.split(':').collect();
        let mut secs = 0u64;
        for p in &parts {
            secs = secs * 60 + p.trim().parse::<u64>().ok()?;
        }
        return Some(secs.min(MAX_SECS));
    }
    let (num, mult) = if let Some(v) = t.strip_suffix("ms") {
        (v, 0.001)
    } else if let Some(v) = t.strip_suffix('s') {
        (v, 1.0)
    } else if let Some(v) = t.strip_suffix('m') {
        (v, 60.0)
    } else if let Some(v) = t.strip_suffix('h') {
        (v, 3600.0)
    } else {
        (t.as_str(), 1.0)
    };
    let n: f64 = num.trim().parse().ok()?;
    Some(((n * mult) as u64).min(MAX_SECS))
}

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
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = std::fs::write(p, now);
}

fn xset(args: &[&str]) -> bool {
    std::process::Command::new("xset")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 立刻关屏（DPMS force off）。不锁屏、不挂起。
pub fn off() -> Result<(), String> {
    if std::env::var("DISPLAY")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err("没有 DISPLAY（X11 会话才支持关屏）".into());
    }
    if !xset(&["dpms", "force", "off"]) {
        return Err("xset dpms force off 失败（显示器可能不支持 DPMS）".into());
    }
    record_off();
    Ok(())
}

/// Linux 下 DPMS 关屏不会让系统待机，保持唤醒是空操作；
/// 保留这个接口是为了让上层代码完全一致。
pub fn hold_awake(_on: bool) {}

pub fn wake_light() {
    let _ = xset(&["dpms", "force", "on"]);
}

pub fn wake() {
    let _ = xset(&["dpms", "force", "on"]);
    // 补一刀「模拟输入」：个别驱动只认硬件事件
    let _ = std::process::Command::new("xdotool")
        .args(["mousemove_relative", "--", "1", "0"])
        .status();
    std::thread::sleep(Duration::from_millis(300));
}

pub fn hide_cursor() {}

pub fn show_cursor() {}

/// 关屏通道是否可用：有 DISPLAY 且 xset 能执行
pub fn probe() -> bool {
    if std::env::var("DISPLAY")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return false;
    }
    xset(&["q"])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Key,
    Mouse,
    Click,
    Resume,
}

impl Hit {
    pub fn label(self) -> &'static str {
        match self {
            Hit::Key => "退出键",
            Hit::Mouse => "鼠标移动",
            Hit::Click => "鼠标点击",
            Hit::Resume => "系统从待机恢复",
        }
    }
}

const HIT_NONE: u8 = 0;
const HIT_KEY: u8 = 1;
const HIT_MOUSE: u8 = 2;
const HIT_CLICK: u8 = 3;
const HIT_RESUME: u8 = 4;

pub type OnHit = Arc<dyn Fn() + Send + Sync>;

static W_HIT: AtomicU8 = AtomicU8::new(HIT_NONE);
static W_STOP: AtomicBool = AtomicBool::new(false);
static W_ALIVE: AtomicBool = AtomicBool::new(false);

fn point(root: &RustConnection, root_win: Window) -> Option<(i16, i16, u16)> {
    let r = x11rb::protocol::xproto::query_pointer(root, root_win)
        .ok()?
        .reply()
        .ok()?;
    Some((r.root_x, r.root_y, r.mask.into()))
}

fn find_keycodes(conn: &RustConnection) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let count = setup.max_keycode - min + 1;
    let Ok(reply) = conn
        .get_keyboard_mapping(min, count)
        .ok()
        .and_then(|c| c.reply().ok())
        .ok_or(())
    else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let per = reply.keysyms_per_keycode as usize;
    let (mut esc, mut space, mut enter) = (Vec::new(), Vec::new(), Vec::new());
    for (i, chunk) in reply.keysyms.chunks(per.max(1)).enumerate() {
        let keycode = (min as usize + i) as u8;
        for ks in chunk {
            match *ks {
                0xff1b => esc.push(keycode),
                0x0020 => space.push(keycode),
                0xff0d => enter.push(keycode),
                _ => {}
            }
        }
    }
    (esc, space, enter)
}

fn watch_loop(key: ExitKey, mouse: bool, block: bool, on_hit: Option<OnHit>) {
    let Ok((conn, screen_num)) = RustConnection::connect(None) else {
        W_ALIVE.store(false, Ordering::SeqCst);
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let mut grabbed = false;
    if block {
        grabbed = conn
            .grab_keyboard(
                true,
                root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .ok()
            .map(|_| true)
            .unwrap_or(false);
    }
    let (esc, space, enter) = find_keycodes(&conn);
    let start = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_pos = point(&conn, root).map(|(x, y, m)| (x, y, m));

    while !W_STOP.load(Ordering::SeqCst) {
        // 退出键：从 X11 事件队列里看 KeyPress
        while let Ok(Some(ev)) = conn.poll_for_event() {
            if let x11rb::protocol::Event::KeyPress(k) = ev {
                let ok = match key {
                    ExitKey::Any => true,
                    ExitKey::Esc => esc.contains(&k.detail),
                    ExitKey::Space => space.contains(&k.detail),
                    ExitKey::Enter => enter.contains(&k.detail),
                };
                if ok {
                    W_HIT.store(HIT_KEY, Ordering::SeqCst);
                    if let Some(cb) = &on_hit {
                        cb();
                    }
                    break;
                }
            }
        }
        if W_HIT.load(Ordering::SeqCst) == HIT_KEY {
            break;
        }
        if mouse {
            if let Some((x, y, mask)) = point(&conn, root) {
                if let Some((px, py, pmask)) = last_pos {
                    if (x as i32 - px as i32).abs() > 2 || (y as i32 - py as i32).abs() > 2 {
                        W_HIT.store(HIT_MOUSE, Ordering::SeqCst);
                    } else if (mask & 0x1f00) != (pmask & 0x1f00) {
                        W_HIT.store(HIT_CLICK, Ordering::SeqCst);
                    }
                }
                last_pos = Some((x, y, mask));
            }
        }
        if W_HIT.load(Ordering::SeqCst) != HIT_NONE {
            if let Some(cb) = &on_hit {
                cb();
            }
            break;
        }
        // 机器睡过一轮：两轮间隔远超轮询周期
        let now = Instant::now();
        if now.duration_since(last_tick) > Duration::from_secs(3)
            && start.elapsed() > Duration::from_secs(3)
        {
            W_HIT.store(HIT_RESUME, Ordering::SeqCst);
            if let Some(cb) = &on_hit {
                cb();
            }
            break;
        }
        last_tick = now;
        std::thread::sleep(Duration::from_millis(20));
    }
    if grabbed {
        let _ = conn.ungrab_keyboard(x11rb::CURRENT_TIME);
    }
    W_ALIVE.store(false, Ordering::SeqCst);
}

/// 息屏期间的后台监听：按键 / 鼠标 / 待机恢复。
pub struct Watch {
    th: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Watch {
    pub fn start(
        key: ExitKey,
        mouse: bool,
        block: bool,
        on_hit: Option<OnHit>,
    ) -> Result<Watch, String> {
        W_HIT.store(HIT_NONE, Ordering::SeqCst);
        W_STOP.store(false, Ordering::SeqCst);
        W_ALIVE.store(true, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let th = std::thread::Builder::new()
            .name("membase-blackout-watch".into())
            .spawn(move || {
                watch_loop(key, mouse, block, on_hit);
                flag.store(true, Ordering::SeqCst);
            })
            .map_err(|e| {
                W_ALIVE.store(false, Ordering::SeqCst);
                format!("监听线程起不来: {e}")
            })?;
        Ok(Watch { th: Some(th), stop })
    }

    pub fn hit(&self) -> Option<Hit> {
        match W_HIT.load(Ordering::SeqCst) {
            HIT_KEY => Some(Hit::Key),
            HIT_MOUSE => Some(Hit::Mouse),
            HIT_CLICK => Some(Hit::Click),
            HIT_RESUME => Some(Hit::Resume),
            _ => None,
        }
    }

    pub fn alive(&self) -> bool {
        !self.stop.load(Ordering::SeqCst) && W_ALIVE.load(Ordering::SeqCst)
    }

    pub fn stop(&mut self) {
        W_STOP.store(true, Ordering::SeqCst);
        if let Some(th) = self.th.take() {
            let _ = th.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn blackout_test_mode() -> bool {
    std::env::var("MEMBASE_BLACKOUT_TEST").is_ok()
}
