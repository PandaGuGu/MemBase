//! MemBase GUI —— 内存基准线 · 桌面图标布局（同一套界面）
//!
//! 界面上每个按钮都有一条等价命令行（见 deskkit-cli 的 ACTIONS 表）：
//! 按钮 hover 会直接把那条命令写出来，程序里也另有一页「命令行」做对照。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use deskkit_cli::layout::{self as cli_map, ACTIONS};
use deskkit_core as core;
use deskkit_core::layout::{IconPos, Layout, LayoutEntry, Plan};
use deskkit_core::layout_store as store;
use deskkit_core::{Kind, Tier};
use deskkit_win::gui_icon::{self, IconProvider};
use deskkit_win::task::{self as boot, TaskInfo};
use deskkit_win::desktop::Desktop;
use deskkit_win::{actions, close, repair, shellview::ShellView, snapshot, win};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

/// 语义调色板：浅色对齐 Windows 11 任务管理器，深色对齐它的夜间模式
#[derive(Clone, Copy)]
struct Palette {
    /// 窗口底 / 侧栏底
    bg: egui::Color32,
    /// 弹窗、输入框等浮层底
    panel: egui::Color32,
    /// 内容卡片底
    card: egui::Color32,
    /// 选中行 / 侧栏选中态
    card_hl: egui::Color32,
    /// 1px 分隔线
    line: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    accent: egui::Color32,
    /// 压在 accent 上的文字
    on_accent: egui::Color32,
    red: egui::Color32,
    amber: egui::Color32,
    green: egui::Color32,
    grey: egui::Color32,
    /// 行悬停底色
    row_hover: egui::Color32,
    /// 表头底色
    head_bg: egui::Color32,
    /// 侧栏项悬停底色
    sel_nav: egui::Color32,
    /// 按钮禁用底
    disabled_bg: egui::Color32,
    /// 输入框 / 文本域底色
    field_bg: egui::Color32,
    /// 文本选中底色
    sel: egui::Color32,
    /// 内存列热力色的基色（按用量叠透明度）
    heat: (u8, u8, u8),
}

const LIGHT: Palette = Palette {
    bg: egui::Color32::from_rgb(243, 243, 243),
    panel: egui::Color32::from_rgb(255, 255, 255),
    card: egui::Color32::from_rgb(255, 255, 255),
    card_hl: egui::Color32::from_rgb(232, 242, 252),
    line: egui::Color32::from_rgb(226, 226, 226),
    text: egui::Color32::from_rgb(27, 27, 27),
    muted: egui::Color32::from_rgb(96, 94, 92),
    accent: egui::Color32::from_rgb(0, 120, 212),
    on_accent: egui::Color32::from_rgb(255, 255, 255),
    red: egui::Color32::from_rgb(196, 43, 28),
    amber: egui::Color32::from_rgb(157, 93, 0),
    green: egui::Color32::from_rgb(16, 124, 16),
    grey: egui::Color32::from_rgb(138, 138, 138),
    row_hover: egui::Color32::from_rgb(245, 245, 245),
    head_bg: egui::Color32::from_rgb(250, 250, 250),
    sel_nav: egui::Color32::from_rgb(232, 232, 232),
    disabled_bg: egui::Color32::from_rgb(240, 240, 240),
    field_bg: egui::Color32::from_rgb(255, 255, 255),
    sel: egui::Color32::from_rgb(204, 228, 247),
    heat: (199, 224, 244),
};

const DARK: Palette = Palette {
    bg: egui::Color32::from_rgb(32, 32, 32),
    panel: egui::Color32::from_rgb(39, 39, 39),
    card: egui::Color32::from_rgb(39, 39, 39),
    card_hl: egui::Color32::from_rgb(40, 58, 78),
    line: egui::Color32::from_rgb(58, 58, 58),
    text: egui::Color32::from_rgb(240, 240, 240),
    muted: egui::Color32::from_rgb(170, 170, 170),
    accent: egui::Color32::from_rgb(76, 194, 255),
    on_accent: egui::Color32::from_rgb(16, 16, 16),
    red: egui::Color32::from_rgb(255, 110, 100),
    amber: egui::Color32::from_rgb(240, 180, 80),
    green: egui::Color32::from_rgb(110, 200, 110),
    grey: egui::Color32::from_rgb(150, 150, 150),
    row_hover: egui::Color32::from_rgb(48, 48, 48),
    head_bg: egui::Color32::from_rgb(44, 44, 44),
    sel_nav: egui::Color32::from_rgb(52, 52, 52),
    disabled_bg: egui::Color32::from_rgb(48, 48, 48),
    field_bg: egui::Color32::from_rgb(45, 45, 45),
    sel: egui::Color32::from_rgb(38, 74, 106),
    heat: (30, 90, 140),
};

static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dark_mode() -> bool {
    DARK_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

fn set_dark_mode(on: bool) {
    DARK_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// 当前生效的调色板（每帧读，切主题后立刻生效）
fn pal() -> Palette {
    if dark_mode() {
        DARK
    } else {
        LIGHT
    }
}

/// 主题偏好落在 %APPDATA%\MemBase\theme.txt，读写都走 cli crate（与 `deskkit theme` 同源）
fn load_theme() -> bool {
    deskkit_cli::read_theme()
}

struct Row {
    kind: Kind,
    name: String,
    /// (PID, 创建时间)：执行前用它确认 PID 没被系统复用
    members: Vec<core::Instance>,
    instances: u32,
    mem_mb: f64,
    path: String,
    tier: Tier,
    reason: String,
    /// 记进保护名单：永不处理
    protect: bool,
    /// 命中的保护条目（`进程:MuMu*` 这类通配也能命中）
    protect_by: String,
    /// 本次要恢复（终止/停止）—— 行右侧的勾选框
    pick: bool,
}

/// 一次「读基准 + 枚举进程/服务 + 算线外清单」的结果。
/// 纯数据：启动时丢给后台线程算，算完再换进界面。
struct Scan {
    base_line: String,
    base_meta: String,
    rows: Vec<Row>,
    skipped: Vec<Row>,
    collect_ms: Option<u64>,
    collect_from_base: bool,
    /// 找不到基准时的提示（有值才写进状态栏）
    missing: Option<String>,
}

/// 左侧导航的页面
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Processes,
    Baseline,
    Layouts,
    Protect,
    Boot,
    Tools,
    Cli,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Processes => "进程",
            Page::Baseline => "基准",
            Page::Layouts => "桌面布局",
            Page::Protect => "保护名单",
            Page::Boot => "开机采集",
            Page::Tools => "工具",
            Page::Cli => "命令行",
            Page::Settings => "设置",
        }
    }
}

/// 息屏模式的状态：全屏纯黑，ESC / 任意键 / 点击退出，到时间自动亮
struct Blackout {
    /// 自动退出的时间点（None = 不自动退出，等用户按键）
    until: Option<f64>,
    /// 提示文字显示到什么时候（前 3 秒显示「按 ESC 退出」）
    hint_until: f64,
    /// 进入时刻：用来忽略「点按钮那一下」的鼠标事件
    entered: std::time::Instant,
    /// 这次息屏有没有真去关显示器电源（关了退出时才需要叫醒）
    powered: bool,
}

/// 桌面布局页的弹窗（删除走单独的 confirm_delete，因为它要显示图标数）
enum LayoutDialog {
    Save { name: String },
    Repair,
}

struct App {
    root: PathBuf,
    base_line: String,
    base_meta: String,
    rows: Vec<Row>,
    skipped: Vec<Row>,
    status: String,
    search: String,
    include_caution: bool,
    confirm: bool,
    confirm_capture: bool,
    confirm_boot: bool,
    /// 开机第几秒采集，对应计划任务里的 --at
    boot_at: u64,
    /// 最近一次采集实际花了多少毫秒（来自基准的 system.json）
    collect_ms: Option<u64>,
    /// 上面这个数是基准里记录的（true）还是本次刷新时实测的（false）
    collect_from_base: bool,
    boot: TaskInfo,
    is_admin: bool,
    protect_file: PathBuf,
    frames: u32,
    shot_path: Option<String>,
    /// 启动时后台扫描的结果接收端（None = 已经拿到或还没开始）
    scan_rx: Option<mpsc::Receiver<Scan>>,
    /// 已经按哪个主题灌过 Visuals（None = 还没灌过）
    themed_for: Option<bool>,
    /// 当前页（左侧导航）
    page: Page,
    /// 深色主题（落在 %APPDATA%\MemBase\theme.txt）
    dark: bool,

    // ---------------- 桌面布局（原 DeskPos 的那一半） ----------------
    /// 已保存的布局，按时间倒序
    layouts: Vec<LayoutEntry>,
    /// 当前选中的布局下标
    sel: usize,
    /// 桌面实时统计：图标数 / 自动排列 / 分辨率 / shell 接口 / 布局缓存
    lay_icons: Option<u32>,
    lay_arrange: Option<bool>,
    lay_screen: String,
    lay_shell: bool,
    lay_cache: String,
    /// 预览里叠「这些图标现在在哪」的蓝色虚线
    show_ghost: bool,
    ghost: Vec<IconPos>,
    /// 选中布局与当前桌面的差异（缓存，避免每帧都去读 explorer 内存）
    plan_cache: Option<(String, Plan)>,
    /// 壁纸纹理（只解一次）
    wallpaper: Option<egui::TextureHandle>,
    wallpaper_tried: bool,
    /// 后台解壁纸的结果接收端
    wallpaper_rx: Option<mpsc::Receiver<Option<gui_icon::Rgba>>>,
    icons: IconProvider,
    icon_tex: HashMap<String, Option<egui::TextureHandle>>,
    /// 本帧给「从系统取图标」留的时间预算（None = 不限制，自检截图时用）
    icon_deadline: Option<std::time::Instant>,
    /// 这帧有图标因为预算用完还没取，下一帧要继续
    icons_pending: bool,
    dialog: Option<LayoutDialog>,
    /// 「命令行」页刚复制过哪一条（用于显示已复制）
    copied: Option<String>,
    /// 恢复布局前的确认弹窗
    confirm_restore: bool,
    /// 删除布局前的确认弹窗
    confirm_delete: bool,

    // ---------------- 工具页（息屏） ----------------
    /// 显示器台数
    scr_monitors: u32,
    /// 关屏消息通道是否可用（远程会话下可能不通）
    scr_channel: bool,
    /// 本程序上次发出的息屏时间
    scr_last: String,
    /// 正在息屏（全屏黑）
    blackout: Option<Blackout>,
    /// 息屏期间的全局输入监听：系统级查按键/光标，跟窗口焦点无关
    bo_watch: Option<deskkit_win::screen::Watch>,
    /// 默认息屏时长（秒，0 = 不自动退出）
    bo_secs: u64,
    /// 退出息屏的热键（默认 ESC）
    bo_key: deskkit_win::screen::ExitKey,
    /// 息屏时是否同时关显示器电源（默认否：只盖黑窗，最不容易退不出来）
    bo_power_off: bool,
    /// 自检模式：DESKKIT_BLACKOUT_TEST=1 时进息屏但不真关显示器（截图验证用）
    bo_test: bool,
}

/// RGBA 图像 → egui 纹理（图标与壁纸共用）
fn upload_texture(ctx: &egui::Context, name: &str, img: gui_icon::Rgba) -> egui::TextureHandle {
    let image =
        egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.data);
    ctx.load_texture(name.to_string(), image, egui::TextureOptions::LINEAR)
}

/// 调试用：把一帧渲染结果存成 BMP（无第三方依赖），用于验证界面
fn save_bmp(img: &egui::ColorImage, path: &str) -> std::io::Result<()> {
    let w = img.size[0];
    let h = img.size[1];
    let row_bytes = w * 4;
    let file_size = 54 + row_bytes * h;
    let mut out: Vec<u8> = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((row_bytes * h) as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..h).rev() {
        for x in 0..w {
            let c = img.pixels[y * w + x];
            out.push(c.b());
            out.push(c.g());
            out.push(c.r());
            out.push(255);
        }
    }
    std::fs::write(path, out)
}

fn json_num(text: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\"", key);
    let i = text.find(&pat)?;
    let rest = text[i + pat.len()..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| c == ',' || c == '\n' || c == '}')
        .unwrap_or(rest.len());
    rest[..end].trim().trim_matches('"').parse().ok()
}

/// 路径太长时中间省略，避免把工具栏撑爆
fn elide(s: &str, max_chars: usize) -> String {
    // 不先收集成 Vec<char>：表格每帧都要调，省一次分配
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut = s
        .char_indices()
        .nth(max_chars.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let mut out = String::with_capacity(cut + 3);
    out.push_str(&s[..cut]);
    out.push('…');
    out
}

/// 这一行是否允许被勾选处理：已保护 / 禁止处理 / 没放开的系统组件都不算
fn can_pick(r: &Row, include_caution: bool) -> bool {
    if r.protect || r.tier == Tier::Blocked {
        return false;
    }
    if r.tier == Tier::Caution && !include_caution {
        return false;
    }
    true
}

fn fmt_mb(mb: f64) -> String {
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

/// 采集耗时：小于 1 秒就报毫秒，免得全显示成「0.0 秒」
fn fmt_dur(ms: u64) -> String {
    if ms < 1000 {
        format!("{} 毫秒", ms)
    } else {
        format!("{:.1} 秒", ms as f64 / 1000.0)
    }
}

/// 说「多久之后完成」：不足 1 秒就不硬凑成「第 61 秒」
fn finish_hint(ms: u64) -> String {
    if ms < 1000 {
        "不到 1 秒就".to_string()
    } else {
        format!("约 {:.1} 秒后", ms as f64 / 1000.0)
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let dark = load_theme();
        set_dark_mode(dark);
        let root = PathBuf::from(deskkit_core::config::data_root(None));
        let protect_file = root.join("protect.txt");
        let boot = boot::query(core::DEFAULT_TASK);
        // 已经安排过就沿用当时的秒数，免得重新注册时手滑改掉
        let boot_at = if boot.installed && boot.at_sec >= 5 {
            boot.at_sec
        } else {
            60
        };
        let mut app = App {
            root,
            base_line: String::from("（没有找到基准快照）"),
            base_meta: String::new(),
            rows: Vec::new(),
            skipped: Vec::new(),
            status: String::new(),
            search: String::new(),
            include_caution: false,
            confirm: false,
            confirm_capture: false,
            confirm_boot: false,
            boot_at,
            collect_ms: None,
            collect_from_base: false,
            boot,
            is_admin: win::is_admin(),
            protect_file,
            frames: 0,
            shot_path: std::env::var("DESKKIT_SHOT").ok(),
            scan_rx: None,
            themed_for: None,
            page: Page::Processes,
            dark,
            layouts: Vec::new(),
            sel: 0,
            lay_icons: None,
            lay_arrange: None,
            lay_screen: String::new(),
            lay_shell: false,
            lay_cache: String::new(),
            show_ghost: false,
            ghost: Vec::new(),
            plan_cache: None,
            wallpaper: None,
            wallpaper_tried: false,
            wallpaper_rx: None,
            icons: IconProvider::new(),
            icon_tex: HashMap::new(),
            icon_deadline: None,
            icons_pending: false,
            dialog: None,
            copied: None,
            confirm_restore: false,
            confirm_delete: false,
            scr_monitors: 0,
            scr_channel: false,
            scr_last: String::new(),
            blackout: None,
            bo_watch: None,
            bo_secs: deskkit_win::screen::default_secs(),
            bo_key: deskkit_win::screen::exit_key(),
            bo_power_off: deskkit_win::screen::power_off(),
            bo_test: std::env::var("DESKKIT_BLACKOUT_TEST").is_ok(),
        };
        // 屏幕分辨率很便宜，先取好
        let (sw, sh) = deskkit_win::sys::screen_size();
        app.lay_screen = format!("{}×{}", sw, sh);
        // 自检/截图：DESKKIT_PAGE=layouts|cli|... 直接开在指定页
        if let Ok(p) = std::env::var("DESKKIT_PAGE") {
            app.page = match p.to_lowercase().as_str() {
                "layouts" | "layout" | "desk" => Page::Layouts,
                "cli" | "commands" => Page::Cli,
                "baseline" => Page::Baseline,
                "protect" => Page::Protect,
                "boot" => Page::Boot,
                "tools" | "tool" => Page::Tools,
                "settings" => Page::Settings,
                _ => Page::Processes,
            };
        }
        // 读基准 + 枚举进程/服务要几百毫秒：丢到后台算，窗口先画出来
        let (tx, rx) = mpsc::channel();
        let (root, protect_file) = (app.root.clone(), app.protect_file.clone());
        std::thread::spawn(move || {
            let _ = tx.send(App::scan(&root, &protect_file));
        });
        app.scan_rx = Some(rx);
        // 布局清单与桌面状态很便宜（几十毫秒），而且顶栏一直在用，照旧启动时读
        app.reload_layouts();
        app.refresh_layout_status();
        app.refresh_screen_status();
        app
    }

    fn whitelist(&self) -> Vec<String> {
        std::fs::read_to_string(&self.protect_file)
            .map(|t| core::parse_whitelist(&t))
            .unwrap_or_default()
    }

    fn save_whitelist(&self) {
        // 只重写「当前列表覆盖到的条目」，其余原样保留：
        // 通配规则（进程:MuMu*）、当前没在跑的旧进程、别的机器带过来的项都不该被这次保存抹掉
        let mut items = self.whitelist();
        let shown: Vec<String> = self
            .rows
            .iter()
            .chain(self.skipped.iter())
            .map(|r| core::protect_key(r.kind, &r.name))
            .collect();
        items.retain(|w| !shown.contains(w));
        for r in self.rows.iter().chain(self.skipped.iter()) {
            if r.protect {
                items.push(core::protect_key(r.kind, &r.name));
            }
        }
        items.sort();
        items.dedup();
        let _ = std::fs::write(&self.protect_file, core::serialize_whitelist(&items));
    }

    /// 读基准 + 枚举进程/服务 + 算线外清单。纯数据、不碰 self，
    /// 所以启动时可以直接丢给后台线程跑，窗口不用等它。
    fn scan(root: &std::path::Path, protect_file: &std::path::Path) -> Scan {
        let mut out = Scan {
            base_line: "（没有找到基准快照）".into(),
            base_meta: String::new(),
            rows: Vec::new(),
            skipped: Vec::new(),
            collect_ms: None,
            collect_from_base: false,
            missing: None,
        };
        let Some(base_dir) = deskkit_win::compare::find_latest_baseline(root) else {
            out.missing = Some(format!(
                "没有基准快照，先跑一次 deskkit collect（{}）",
                root.display()
            ));
            return out;
        };

        let sysj = std::fs::read_to_string(base_dir.join("system.json")).unwrap_or_default();
        let bn = json_num(&sysj, "UptimeSeconds").unwrap_or(0.0);
        let used = json_num(&sysj, "UsedPhysicalMemoryMB").unwrap_or(0.0);
        let load = json_num(&sysj, "MemoryLoadPercent").unwrap_or(0.0);
        let captured = json_num(&sysj, "CapturedAtLocal").unwrap_or(0.0);
        let _ = captured;
        let collect_ms = json_num(&sysj, "CollectMs").unwrap_or(0.0);
        out.collect_ms = if collect_ms > 0.0 {
            Some(collect_ms as u64)
        } else {
            None
        };
        out.collect_from_base = out.collect_ms.is_some();
        let cap_text = extract_captured(&sysj);
        // 把来源目录名也显示出来：这几个数字全部读自它的 system.json，改文件界面就会跟着变
        let run_name = base_dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| base_dir.display().to_string());
        out.base_line = format!("基准快照：{}", cap_text);
        out.base_meta = format!(
            "采集于开机第 {} 秒 · 已用 {:.0} MB · 占用率 {:.0}% · {}",
            bn as u64, used, load, run_name
        );

        let base_procs = core::parse_by_name(
            &std::fs::read_to_string(base_dir.join("processes_by_name.csv")).unwrap_or_default(),
        );
        let base_svc = core::parse_service_states(
            &std::fs::read_to_string(base_dir.join("services.csv")).unwrap_or_default(),
        );

        let snap = snapshot::snapshot_now(true);
        // 老基准（PS 版或本版之前的快照）没记 CollectMs，就用这次刷新的实测值兜底
        if out.collect_ms.is_none() {
            out.collect_ms = Some(snap.collect_ms);
            out.collect_from_base = false;
        }
        let protected = win::protected_pids();
        let wl = std::fs::read_to_string(protect_file)
            .map(|t| core::parse_whitelist(&t))
            .unwrap_or_default();

        // 清单构建统一交给 membase-core：GUI 与 CLI 共用同一套判定，避免规则漂移
        let live_procs: Vec<core::LiveProc> = snap
            .procs
            .iter()
            .map(|p| core::LiveProc {
                name: p.name.clone(),
                pid: p.pid,
                ws_mb: p.ws_mb,
                priv_mb: p.priv_mb,
                path: p.path.clone(),
                created: p.created,
            })
            .collect();
        let live_svcs: Vec<core::LiveSvc> = snap
            .svcs
            .iter()
            .map(|s| core::LiveSvc {
                name: s.name.clone(),
                state: s.state.clone(),
                start_mode: s.start_mode.clone(),
                pid: s.pid,
                ws_mb: s.host_ws.unwrap_or(0.0),
                path: s.path.clone(),
            })
            .collect();

        let plan = core::build_plan(
            &base_procs,
            &base_svc,
            &live_procs,
            &live_svcs,
            &protected,
            &wl,
            0.0, // GUI 不按内存阈值过滤，交给搜索框
        );

        let mut rows: Vec<Row> = Vec::new();
        let mut skipped: Vec<Row> = Vec::new();
        for it in plan {
            let row = Row {
                kind: it.kind,
                name: it.name,
                members: it.members,
                instances: it.instances,
                mem_mb: it.mem_mb,
                path: it.path,
                tier: it.tier,
                reason: it.reason,
                protect: it.protect,
                protect_by: it.protect_by,
                // 建议处理项默认勾上；系统组件默认不勾，勾了「包含系统组件」才连带选上
                pick: it.tier == Tier::Suggested && !it.protect,
            };
            if row.tier == Tier::Blocked {
                skipped.push(row);
            } else {
                rows.push(row);
            }
        }

        rows.sort_by(|a, b| {
            let ta = match a.tier {
                Tier::Suggested => 0,
                Tier::Caution => 1,
                Tier::Blocked => 2,
            };
            let tb = match b.tier {
                Tier::Suggested => 0,
                Tier::Caution => 1,
                Tier::Blocked => 2,
            };
            ta.cmp(&tb).then(
                b.mem_mb
                    .partial_cmp(&a.mem_mb)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        out.rows = rows;
        out.skipped = skipped;
        out
    }

    /// 把一次扫描结果换进界面（后台线程算完、或手动刷新时用）
    fn apply_scan(&mut self, s: Scan) {
        self.base_line = s.base_line;
        self.base_meta = s.base_meta;
        self.rows = s.rows;
        self.skipped = s.skipped;
        self.collect_ms = s.collect_ms;
        self.collect_from_base = s.collect_from_base;
        if let Some(m) = s.missing {
            self.status = m;
        }
    }

    fn reload(&mut self) {
        // 手动刷新以本次结果为准：丢掉可能还在路上的启动扫描
        self.scan_rx = None;
        let s = App::scan(&self.root, &self.protect_file);
        self.apply_scan(s);
    }

    fn selected_count(&self) -> (usize, f64) {
        let inc = self.include_caution;
        self.rows
            .iter()
            .filter(|r| r.pick && can_pick(r, inc))
            .fold((0usize, 0.0f64), |(n, mb), r| (n + 1, mb + r.mem_mb))
    }

    /// (可勾选行数, 已勾选行数)
    fn pick_stats(&self) -> (usize, usize) {
        let inc = self.include_caution;
        self.rows
            .iter()
            .filter(|r| can_pick(r, inc))
            .fold((0usize, 0usize), |(all, on), r| {
                (all + 1, on + usize::from(r.pick))
            })
    }

    fn protected_count(&self) -> usize {
        self.rows.iter().filter(|r| r.protect).count()
    }

    fn set_all_pick(&mut self, on: bool) {
        let inc = self.include_caution;
        for r in self.rows.iter_mut() {
            if can_pick(r, inc) {
                r.pick = on;
            } else if r.protect {
                r.pick = false;
            }
        }
    }

    /// 一次写全名单；关掉时只把「原本被保护」的行恢复默认勾选，不动别人
    fn set_all_protect(&mut self, on: bool) {
        for r in self.rows.iter_mut() {
            if on {
                r.protect = true;
                r.pick = false;
            } else if r.protect {
                r.protect = false;
                r.pick = r.tier == Tier::Suggested;
            }
        }
        self.save_whitelist();
    }

    fn execute(&mut self) {
        let protected = win::protected_pids();
        let mut ok = 0usize;
        let mut fail = 0usize;
        let mut details: Vec<String> = Vec::new();

        // 进程组统一走「先礼后兵」：一次发 WM_CLOSE、共用一个宽限期，
        // 而不是每个勾选项各等 3 秒（否则 20 项要卡一分钟）。
        let mut proc_targets: Vec<(String, Vec<close::Target>)> = Vec::new();
        let mut svc_names: Vec<String> = Vec::new();
        for r in &self.rows {
            if !r.pick || !can_pick(r, self.include_caution) {
                continue;
            }
            match r.kind {
                Kind::Process => proc_targets.push((
                    r.name.clone(),
                    r.members
                        .iter()
                        .map(|m| close::Target {
                            pid: m.pid,
                            created: m.created,
                        })
                        .collect(),
                )),
                Kind::Service => svc_names.push(r.name.clone()),
            }
        }

        let views: Vec<close::Group> = proc_targets
            .iter()
            .map(|(n, t)| close::Group {
                name: n,
                targets: t,
            })
            .collect();
        let reports = close::close_groups(&views, close::CloseOptions::default(), &protected);
        let mut graceful = 0usize;
        let mut forced = 0usize;
        for (i, rep) in reports.iter().enumerate() {
            if rep.done() {
                ok += 1;
            } else {
                fail += 1;
            }
            graceful += rep.closed as usize;
            forced += rep.forced as usize;
            if rep.remaining > 0 || rep.skipped > 0 || rep.errors.len() > 0 {
                details.push(rep.one_line(&proc_targets[i].0));
            }
        }

        for name in &svc_names {
            match win::stop_service(name) {
                Ok(()) => ok += 1,
                Err(e) => {
                    fail += 1;
                    details.push(format!("{}: {}", name, e));
                }
            }
        }

        self.confirm = false;
        self.status = format!(
            "已执行：成功 {} 项，失败 {} 项（礼貌关闭 {} 个实例，强制 {} 个）",
            ok, fail, graceful, forced
        );
        if let Some(first) = details.first() {
            self.status.push_str(&format!("（{}）", first));
        }
        self.reload();
    }

    /// 让用户自己选数据目录，并写进 %APPDATA%\MemBase\config.json
    fn choose_dir(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("选择基准线数据目录")
            .pick_folder();
        let dir = match picked {
            Some(d) => d,
            None => {
                self.status = "已取消更改目录".into();
                return;
            }
        };
        let text = dir.to_string_lossy().to_string();
        let cfg = deskkit_core::config::Config {
            data_root: Some(text.clone()),
        };
        match deskkit_core::config::save(&cfg) {
            Ok(path) => {
                self.root = dir.clone();
                self.protect_file = dir.join("protect.txt");
                self.reload();
                self.status = if self.base_line.contains("没有找到") {
                    format!(
                        "数据目录已改为 {}（还没有基准，点「扫描当前 → 设为基准」建一份）",
                        text
                    )
                } else {
                    format!("数据目录已改为 {}，配置写入 {}", text, path.display())
                };
            }
            Err(e) => self.status = format!("保存配置失败：{e}"),
        }
    }

    /// 扫描当前状态，直接立成新基准（不用重启、不用等开机 60 秒）
    fn capture_now(&mut self) {        let snap = snapshot::snapshot_now(true);
        let run_dir = snapshot::new_run_dir(&self.root);
        let sub = run_dir.join("now");
        match snapshot::write_snapshot(&sub, &snap) {
            Ok(()) => {
                self.status = format!(
                    "已把当前状态设为基准：{} 进程 / {} MB / 占用率 {}% · 采集耗时 {}",
                    snap.procs.len(),
                    snap.used_phys_mb,
                    snap.load_pct,
                    fmt_dur(snap.collect_ms)
                );
                self.reload();
            }
            Err(e) => self.status = format!("设基准失败：{e}"),
        }
    }

    /// 注册开机采集任务：把「自己」拷到数据目录，再挂计划任务。
    /// 同一个 exe 带参数就是命令行版，不需要额外的 membase-cli.exe。
    fn install_boot(&mut self) {
        let src = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("取不到当前程序路径：{e}");
                return;
            }
        };
        match boot::install(&src, &self.root, self.boot_at, core::DEFAULT_TASK) {
            Ok(target) => {
                self.boot = boot::query(core::DEFAULT_TASK);
                self.status = format!(
                    "已安排：下次开机第 {} 秒采集一次（程序 {}）",
                    self.boot_at,
                    target.display()
                );
            }
            Err(e) => self.status = format!("安排失败：{}", e),
        }
    }

    fn remove_boot(&mut self) {
        let res = boot::uninstall(core::DEFAULT_TASK);
        self.boot = boot::query(core::DEFAULT_TASK);
        self.status = match res {
            Ok(()) => "已取消开机采集任务".to_string(),
            Err(e) => format!("取消失败：{}", e),
        };
    }

    fn export_baseline(&mut self) {
        let dir = match deskkit_win::compare::find_latest_baseline(&self.root) {
            Some(d) => d,
            None => {
                self.status = "没有基准可以导出".into();
                return;
            }
        };
        let captured =
            deskkit_core::bundle::snapshot_meta(&dir, "CapturedAtLocal").unwrap_or_default();
        let mut stamp = deskkit_core::bundle::stamp_from_local(&captured);
        if stamp.is_empty() {
            stamp = deskkit_core::bundle::timestamp();
        }
        let picked = rfd::FileDialog::new()
            .set_title("导出基准线")
            .set_file_name(&format!("baseline_{}.mbz", stamp))
            .add_filter("MemBase 基准线包", &["mbz"])
            .save_file();
        let path = match picked {
            Some(p) => p,
            None => {
                self.status = "已取消导出".into();
                return;
            }
        };
        match deskkit_core::bundle::export_baseline(&dir, &path) {
            Ok(r) => {
                self.status = format!(
                    "已导出 {} 个文件 / {} 字节 → {}",
                    r.files,
                    r.bytes,
                    r.path.display()
                )
            }
            Err(e) => self.status = format!("导出失败：{e}"),
        }
    }

    fn import_baseline(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("导入基准线包")
            .add_filter("MemBase 基准线包", &["mbz", "zip"])
            .pick_file();
        let path = match picked {
            Some(p) => p,
            None => {
                self.status = "已取消导入".into();
                return;
            }
        };
        // 先校验，坏包绝不落地
        match deskkit_core::bundle::verify_package(&path) {
            Ok((n, crc, captured)) => {
                match deskkit_core::bundle::import_baseline(&self.root, &path, "imported") {
                    Ok(r) => {
                        self.status = format!(
                            "已导入 {} 个文件（采集 {}）→ {}",
                            n,
                            captured,
                            r.dir.display()
                        );
                        let _ = crc;
                        self.reload();
                    }
                    Err(e) => self.status = format!("导入失败：{e}"),
                }
            }
            Err(e) => self.status = format!("包校验失败：{e}"),
        }
    }
}

fn extract_captured(sysj: &str) -> String {
    let key = "\"CapturedAtLocal\"";
    if let Some(i) = sysj.find(key) {
        let rest = &sysj[i + key.len()..];
        if let Some(q1) = rest.find('"') {
            let rest2 = &rest[q1 + 1..];
            if let Some(q2) = rest2.find('"') {
                return rest2[..q2].to_string();
            }
        }
    }
    String::from("未知")
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // 不写死盘符：用 %WINDIR% 拼字体目录
    let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let font_dir = std::path::Path::new(&win_dir).join("Fonts");
    let candidates = [
        font_dir.join("msyh.ttc"),
        font_dir.join("Deng.ttf"),
        font_dir.join("simhei.ttf"),
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

/// 把当前调色板灌进 egui 的 Visuals（切主题时再调一次即可立刻换肤）
fn apply_theme(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    let c = pal();
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = c.bg;
    v.window_fill = c.panel;
    v.extreme_bg_color = c.field_bg;
    v.override_text_color = Some(c.text);
    v.selection.bg_fill = c.sel;
    // 深色下分隔线很暗，控件（勾选框 / 输入框）描边单独提亮一档才看得清
    let ctl_line = if dark {
        egui::Color32::from_rgb(112, 112, 112)
    } else {
        c.line
    };
    v.widgets.noninteractive.bg_fill = c.card;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, c.line);
    v.widgets.inactive.bg_fill = c.field_bg;
    v.widgets.inactive.weak_bg_fill = c.field_bg;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, ctl_line);
    v.widgets.hovered.bg_fill = c.row_hover;
    v.widgets.hovered.weak_bg_fill = c.row_hover;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ctl_line);
    v.widgets.active.bg_fill = c.sel_nav;
    v.widgets.active.weak_bg_fill = c.sel_nav;
    ctx.all_styles_mut(move |s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
    });
}


// ---------- 浅色基元（对齐 Windows 11 任务管理器的观感）----------

/// 卡片页头：左边标题 + 副标题，右边动作；底下一根分隔线
fn page_head<F: FnOnce(&mut egui::Ui)>(ui: &mut egui::Ui, title: &str, sub: &str, actions: F) {
    let r = ui.available_rect_before_wrap();
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(egui::RichText::new(title).size(16.5).strong().color(pal().text));
        if !sub.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(sub).size(11.5).color(pal().muted));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            actions(ui);
        });
    });
    ui.add_space(9.0);
    let y = ui.cursor().top();
    ui.painter().rect_filled(
        egui::Rect::from_min_size(egui::pos2(r.left(), y), egui::vec2(r.width(), 1.0)),
        egui::CornerRadius::ZERO,
        pal().line,
    );
}

/// 普通按钮
fn btn(text: &str, size: f32, w: f32) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).size(size).color(pal().text))
        .min_size(egui::vec2(w, 30.0))
}

/// 主操作按钮：可用时蓝底白字，不可用时灰底灰字
fn btn_primary(text: &str, w: f32, on: bool) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .size(13.0)
            .color(if on { pal().on_accent } else { pal().muted }),
    )
    .fill(if on { pal().accent } else { pal().disabled_bg })
    .min_size(egui::vec2(w, 30.0))
}


/// 按钮 hover 统一样式：先说什么事，再给出等价命令
fn with_cli(resp: egui::Response, key: &str) -> egui::Response {
    match cli_map::cli_for(key) {
        Some(c) => resp.on_hover_text(format!("同 CLI：{}", c)),
        None => resp,
    }
}

/// 小号次要按钮（列表下方的批量动作）
fn btn_small(text: &str, w: f32) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).size(12.0).color(pal().text))
        .min_size(egui::vec2(w, 27.0))
}

/// 「键 / 值」一行
fn kv_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.add_sized(
            [88.0, 18.0],
            egui::Label::new(egui::RichText::new(key).size(12.0).color(pal().muted)),
        );
        ui.label(egui::RichText::new(elide(value, 76)).size(12.5).color(pal().text))
            .on_hover_text(value);
    });
    ui.add_space(2.0);
}

/// 一段说明文字
fn note(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(egui::RichText::new(text).size(11.5).color(color));
    });
    ui.add_space(2.0);
}

/// 侧栏图标：全部用 painter 画，不依赖图标字体
fn nav_icon(p: &egui::Painter, c: egui::Pos2, page: Page, color: egui::Color32) {
    let s = egui::Stroke::new(1.3, color);
    match page {
        Page::Processes => {
            for (i, w) in [13.0f32, 8.0, 11.0].iter().enumerate() {
                let y = c.y - 4.5 + i as f32 * 4.5;
                p.line_segment([egui::pos2(c.x - 6.5, y), egui::pos2(c.x - 6.5 + w, y)], s);
            }
        }
        Page::Baseline => {
            p.circle_stroke(c, 6.5, s);
            p.line_segment(
                [egui::pos2(c.x - 4.5, c.y + 2.0), egui::pos2(c.x + 4.5, c.y - 2.0)],
                s,
            );
        }
        Page::Protect => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(11.0, 12.0)),
                egui::CornerRadius::same(3),
                s,
                egui::StrokeKind::Middle,
            );
            p.line_segment(
                [egui::pos2(c.x - 2.5, c.y - 0.5), egui::pos2(c.x - 0.5, c.y + 2.0)],
                s,
            );
            p.line_segment(
                [egui::pos2(c.x - 0.5, c.y + 2.0), egui::pos2(c.x + 3.0, c.y - 2.5)],
                s,
            );
        }
        Page::Boot => {
            p.circle_stroke(c, 6.5, s);
            p.line_segment([c, egui::pos2(c.x, c.y - 4.0)], s);
            p.line_segment([c, egui::pos2(c.x + 2.8, c.y + 1.4)], s);
        }
        Page::Layouts => {
            // 一块屏幕 + 里面四个小方块（图标）
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(13.0, 10.5)),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Middle,
            );
            for (dx, dy) in [(-3.0f32, -2.0f32), (1.0, -2.0), (-3.0, 2.0), (1.0, 2.0)] {
                p.rect_filled(
                    egui::Rect::from_center_size(egui::pos2(c.x + dx, c.y + dy), egui::vec2(2.6, 2.6)),
                    egui::CornerRadius::same(1),
                    color,
                );
            }
        }
        Page::Tools => {
            // 屏幕 + 一道斜杠：屏幕关掉
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(13.0, 10.0)),
                egui::CornerRadius::same(2),
                s,
                egui::StrokeKind::Middle,
            );
            p.line_segment(
                [
                    egui::pos2(c.x - 5.5, c.y + 5.0),
                    egui::pos2(c.x + 5.5, c.y - 5.0),
                ],
                s,
            );
        }
        Page::Cli => {
            // 终端提示符 >_
            p.line_segment(
                [egui::pos2(c.x - 5.0, c.y - 3.5), egui::pos2(c.x - 1.5, c.y)],
                s,
            );
            p.line_segment(
                [egui::pos2(c.x - 1.5, c.y), egui::pos2(c.x - 5.0, c.y + 3.5)],
                s,
            );
            p.line_segment(
                [egui::pos2(c.x + 0.5, c.y + 3.5), egui::pos2(c.x + 5.0, c.y + 3.5)],
                s,
            );
        }
        Page::Settings => {
            p.circle_stroke(c, 4.8, s);
            for a in [0.0f32, 90.0, 180.0, 270.0] {
                let (sn, cs) = a.to_radians().sin_cos();
                p.line_segment(
                    [
                        egui::pos2(c.x + cs * 4.8, c.y + sn * 4.8),
                        egui::pos2(c.x + cs * 7.6, c.y + sn * 7.6),
                    ],
                    s,
                );
            }
        }
    }
}

/// 表格列宽：名称吃掉剩余宽度，其余定宽
#[derive(Clone, Copy)]
struct Cols {
    w: [f32; 6],
}

impl Cols {
    fn new(total: f32) -> Self {
        // 类型 / 分级 / 内存 / 保护 / 恢复
        let fixed = 58.0 + 82.0 + 92.0 + 62.0 + 62.0;
        Self {
            w: [(total - 24.0 - fixed).max(150.0), 58.0, 82.0, 92.0, 62.0, 62.0],
        }
    }

    /// 第 i 列在 area 里的矩形（0 名称 / 1 类型 / 2 分级 / 3 内存 / 4 保护 / 5 恢复）
    fn rect(&self, area: egui::Rect, i: usize) -> egui::Rect {
        let x = area.left() + self.w[..i].iter().sum::<f32>();
        egui::Rect::from_min_size(egui::pos2(x, area.top()), egui::vec2(self.w[i], area.height()))
    }
}

/// 内存列的热力底色：占得越多越蓝（和任务管理器给 CPU 列上色一个意思）
fn heat(mb: f64) -> egui::Color32 {
    let t = (mb / 600.0).clamp(0.0, 1.0);
    let (r, g, b) = pal().heat;
    egui::Color32::from_rgba_unmultiplied(r, g, b, (t * 220.0) as u8)
}

/// 表头：浅灰底 + 底部/列间分隔线
fn table_head(ui: &mut egui::Ui, cols: &Cols) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::hover());
    let area = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 16.0, rect.top()),
        egui::vec2(rect.width() - 24.0, rect.height()),
    );
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::ZERO, pal().head_bg);
    p.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.bottom() - 1.0),
            egui::vec2(rect.width(), 1.0),
        ),
        egui::CornerRadius::ZERO,
        pal().line,
    );
    for (i, t) in ["名称", "类型", "分级", "内存", "保护", "恢复"].iter().enumerate() {
        let c = cols.rect(area, i);
        let (pos, anchor) = match i {
            3 => (egui::pos2(c.right() - 10.0, c.center().y), egui::Align2::RIGHT_CENTER),
            4 | 5 => (c.center(), egui::Align2::CENTER_CENTER),
            _ => (egui::pos2(c.left(), c.center().y), egui::Align2::LEFT_CENTER),
        };
        p.text(pos, anchor, *t, egui::FontId::proportional(12.0), pal().muted);
    }
    for k in [3usize, 4, 5] {
        let c = cols.rect(area, k);
        p.rect_filled(
            egui::Rect::from_min_size(egui::pos2(c.left(), rect.top()), egui::vec2(1.0, rect.height())),
            egui::CornerRadius::ZERO,
            pal().line,
        );
    }
}

impl App {
    /// 左侧导航：四个页面，设置固定在最下面
    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        for p in [
            Page::Processes,
            Page::Baseline,
            Page::Layouts,
            Page::Protect,
            Page::Boot,
            Page::Tools,
            Page::Cli,
        ] {
            self.nav_item(ui, p);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            self.nav_item(ui, Page::Settings);
        });
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, page: Page) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::click());
        let sel = self.page == page;
        let p = ui.painter();
        if sel {
            p.rect_filled(rect, egui::CornerRadius::same(5), pal().sel_nav);
            p.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 1.0, rect.center().y - 9.0),
                    egui::vec2(3.0, 18.0),
                ),
                egui::CornerRadius::same(2),
                pal().accent,
            );
        } else if resp.hovered() {
            p.rect_filled(rect, egui::CornerRadius::same(5), pal().row_hover);
        }
        nav_icon(p, egui::pos2(rect.left() + 16.0, rect.center().y), page, if sel { pal().accent } else { pal().muted });
        p.text(
            egui::pos2(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            page.label(),
            egui::FontId::proportional(13.0),
            if sel { pal().text } else { pal().muted },
        );
        if resp.clicked() {
            self.page = page;
        }
        ui.add_space(2.0);
    }

    /// 进程页：任务管理器式表格
    fn page_processes(&mut self, ui: &mut egui::Ui) {
        // 启动扫描还在后台跑：先如实说明，别让空列表看起来像「没问题」
        let scanning = self.scan_rx.is_some();
        let sub = if scanning {
            "正在扫描当前状态…".to_string()
        } else {
            format!(
                "线外 {} 项 · 已保护 {} · 系统关键 {}",
                self.rows.len(),
                self.protected_count(),
                self.skipped.len()
            )
        };
        page_head(ui, "进程", &sub, |ui| {
            let (n, mb) = self.selected_count();
            let on = n > 0 && self.is_admin;
            let text = if n == 0 {
                "未勾选任何项".to_owned()
            } else {
                format!("恢复选中 {} 项 · {}", n, fmt_mb(mb))
            };
            let resp = ui.add_enabled(on, btn_primary(&text, 178.0, on));
            let resp = if on {
                resp.on_hover_text(format!(
                    "只会处理「恢复」那列勾上的行\n同 CLI：{}",
                    cli_map::cli_for("proc_trim").unwrap_or("")
                ))
            } else if n == 0 {
                resp.on_hover_text("先在列表里勾上要恢复的项")
            } else {
                resp.on_hover_text("需要以管理员身份运行")
            };
            if resp.clicked() {
                self.confirm = true;
            }

            let (pickable, picked) = self.pick_stats();
            let all_picked = pickable > 0 && picked == pickable;
            let label = if pickable == 0 {
                "无可勾选项".to_owned()
            } else if all_picked {
                "清空勾选".to_owned()
            } else {
                format!("全选 {} 项", pickable)
            };
            ui.add_space(8.0);
            if ui
                .add_enabled(pickable > 0, btn(&label, 12.5, 86.0))
                .on_hover_text(format!(
                    "{}\n同 CLI：{}",
                    if all_picked {
                        "取消所有勾选：这一轮什么都不恢复"
                    } else {
                        "勾选所有可处理项；已保护的和系统组件不受影响"
                    },
                    cli_map::cli_for("proc_pick_all").unwrap_or("")
                ))
                .clicked()
            {
                self.set_all_pick(!all_picked);
            }

            let protected_n = self.protected_count();
            let all_protected = !self.rows.is_empty() && protected_n == self.rows.len();
            let plabel = if protected_n > 0 && all_protected {
                "取消全部保护"
            } else {
                "全部保护"
            };
            ui.add_space(8.0);
            if ui
                .add_enabled(!self.rows.is_empty(), btn(plabel, 12.5, 104.0))
                .on_hover_text(format!(
                    "{}\n同 CLI：{}",
                    if all_protected {
                        "把这些项从保护名单里去掉；通配规则不受影响"
                    } else {
                        "把列表里的项全部记进保护名单，以后永不处理"
                    },
                    cli_map::cli_for("proc_protect_all").unwrap_or("")
                ))
                .clicked()
            {
                self.set_all_protect(!all_protected);
                self.status = if all_protected {
                    "已取消全部保护".to_owned()
                } else {
                    format!("已把 {} 项记为保护", self.rows.len())
                };
            }
            if protected_n > 0 && !all_protected {
                ui.add_space(8.0);
                if ui
                    .add(btn(&format!("已保护 {} · 清空", protected_n), 11.5, 118.0))
                    .on_hover_text("把这几位从保护名单里去掉")
                    .clicked()
                {
                    self.set_all_protect(false);
                    self.status = "已取消全部保护".to_owned();
                }
            }

            ui.add_space(8.0);
            if ui
                .checkbox(&mut self.include_caution, "含系统组件")
                .on_hover_text(format!(
                    "放开「谨慎」那一档，让系统组件也能被勾选\n同 CLI：{}",
                    cli_map::cli_for("proc_include_caution").unwrap_or("")
                ))
                .changed()
            {
                let on = self.include_caution;
                for r in self.rows.iter_mut() {
                    if r.tier == Tier::Caution && !r.protect {
                        r.pick = on;
                    }
                }
            }
            ui.add_space(8.0);
            let resp = ui.add(btn("刷新对比", 12.5, 84.0));
            let resp = with_cli(resp, "proc_refresh");
            if resp.clicked() {
                self.reload();
                self.status = format!("已刷新：线外 {} 项", self.rows.len());
            }
        });

        ui.add_space(8.0);
        let cols = Cols::new(ui.available_width());
        table_head(ui, &cols);

        let rows = std::mem::take(&mut self.rows);
        let mut toggled: Vec<(usize, bool)> = Vec::new();
        let mut picks: Vec<(usize, bool)> = Vec::new();
        let search = self.search.to_lowercase();
        let include_caution = self.include_caution;
        let matched = rows
            .iter()
            .filter(|r| search.is_empty() || r.name.to_lowercase().contains(&search))
            .count();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if matched == 0 {
                    ui.add_space(26.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(if rows.is_empty() {
                                if self.scan_rx.is_some() {
                                    "正在扫描当前状态…"
                                } else {
                                    "没有线外项：当前状态和基准一致"
                                }
                            } else {
                                "没有匹配的项"
                            })
                            .size(12.5)
                            .color(pal().grey),
                        );
                    });
                    return;
                }

                for (i, r) in rows.iter().enumerate() {
                    if !search.is_empty() && !r.name.to_lowercase().contains(&search) {
                        continue;
                    }
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::hover());
                    let hovered = resp.hovered();
                    let fill = if r.protect {
                        pal().bg
                    } else if r.pick {
                        pal().card_hl
                    } else if hovered {
                        pal().row_hover
                    } else {
                        pal().card
                    };
                    let area = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 16.0, rect.top()),
                        egui::vec2(rect.width() - 24.0, rect.height()),
                    );
                    let p = ui.painter();
                    p.rect_filled(rect, egui::CornerRadius::ZERO, fill);

                    // 内存列热力底 + 数值列之间的竖线 + 行底线
                    let mc = cols.rect(area, 3);
                    p.rect_filled(mc, egui::CornerRadius::ZERO, heat(r.mem_mb));
                    for k in [3usize, 4, 5] {
                        let c = cols.rect(area, k);
                        p.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(c.left(), rect.top()),
                                egui::vec2(1.0, rect.height()),
                            ),
                            egui::CornerRadius::ZERO,
                            pal().line,
                        );
                    }
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                            egui::vec2(rect.width(), 1.0),
                        ),
                        egui::CornerRadius::ZERO,
                        pal().line,
                    );

                    // 图标块：进程蓝底、服务灰底，里面放名字首字
                    let nc = cols.rect(area, 0);
                    let tile = egui::Rect::from_min_size(
                        egui::pos2(nc.left() + 2.0, rect.center().y - 9.0),
                        egui::vec2(18.0, 18.0),
                    );
                    p.rect_filled(
                        tile,
                        egui::CornerRadius::same(4),
                        if r.kind == Kind::Process { pal().accent } else { pal().grey },
                    );
                    let initial = r
                        .name
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default();
                    p.text(
                        tile.center(),
                        egui::Align2::CENTER_CENTER,
                        initial,
                        egui::FontId::proportional(11.0),
                        pal().on_accent,
                    );

                    let name_color = if r.protect || !r.pick { pal().muted } else { pal().text };
                    let mut shown_name = r.name.clone();
                    if r.instances > 1 {
                        shown_name = format!("{}  · {} 实例", shown_name, r.instances);
                    }
                    p.text(
                        egui::pos2(nc.left() + 26.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        elide(&shown_name, 56),
                        egui::FontId::proportional(13.0),
                        name_color,
                    );

                    let kc = cols.rect(area, 1);
                    p.text(
                        egui::pos2(kc.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        if r.kind == Kind::Process { "进程" } else { "服务" },
                        egui::FontId::proportional(12.0),
                        pal().muted,
                    );

                    let tc = cols.rect(area, 2);
                    let (tier_text, tier_color) = match r.tier {
                        Tier::Suggested => ("建议", pal().amber),
                        Tier::Caution => ("谨慎", pal().muted),
                        Tier::Blocked => ("禁止", pal().grey),
                    };
                    p.text(
                        egui::pos2(tc.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        tier_text,
                        egui::FontId::proportional(12.0),
                        tier_color,
                    );

                    p.text(
                        egui::pos2(mc.right() - 10.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        fmt_mb(r.mem_mb),
                        egui::FontId::proportional(12.5),
                        if r.mem_mb >= 200.0 && r.pick { pal().red } else { pal().muted },
                    );

                    // 保护：勾上就写进保护名单
                    let mut prot = r.protect;
                    let pc = egui::Rect::from_center_size(cols.rect(area, 4).center(), egui::vec2(22.0, 22.0));
                    let presp = ui.put(pc, egui::Checkbox::without_text(&mut prot));
                    if presp.changed() {
                        toggled.push((i, prot));
                    }
                    let _ = presp.on_hover_text(if r.protect {
                        if r.protect_by != core::protect_key(r.kind, &r.name) {
                            format!("通配保护命中：{}", r.protect_by)
                        } else {
                            "已保护：永不处理".to_owned()
                        }
                    } else {
                        format!(
                            "勾上 = 永不处理，并写进保护名单\n同 CLI：{}",
                            cli_map::cli_for("proc_protect").unwrap_or("")
                        )
                    });

                    // 恢复：勾上就参与这一轮关闭
                    let mut pk = r.pick;
                    let can = can_pick(r, include_caution);
                    let rc = egui::Rect::from_center_size(cols.rect(area, 5).center(), egui::vec2(22.0, 22.0));
                    let rresp = ui
                        .scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(rc)
                                .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
                            |ui| ui.add_enabled(can, egui::Checkbox::without_text(&mut pk)),
                        )
                        .inner;
                    if rresp.changed() {
                        picks.push((i, pk));
                    }
                    let _ = rresp.on_hover_text(if r.protect {
                        "已保护的项目不会被处理"
                    } else if !can {
                        "系统组件：先勾上「含系统组件」"
                    } else {
                        "勾上 = 本次恢复时终止它"
                    });

                    let _ = resp.on_hover_text(format!(
                        "{}{}",
                        r.reason,
                        if r.path.is_empty() {
                            String::new()
                        } else {
                            format!("\n{}", r.path)
                        }
                    ));
                }

                if !self.skipped.is_empty() {
                    ui.add_space(10.0);
                    let names: Vec<String> = self
                        .skipped
                        .iter()
                        .take(20)
                        .map(|s| format!("{}  {}", s.name, fmt_mb(s.mem_mb)))
                        .collect();
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "已自动跳过 {} 项：自身进程与关键系统组件，永不处理",
                                self.skipped.len()
                            ))
                            .size(11.5)
                            .color(pal().grey),
                        )
                        .on_hover_text(names.join("\n"));
                    });
                }
            });

        self.rows = rows;
        let touched = !toggled.is_empty();
        for (i, v) in toggled {
            if let Some(r) = self.rows.get_mut(i) {
                r.protect = v;
                if v {
                    r.pick = false;
                }
            }
        }
        if touched {
            self.save_whitelist();
        }
        for (i, v) in picks {
            if let Some(r) = self.rows.get_mut(i) {
                r.pick = v && !r.protect;
            }
        }
    }

    /// 基准页：基准线本体
    fn page_baseline(&mut self, ui: &mut egui::Ui) {
        page_head(ui, "基准", "刚开机那一刻的参照物", |ui| {
            let resp = ui.add(btn_primary("扫描当前状态设为基准", 168.0, true));
            let resp = with_cli(resp, "baseline_capture");
            if resp.clicked() {
                self.confirm_capture = true;
            }
            ui.add_space(8.0);
            if ui
                .add(btn("导出基准", 12.5, 84.0))
                .on_hover_text(format!(
                    "打包成 .mbz，单文件就能拷走\n同 CLI：{}",
                    cli_map::cli_for("baseline_export").unwrap_or("")
                ))
                .clicked()
            {
                self.export_baseline();
            }
            ui.add_space(8.0);
            if ui
                .add(btn("导入基准", 12.5, 84.0))
                .on_hover_text(format!(
                    "从 .mbz 还原一份基准\n同 CLI：{}",
                    cli_map::cli_for("baseline_import").unwrap_or("")
                ))
                .clicked()
            {
                self.import_baseline();
            }
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                kv_row(ui, "基准快照", &self.base_line);
                kv_row(ui, "采集详情", &self.base_meta);
                kv_row(ui, "数据目录", &self.root.to_string_lossy());
                kv_row(
                    ui,
                    "采集耗时",
                    &match self.collect_ms {
                        Some(ms) => format!(
                            "{}（{}）",
                            fmt_dur(ms),
                            if self.collect_from_base {
                                "基准自己记的"
                            } else {
                                "本次刷新实测"
                            }
                        ),
                        None => "未知".to_owned(),
                    },
                );
                kv_row(
                    ui,
                    "保护名单",
                    &format!("{} 条 · {}", self.whitelist().len(), self.protect_file.to_string_lossy()),
                );
                ui.add_space(10.0);
                note(ui, "基准 = 刚开机时该有的样子；刷新对比时多出来的东西才会出现在「进程」页。", pal().muted);
                note(ui, "把当前状态设为基准，会把此刻开着的浏览器、游戏一并算进去，之后不再清理。", pal().amber);
            });
    }

    /// 保护名单页：和 protect.txt 是同一个文件，改完立刻生效
    fn page_protect(&mut self, ui: &mut egui::Ui) {
        let items = self.whitelist();
        let sub = format!("{} 条规则 · 与列表里的「保护」勾选框同一个文件", items.len());
        let mut remove: Option<String> = None;
        page_head(ui, "保护名单", &sub, |ui| {
            if ui
                .add(btn("打开数据目录", 12.5, 118.0))
                .on_hover_text(self.protect_file.to_string_lossy())
                .clicked()
            {
                win::open_folder(&self.root.to_string_lossy());
            }
            ui.add_space(8.0);
            if ui
                .add_enabled(!items.is_empty(), btn("清空全部", 12.5, 88.0))
                .on_hover_text(format!(
                    "删掉 protect.txt 里所有规则，通配规则一并删除\n同 CLI：{}",
                    cli_map::cli_for("protect_clear").unwrap_or("")
                ))
                .clicked()
            {
                let _ = std::fs::write(&self.protect_file, core::serialize_whitelist(&[]));
                self.reload();
                self.status = "已清空保护名单".to_owned();
            }
        });
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(
                                "名单是空的：列表里勾「保护」会把项写进来，也可以直接手写 进程:MuMu* 这种通配规则",
                            )
                            .size(12.0)
                            .color(pal().grey),
                        );
                    });
                }
                for it in items.iter() {
                    let (record, kind) = match it.split_once(':') {
                        Some((k, n)) => (n.to_owned(), if k == "服务" { "服务" } else { "进程" }),
                        None => (it.clone(), "进程+服务"),
                    };
                    let glob = record.contains('*') || record.contains('?');
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::hover());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, egui::CornerRadius::ZERO, pal().row_hover);
                    }
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                            egui::vec2(rect.width(), 1.0),
                        ),
                        egui::CornerRadius::ZERO,
                        pal().line,
                    );
                    p.text(
                        egui::pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &record,
                        egui::FontId::proportional(13.0),
                        pal().text,
                    );
                    p.text(
                        egui::pos2(rect.left() + 250.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        kind,
                        egui::FontId::proportional(12.0),
                        pal().muted,
                    );
                    p.text(
                        egui::pos2(rect.left() + 310.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        if glob { "通配匹配" } else { "精确匹配" },
                        egui::FontId::proportional(12.0),
                        if glob { pal().accent } else { pal().muted },
                    );
                    let c = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 50.0, rect.center().y),
                        egui::vec2(76.0, 24.0),
                    );
                    let r = ui.put(c, btn("移除", 12.0, 76.0));
                    let r = with_cli(r, "protect_remove");
                    if r.clicked() {
                        remove = Some(it.clone());
                    }
                    ui.add_space(2.0);
                }
            });
        if let Some(k) = remove {
            self.remove_whitelist_entry(&k);
        }
    }

    /// 开机采集页：把 `deskkit install --at N` 做成按钮
    fn page_boot(&mut self, ui: &mut egui::Ui) {
        let sub = if self.boot.installed { "已安排" } else { "未安排" };
        page_head(ui, "开机采集", sub, |ui| {
            let can = self.is_admin;
            let label = if self.boot.installed {
                "重新安排"
            } else {
                "安排下次开机采集"
            };
            let b = egui::Button::new(
                egui::RichText::new(label)
                    .size(13.0)
                    .color(if can { pal().on_accent } else { pal().muted }),
            )
            .fill(if can { pal().accent } else { pal().disabled_bg })
            .min_size(egui::vec2(160.0, 30.0));
            let resp = ui.add_enabled(can, b);
            let resp = if can {
                resp.on_hover_text(format!(
                    "注册一个 SYSTEM 权限的开机任务，采完自动销毁\n同 CLI：{}（当前设定 {} 秒）",
                    cli_map::cli_for("boot_install").unwrap_or(""),
                    self.boot_at
                ))
            } else {
                resp.on_hover_text("需要以管理员身份运行才能挂开机任务")
            };
            if resp.clicked() {
                self.confirm_boot = true;
            }
            if self.boot.installed {
                ui.add_space(8.0);
                let r = ui.add(btn("取消安排", 12.5, 88.0));
                let r = with_cli(r, "boot_uninstall");
                if r.clicked()
                {
                    self.remove_boot();
                }
            }
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new("开机后第").size(12.0).color(pal().muted));
                    ui.add(
                        egui::DragValue::new(&mut self.boot_at)
                            .range(5..=600)
                            .speed(5.0)
                            .suffix(" 秒"),
                    )
                    .on_hover_text("开机后等多久再快照；太早桌面的启动项还没起来");
                    ui.label(egui::RichText::new("采集").size(12.0).color(pal().muted));
                    ui.label(
                        egui::RichText::new(match self.collect_ms {
                            Some(ms) => format!("，耗时约 {}", fmt_dur(ms)),
                            None => "，耗时未知".to_owned(),
                        })
                        .size(11.5)
                        .color(pal().muted),
                    );
                });
                ui.add_space(12.0);
                kv_row(ui, "计划任务", core::DEFAULT_TASK);
                kv_row(
                    ui,
                    "任务状态",
                    &if self.boot.installed {
                        format!(
                            "已安排：开机第 {} 秒",
                            if self.boot.at_sec >= 5 { self.boot.at_sec } else { self.boot_at }
                        )
                    } else {
                        "未安排".to_owned()
                    },
                );
                kv_row(ui, "任务程序", &self.boot.exe);
                kv_row(
                    ui,
                    "运行身份",
                    if self.is_admin { "管理员" } else { "普通用户（无法安排任务）" },
                );
                ui.add_space(10.0);
                note(ui, "采集完任务与程序副本会自动删除，不留常驻进程。", pal().muted);
                note(ui, "采集本身通常几十毫秒，对开机体验没有可感知影响。", pal().muted);
            });
    }

    /// 设置页：数据目录与运行环境
    fn page_settings(&mut self, ui: &mut egui::Ui) {
        page_head(ui, "设置", "数据目录与运行环境", |ui| {
            let resp = ui.add(btn("打开数据目录", 12.5, 118.0));
            let resp = with_cli(resp, "settings_open_dir");
            if resp.clicked() {
                win::open_folder(&self.root.to_string_lossy());
            }
            ui.add_space(8.0);
            let resp = ui.add(btn("更改数据目录…", 12.5, 128.0));
            let resp = with_cli(resp, "settings_choose_dir");
            if resp.clicked() {
                self.choose_dir();
            }
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                kv_row(ui, "数据目录", &self.root.to_string_lossy());
                kv_row(ui, "保护名单", &self.protect_file.to_string_lossy());
                kv_row(
                    ui,
                    "版本",
                    &format!(
                        "MemBase {} · MemBase.exe（egui · 纯 Rust）",
                        env!("CARGO_PKG_VERSION")
                    ),
                );
                kv_row(ui, "布局目录", &store::dir().to_string_lossy());
                kv_row(
                    ui,
                    "桌面状态",
                    &format!(
                        "{} 个图标 · {} · shell 接口{}",
                        self.lay_icons
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "—".into()),
                        match self.lay_arrange {
                            Some(true) => "自动排列开启",
                            Some(false) => "自动排列关闭",
                            None => "连不上桌面",
                        },
                        if self.lay_shell { "可用" } else { "不可用" }
                    ),
                );
                kv_row(ui, "平台", "Windows 10 / 11");
                kv_row(
                    ui,
                    "运行身份",
                    if self.is_admin {
                        "管理员（全部动作可用）"
                    } else {
                        "普通用户：恢复与开机采集会失败"
                    },
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.add_sized(
                        [88.0, 18.0],
                        egui::Label::new(
                            egui::RichText::new("外观").size(12.0).color(pal().muted),
                        ),
                    );
                    if ui
                        .selectable_label(!self.dark, egui::RichText::new("浅色").size(12.5))
                        .on_hover_text(format!(
                            "同 CLI：{}",
                            cli_map::cli_for("theme_light").unwrap_or("")
                        ))
                        .clicked()
                    {
                        self.dark = false;
                    }
                    if ui
                        .selectable_label(self.dark, egui::RichText::new("深色").size(12.5))
                        .on_hover_text(format!(
                            "同 CLI：{}",
                            cli_map::cli_for("theme_dark").unwrap_or("")
                        ))
                        .clicked()
                    {
                        self.dark = true;
                    }
                    if self.dark != dark_mode() {
                        set_dark_mode(self.dark);
                        // 与 `deskkit theme set` 同一个实现，界面和命令不会各写各的
                        let _ = deskkit_cli::write_theme(self.dark);
                        self.status = format!(
                            "外观已切到{}（同 CLI：deskkit theme set {}）",
                            if self.dark { "深色" } else { "浅色" },
                            if self.dark { "dark" } else { "light" }
                        );
                    }
                });
                ui.add_space(6.0);
                note(ui, "三档分级：建议（第三方程序）、谨慎（Windows 组件，需勾「含系统组件」）、禁止（关键系统组件，永不处理）。", pal().muted);
                note(ui, "先礼后兵：有窗口的先发关闭消息（等同点 ✕），宽限期后仍未退出的才强制结束。", pal().muted);
                note(ui, "「恢复选中」点下去前还有一次确认；命令行版本默认 dry-run，只有显式 --execute 才动手。", pal().muted);
            });
    }


    // ================= 桌面布局：数据与动作 =================

    /// 重新读布局目录，尽量保住当前选中项
    fn reload_layouts(&mut self) {
        let keep = self.current_layout().map(|e| e.file_name.clone());
        self.layouts = store::list();
        self.sel = keep
            .and_then(|k| self.layouts.iter().position(|e| e.file_name == k))
            .unwrap_or(0)
            .min(self.layouts.len().saturating_sub(1));
        self.refresh_plan();
    }

    fn current_layout(&self) -> Option<&LayoutEntry> {
        self.layouts.get(self.sel)
    }

    /// 桌面实时状态：图标数 / 自动排列 / 分辨率 / shell 接口 / 布局缓存
    fn refresh_layout_status(&mut self) {
        match Desktop::open() {
            Ok(d) => {
                self.lay_icons = d.count().ok().map(|n| n.max(0) as u32);
                self.lay_arrange = Some(d.auto_arrange());
            }
            Err(_) => {
                self.lay_icons = None;
                self.lay_arrange = None;
            }
        }
        let (w, h) = deskkit_win::sys::screen_size();
        self.lay_screen = format!("{}×{}", w, h);
        self.lay_shell = ShellView::try_open().is_some();
        self.lay_cache = repair::describe_cache();
    }

    /// 差异只在选中项 / 桌面状态变化时重算：每帧读 explorer 内存太贵
    fn refresh_plan(&mut self) {
        let Some(e) = self.current_layout() else {
            self.plan_cache = None;
            self.ghost.clear();
            return;
        };
        let key = format!("{}#{}", e.file_name, e.stamp);
        let data = e.data.clone();
        if let Some((k, _)) = &self.plan_cache {
            if k == &key {
                return;
            }
        }
        let plan = actions::plan(&data).unwrap_or_default();
        self.plan_cache = Some((key, plan));
        self.load_ghost();
    }

    fn load_ghost(&mut self) {
        self.ghost = match self.current_layout() {
            Some(e) => actions::ghost(&e.data).unwrap_or_default(),
            None => Vec::new(),
        };
    }

    fn do_layout_save(&mut self, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.status = "[x] 布局名字不能为空".into();
            return;
        }
        match actions::snapshot(&name) {
            Ok(layout) => {
                if layout.icons.is_empty() {
                    self.status = "[x] 桌面上没有图标（或图标被隐藏），未保存".into();
                    return;
                }
                match store::save(&layout) {
                    Ok(_) => {
                        self.icons.invalidate();
                        self.icon_tex.clear();
                        self.status = format!(
                            "已保存「{}」· {} 个图标 · 同 CLI：deskkit layout save {}",
                            name,
                            layout.icons.len(),
                            name
                        );
                        self.reload_layouts();
                        self.sel = self
                            .layouts
                            .iter()
                            .position(|e| e.data.name == name)
                            .unwrap_or(0);
                        self.refresh_layout_status();
                        self.refresh_plan();
                    }
                    Err(e) => self.status = format!("[x] 保存失败：{:#}", e),
                }
            }
            Err(e) => self.status = format!("[x] 读不到桌面：{:#}", e),
        }
    }

    fn do_layout_restore(&mut self) {
        let Some(entry) = self.current_layout() else {
            self.status = "[x] 先选中一个布局".into();
            return;
        };
        let data: Layout = entry.data.clone();
        let name = entry.display_name().to_string();
        match actions::restore(&data) {
            Ok(r) => {
                let mut parts = vec![format!("已恢复 {} 个图标", r.restored)];
                parts.push(format!("移动 {} 个", r.moved));
                if r.auto_arrange_disabled {
                    parts.push("已关闭自动排列".into());
                }
                if !r.missing.is_empty() {
                    parts.push(format!("{} 个已消失", r.missing.len()));
                }
                if !r.extra.is_empty() {
                    parts.push(format!("{} 个新增留在原位", r.extra.len()));
                }
                if !r.failed.is_empty() {
                    parts.push(format!("{} 个没摆到位", r.failed.len()));
                }
                parts.push(if r.used_shell_api {
                    "位置已写入系统，重启后保持".into()
                } else {
                    "只改了内存，重启可能被覆盖".into()
                });
                self.status = format!("「{}」{}", name, parts.join(" · "));
                self.refresh_layout_status();
                self.plan_cache = None;
                self.refresh_plan();
            }
            Err(e) => self.status = format!("[x] 恢复失败：{:#}", e),
        }
    }

    fn do_layout_diff(&mut self) {
        let Some(entry) = self.current_layout() else { return };
        let name = entry.display_name().to_string();
        self.plan_cache = None;
        self.refresh_plan();
        if let Some((_, p)) = &self.plan_cache {
            self.status = if p.to_move == 0 {
                format!("「{}」与当前桌面完全一致（{} 个图标）", name, p.total)
            } else {
                format!(
                    "「{}」{} 个图标会移动 · 已消失 {} · 新增 {}",
                    name,
                    p.to_move,
                    p.missing.len(),
                    p.extra.len()
                )
            };
        }
    }

    fn do_layout_delete(&mut self, index: usize) {
        let Some(entry) = self.layouts.get(index) else { return };
        let name = entry.display_name().to_string();
        match store::delete_entry(entry) {
            Ok(()) => {
                self.status = format!("已删除布局「{}」", name);
                self.plan_cache = None;
                self.reload_layouts();
            }
            Err(e) => self.status = format!("[x] 删除失败：{:#}", e),
        }
    }

    fn do_layout_repair(&mut self) {
        match repair::repair() {
            Ok(None) => self.status = "系统里没有 IconLayouts 缓存，已经是干净状态".into(),
            Ok(Some(info)) => {
                self.status = format!(
                    "已备份并删除 IconLayouts（{} 字节）→ 重启后执行一次恢复即可",
                    info.bytes
                );
                self.refresh_layout_status();
            }
            Err(e) => self.status = format!("[x] 修复失败：{:#}", e),
        }
    }

    /// 图标纹理：第一次用时从系统取真实图标，失败给个纯色块
    fn icon_texture(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        if let Some(hit) = self.icon_tex.get(name) {
            return hit.clone();
        }
        // 每帧的时间预算用完了：这帧先画占位色块，下一帧接着取
        let out_of_time = self
            .icon_deadline
            .map_or(false, |t| std::time::Instant::now() >= t);
        if out_of_time {
            self.icons_pending = true;
            return None;
        }
        let handle = self.icons.get(name).map(|img| upload_texture(ctx, name, img));
        self.icon_tex.insert(name.to_string(), handle.clone());
        handle
    }

    fn ensure_wallpaper(&mut self, ctx: &egui::Context) {
        if self.wallpaper.is_some() {
            return;
        }
        // 后台解好的壁纸：这一帧上传成纹理
        if self.wallpaper_rx.is_some() {
            let got = self.wallpaper_rx.as_ref().unwrap().try_recv();
            match got {
                Ok(Some(img)) => {
                    self.wallpaper = Some(upload_texture(ctx, "wallpaper", img));
                    self.wallpaper_rx = None;
                }
                Ok(None) => self.wallpaper_rx = None,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                Err(mpsc::TryRecvError::Disconnected) => self.wallpaper_rx = None,
            }
            return;
        }
        if self.wallpaper_tried {
            return;
        }
        self.wallpaper_tried = true;
        // 截图验收要稳定拍到真壁纸：自检模式下直接同步解
        if self.shot_path.is_some() {
            if let Some(img) = gui_icon::wallpaper_rgba(900) {
                self.wallpaper = Some(upload_texture(ctx, "wallpaper", img));
            }
            return;
        }
        // 4K 壁纸解码 + 缩放是几百毫秒级的活，放后台，界面先用渐变兜底
        let (tx, rx) = mpsc::channel();
        self.wallpaper_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(gui_icon::wallpaper_rgba(900));
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// 工具页要显示的三件事：几台显示器、关屏通道通不通、上次什么时候息屏
    fn refresh_screen_status(&mut self) {
        self.scr_monitors = deskkit_win::screen::monitor_count();
        self.scr_channel = deskkit_win::screen::probe();
        self.scr_last = deskkit_win::screen::last_off().unwrap_or_default();
    }

    /// 进入息屏：窗口铺成全屏纯黑；**默认不动显示器电源**。
    ///
    /// 关显示器电源（SC_MONITORPOWER）是「更省电」但更危险的一步：有些机器关得上、
    /// 叫不醒，屏幕就黑在那儿按什么都没反应。所以它默认关闭，只在用户在工具页
    /// 明确打开「同时关闭显示器」时才做。
    fn enter_blackout(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let secs = self.bo_secs;
        let mut note = String::new();
        let mut powered = false;
        if !self.bo_test && deskkit_win::screen::power_off() {
            match deskkit_win::screen::off() {
                // 关不掉也不算失败：黑屏照样挡住内容，只是费点电，如实告诉用户
                Ok(()) => powered = true,
                Err(e) => note = format!("（{}）", e),
            }
        }
        // 退出走系统级监听：低级键盘钩子 + 按键/光标轮询，跟窗口有没有焦点、
        // 显示器开着关着都无关。只认窗口事件的话，黑屏后经常就退不出来了
        let wake_ui = {
            let ctx = ctx.clone();
            // 命中那一刻直接戳界面重画：不能指望 100ms 的定时器在黑屏时也准时
            std::sync::Arc::new(move || ctx.request_repaint())
                as std::sync::Arc<dyn Fn() + Send + Sync>
        };
        match deskkit_win::screen::Watch::start(
            self.bo_key,
            // 自检模式下不认鼠标移动：截图验收时谁碰一下鼠标都会把黑屏层收掉
            !self.bo_test && deskkit_win::screen::mouse_exit(),
            !self.bo_test && deskkit_win::screen::block_keys(),
            Some(wake_ui),
        ) {
            Ok(w) => self.bo_watch = Some(w),
            Err(e) => note = format!("（{e}）"),
        }
        if !note.is_empty() {
            self.status = format!("息屏{note}");
        }
        self.blackout = Some(Blackout {
            until: if secs > 0 {
                Some(now + secs as f64)
            } else {
                None
            },
            hint_until: now + 3.0,
            entered: std::time::Instant::now(),
            powered,
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        // 息屏就是一片纯黑，鼠标指针也得收掉，不然屏幕上永远挂着一个箭头
        deskkit_win::screen::hide_cursor();
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
        ctx.request_repaint();
    }

    /// 退出息屏：还原窗口；真关过显示器才去叫醒它（没关过就什么都不用做）
    fn exit_blackout(&mut self, ctx: &egui::Context, reason: &str) {
        let powered = self.blackout.as_ref().map(|b| b.powered).unwrap_or(false);
        self.blackout = None;
        if let Some(mut w) = self.bo_watch.take() {
            w.stop();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::Normal,
        ));
        // 不管之前是什么状态，退出息屏都把指针还回来
        deskkit_win::screen::show_cursor();
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        if powered {
            deskkit_win::screen::wake();
        }
        self.refresh_screen_status();
        self.status = format!(
            "已退出息屏（{}）· 同 CLI：deskkit screen off --seconds {} --exit-key {}",
            reason,
            self.bo_secs,
            self.bo_key.as_str()
        );
        ctx.request_repaint();
    }

    /// 息屏期间的状态机：谁把我们叫回来、到点没有。
    ///
    /// 必须放在 [`eframe::App::logic`] 里，不能写在画界面的那条路上：
    /// 窗口一旦被最小化 / 隐藏，eframe 只调 `logic`，**根本不调 `ui`**，
    /// 退出判断写在 `ui` 里就再也不会被执行 —— 那就是「息屏退不出来」。
    fn blackout_tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some(b) = &self.blackout else { return };
        let armed = b.entered.elapsed().as_millis() > 800;
        let timed_out = b.until.map(|t| now >= t).unwrap_or(false);

        // 全局监听（首选）：跟窗口焦点、显示器开关、窗口是否被隐藏都无关
        let watched = self.bo_watch.as_ref().and_then(|w| w.hit());
        // 监听线程自己没了就说明「再也收不到退出」，宁可多退一次也别把人关在黑屏里
        let watch_dead = self
            .bo_watch
            .as_ref()
            .map(|w| !w.alive())
            .unwrap_or(false);

        if timed_out {
            self.exit_blackout(ctx, "到时间了");
            return;
        }
        if watch_dead {
            self.exit_blackout(ctx, "监听异常");
            return;
        }
        if !armed {
            return;
        }
        if let Some(hit) = watched {
            self.exit_blackout(ctx, hit.label());
            return;
        }
        // 兜底：窗口自己的事件（钩子挂不上、或监听没起来时还有这条路）
        let any_key = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        let click = ctx.input(|i| i.pointer.any_click());
        let hot = match self.bo_key {
            deskkit_win::screen::ExitKey::Esc => ctx.input(|i| i.key_pressed(egui::Key::Escape)),
            deskkit_win::screen::ExitKey::Space => ctx.input(|i| i.key_pressed(egui::Key::Space)),
            deskkit_win::screen::ExitKey::Enter => ctx.input(|i| i.key_pressed(egui::Key::Enter)),
            deskkit_win::screen::ExitKey::Any => any_key,
        };
        if hot || click {
            let why = if click {
                "鼠标点击"
            } else {
                self.bo_key.label()
            };
            self.exit_blackout(ctx, why);
        }
    }

    /// 息屏模式的整屏绘制：纯黑 + 前 3 秒的退出提示 + 倒计时。
    /// 只负责画，退不退由 [`Self::blackout_tick`] 决定。
    fn blackout_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // 关键：每帧都把 egui 的光标图标清成 None，egui-winit 才会持续
        // 调 `set_cursor_visible(false)`。只靠进息屏时发一次的
        // `CursorVisible(false)` 不够 —— 窗口全屏切换、指针进出会让
        // egui-winit 在后续帧里又把光标 `set_cursor_visible(true)` 顶回来。
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::None);
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, egui::Color32::BLACK);

        let now = ctx.input(|i| i.time);
        let Some(b) = &self.blackout else { return };
        if now >= b.hint_until {
            return;
        }
        let text = match b.until {
            Some(t) => {
                let left = (t - now).max(0.0) as u64;
                format!(
                    "按 {} 退出 · {} 后自动亮",
                    self.bo_key.label(),
                    deskkit_win::screen::fmt_hms(left)
                )
            }
            None => format!("按 {} 退出", self.bo_key.label()),
        };
        ui.painter().text(
            egui::pos2(rect.center().x, rect.center().y + 60.0),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(62),
        );
    }

    /// 把当前选中布局的等价命令拼出来：命令模板来自 ACTIONS 表，
    /// 这里只把 `<名字>` 换成真实的布局名，避免两处各写一份
    fn cli_hint(&self, key: &str) -> String {
        let name = self
            .current_layout()
            .map(|e| e.display_name().to_string())
            .unwrap_or_else(|| "<名字>".into());
        cli_map::cli_for(key)
            .unwrap_or("")
            .replace("<名字>", &name)
    }

    /// 从保护名单里删掉一条，并立刻重新对比
    fn remove_whitelist_entry(&mut self, key: &str) {
        let mut items = self.whitelist();
        items.retain(|w| w != key);
        let _ = std::fs::write(&self.protect_file, core::serialize_whitelist(&items));
        self.reload();
        self.status = format!("已从保护名单移除 {}", key);
    }
}


/// 两个新页面（桌面布局 / 命令行）单独开一块 impl，避免和原有方法挤在一起
impl App {
    /// 桌面布局页：左边布局清单，右边真实缩略预览（壁纸 + 真图标 + 任务栏）
    fn page_layouts(&mut self, ui: &mut egui::Ui) {
        let arrange = match self.lay_arrange {
            Some(true) => "自动排列：开启".to_string(),
            Some(false) => "自动排列：已关闭".to_string(),
            None => "自动排列：未知".to_string(),
        };
        let sub = match self.lay_icons {
            Some(n) => format!(
                "{} 份布局 · 桌面 {} 个图标 · {} · {}",
                self.layouts.len(),
                n,
                arrange,
                self.lay_screen
            ),
            None => format!("{} 份布局 · 连不上桌面（explorer 没在跑？）", self.layouts.len()),
        };

        page_head(ui, "桌面布局", &sub, |ui| {
            let resp = ui.add(btn_primary("保存当前桌面", 118.0, true));
            let resp = resp.on_hover_text(format!(
                "记下此刻每个图标的名字与像素坐标\n同 CLI：{}",
                cli_map::cli_for("layout_save").unwrap_or("")
            ));
            if resp.clicked() {
                self.dialog = Some(LayoutDialog::Save {
                    name: chrono::Local::now().format("%m-%d %H:%M").to_string(),
                });
            }

            let can = !self.layouts.is_empty() && self.lay_icons.is_some();
            ui.add_space(8.0);
            let resp = ui.add_enabled(can, btn_primary("恢复选中布局", 124.0, can));
            let resp = if can {
                resp.on_hover_text(format!(
                    "把图标摆回这份布局记录的位置\n同 CLI：{}",
                    self.cli_hint("layout_restore")
                ))
            } else if self.layouts.is_empty() {
                resp.on_hover_text("还没有布局，先存一份")
            } else {
                resp.on_hover_text("连不上桌面，无法恢复")
            };
            if resp.clicked() {
                self.confirm_restore = true;
            }

            ui.add_space(8.0);
            let resp = ui.add(btn("刷新", 12.5, 64.0));
            let resp = resp.on_hover_text(format!(
                "重读布局目录与桌面状态\n同 CLI：{}",
                cli_map::cli_for("layout_refresh").unwrap_or("")
            ));
            if resp.clicked() {
                self.reload_layouts();
                self.refresh_layout_status();
                self.plan_cache = None;
                self.refresh_plan();
                self.icons.invalidate();
                self.icon_tex.clear();
                self.wallpaper = None;
                self.wallpaper_tried = false;
                self.status = format!("已刷新：{} 份布局", self.layouts.len());
            }
        });

        ui.add_space(6.0);
        let total_h = (ui.available_height() - 10.0).max(200.0);
        ui.horizontal_top(|ui| {
            // ---------------- 左：布局清单 ----------------
            ui.add_space(8.0);
            ui.allocate_ui_with_layout(
                egui::vec2(286.0, total_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add_space(6.0);
                    let search = self.search.to_lowercase();
                    let items: Vec<usize> = self
                        .layouts
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| {
                            search.is_empty()
                                || e.display_name().to_lowercase().contains(&search)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    let mut click: Option<usize> = None;
                    let list_h = (ui.available_height() - 74.0).max(80.0);
                    egui::ScrollArea::vertical()
                        .id_salt("layout_list")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.layouts.is_empty() {
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(
                                        "还没有保存过布局\n点右上角「保存当前桌面」存一份",
                                    )
                                    .size(12.0)
                                    .color(pal().grey),
                                );
                            } else if items.is_empty() {
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new("没有匹配的布局")
                                        .size(12.0)
                                        .color(pal().grey),
                                );
                            }
                            for i in items {
                                let e = &self.layouts[i];
                                let w = ui.available_width();
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(w, 46.0),
                                    egui::Sense::click(),
                                );
                                let sel = i == self.sel;
                                let p = ui.painter();
                                if sel {
                                    p.rect_filled(rect, egui::CornerRadius::same(5), pal().card_hl);
                                    p.rect_filled(
                                        egui::Rect::from_min_size(
                                            egui::pos2(rect.left() + 1.0, rect.center().y - 12.0),
                                            egui::vec2(3.0, 24.0),
                                        ),
                                        egui::CornerRadius::same(2),
                                        pal().accent,
                                    );
                                } else if resp.hovered() {
                                    p.rect_filled(rect, egui::CornerRadius::same(5), pal().row_hover);
                                }
                                p.text(
                                    egui::pos2(rect.left() + 12.0, rect.top() + 13.0),
                                    egui::Align2::LEFT_CENTER,
                                    elide(e.display_name(), 20),
                                    egui::FontId::proportional(13.0),
                                    if sel { pal().text } else { pal().muted },
                                );
                                p.text(
                                    egui::pos2(rect.left() + 12.0, rect.top() + 31.0),
                                    egui::Align2::LEFT_CENTER,
                                    elide(
                                        &format!(
                                            "{} 个图标 · {} · {}",
                                            e.count(),
                                            e.data.screen_text(),
                                            e.stamp
                                        ),
                                        26,
                                    ),
                                    egui::FontId::proportional(11.0),
                                    pal().grey,
                                );
                                let resp = resp.on_hover_text(format!(
                                    "{}\n{} 个图标 · {}\n保存于 {}",
                                    e.display_name(),
                                    e.count(),
                                    e.data.screen_text(),
                                    e.stamp
                                ));
                                if resp.clicked() {
                                    click = Some(i);
                                }
                                ui.add_space(2.0);
                            }
                        });
                    if let Some(i) = click {
                        if i != self.sel {
                            self.sel = i;
                            self.refresh_plan();
                        }
                    }

                    ui.add_space(6.0);
                    // 次要动作：一行放不下就换行
                    ui.horizontal_wrapped(|ui| {
                        let has = !self.layouts.is_empty();
                        let r = ui.add_enabled(has, btn_small("差异预览", 76.0));
                        let r = r.on_hover_text(format!(
                            "只读：报告几个会动 / 几个已消失 / 几个新增\n同 CLI：{}",
                            self.cli_hint("layout_diff")
                        ));
                        if r.clicked() {
                            self.do_layout_diff();
                        }
                        ui.add_space(4.0);
                        let r = ui.add_enabled(has, btn_small("删除", 58.0));
                        let r = r.on_hover_text(format!(
                            "删掉这份布局文件，不动桌面\n同 CLI：{}",
                            self.cli_hint("layout_delete")
                        ));
                        if r.clicked() {
                            self.confirm_delete = true;
                        }
                        ui.add_space(4.0);
                        let r = ui.add(btn_small("修复缓存", 76.0));
                        let r = r.on_hover_text(format!(
                            "重启后固定错乱同一批图标时才需要\n同 CLI：{}",
                            cli_map::cli_for("layout_repair").unwrap_or("")
                        ));
                        if r.clicked() {
                            self.dialog = Some(LayoutDialog::Repair);
                        }
                        ui.add_space(4.0);
                        let r = ui.add(btn_small("诊断接口", 76.0));
                        let r = r.on_hover_text(format!(
                            "看 shell 官方接口能不能用（决定位置能否持久化）\n同 CLI：{}",
                            cli_map::cli_for("layout_probe").unwrap_or("")
                        ));
                        if r.clicked() {
                            match ShellView::try_open() {
                                Some(sv) => {
                                    self.status = format!(
                                        "shell 桌面视图可用：{} 个条目，恢复会写入系统并持久化",
                                        sv.count()
                                    )
                                }
                                None => {
                                    self.status = "shell 桌面视图拿不到：恢复会退回改内存，重启可能被覆盖".into()
                                }
                            }
                            self.lay_shell = ShellView::try_open().is_some();
                        }
                        ui.add_space(4.0);
                        let r = ui.add(btn_small("打开目录", 76.0));
                        let r = r.on_hover_text(format!(
                            "{}\n同 CLI：{}",
                            store::dir().display(),
                            cli_map::cli_for("layout_open_dir").unwrap_or("")
                        ));
                        if r.clicked() {
                            win::open_folder(&store::dir().to_string_lossy());
                        }
                    });
                },
            );

            // 竖向分隔线
            let r = ui.available_rect_before_wrap();
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(r.left(), r.top()),
                    egui::vec2(1.0, total_h - 12.0),
                ),
                egui::CornerRadius::ZERO,
                pal().line,
            );
            ui.add_space(10.0);

            // ---------------- 右：差异条 + 预览 ----------------
            ui.vertical(|ui| {
                self.layout_diff_strip(ui);
                let data = self.current_layout().map(|e| e.data.clone());
                match data {
                    Some(layout) => self.preview_layout(ui, &layout),
                    None => {
                        ui.add_space(30.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(
                                    "左侧还没有布局。保存一份之后，这里会画出你桌面当时的真实样子。",
                                )
                                .size(12.5)
                                .color(pal().grey),
                            );
                        });
                    }
                }
            });
        });
    }

    /// 预览上方的一行差异摘要（会动 / 已消失 / 新增 / 叠加当前位置）
    fn layout_diff_strip(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let (total, move_n, miss, extra) = match &self.plan_cache {
                Some((_, p)) => (p.total, p.to_move, p.missing.len(), p.extra.len()),
                None => (0, 0, 0, 0),
            };
            let muted = pal().muted;
            ui.label(
                egui::RichText::new(format!("布局 {} 个图标", total))
                    .size(11.5)
                    .color(muted),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!("会动 {}", move_n))
                    .size(11.5)
                    .color(if move_n > 0 { pal().amber } else { muted }),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!("已消失 {}", miss))
                    .size(11.5)
                    .color(if miss > 0 { pal().red } else { muted }),
            );
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!("新增 {}", extra))
                    .size(11.5)
                    .color(if extra > 0 { pal().accent } else { muted }),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                let r = ui.checkbox(&mut self.show_ghost, "叠加当前位置");
                let r = r.on_hover_text(format!(
                    "蓝色虚线 = 这些图标此刻在哪\n同 CLI：{}",
                    cli_map::cli_for("layout_ghost").unwrap_or("")
                ));
                if r.changed() && self.show_ghost {
                    self.load_ghost();
                }
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(if self.lay_shell {
                        "shell 接口 可用"
                    } else {
                        "shell 接口 不可用"
                    })
                    .size(11.5)
                    .color(if self.lay_shell { pal().green } else { pal().amber }),
                );
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("布局缓存 {}", self.lay_cache))
                        .size(11.5)
                        .color(muted),
                );
            });
        });
        ui.add_space(6.0);
    }

    /// 真实缩略预览：壁纸打底，按保存坐标摆真图标，最后画任务栏
    fn preview_layout(&mut self, ui: &mut egui::Ui, layout: &Layout) {
        self.ensure_wallpaper(ui.ctx());
        let size = ui.available_size();
        if size.x < 80.0 || size.y < 80.0 {
            return;
        }
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::same(8), pal().bg);

        if layout.icons.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "这份布局里没有图标",
                egui::FontId::proportional(12.5),
                pal().grey,
            );
            return;
        }

        let canvas = Canvas::new(rect, layout);
        let screen = canvas.screen_rect;

        // 壁纸（等比裁切铺满）；取不到就用深蓝渐变兜底
        if let Some(tex) = &self.wallpaper {
            let uv = cover_uv(tex.size_vec2(), screen);
            painter.image(tex.id(), screen, uv, egui::Color32::WHITE);
        } else {
            let steps = 28;
            for i in 0..steps {
                let t0 = i as f32 / steps as f32;
                let t1 = (i + 1) as f32 / steps as f32;
                let band = egui::Rect::from_min_max(
                    egui::pos2(screen.left(), screen.top() + screen.height() * t0),
                    egui::pos2(screen.right(), screen.top() + screen.height() * t1 + 1.0),
                );
                painter.rect_filled(
                    band,
                    egui::CornerRadius::ZERO,
                    mix_rgb((0x17, 0x25, 0x45), (0x23, 0x1B, 0x3E), t0),
                );
            }
        }

        let box_side = ICON_BOX * canvas.scale;

        // 叠加当前位置：只标「这次会动」的
        if self.show_ghost {
            for g in &self.ghost {
                let r = egui::Rect::from_min_size(
                    canvas.pos(g.x as f32, g.y as f32),
                    egui::Vec2::splat(box_side),
                );
                dashed_rect(
                    &painter,
                    r,
                    egui::Stroke::new(1.3, egui::Color32::from_rgba_unmultiplied(0x6F, 0x8C, 0xFF, 0xE6)),
                    4.0,
                    3.0,
                );
            }
        }

        // 悬停命中（倒序遍历，后画的在上面）
        let pointer = resp.hover_pos();
        let mut hovered: Option<(String, egui::Rect)> = None;
        for it in layout.icons.iter().rev() {
            let r = egui::Rect::from_min_size(
                canvas.pos(it.x as f32, it.y as f32),
                egui::Vec2::splat(box_side),
            );
            if let Some(p) = pointer {
                if r.contains(p) && hovered.is_none() {
                    hovered = Some((it.name.clone(), r));
                }
            }
        }

        for it in &layout.icons {
            let r = egui::Rect::from_min_size(
                canvas.pos(it.x as f32, it.y as f32),
                egui::Vec2::splat(box_side),
            );
            if let Some((name, _)) = &hovered {
                if name == &it.name {
                    painter.rect_filled(
                        r.expand(1.0),
                        egui::CornerRadius::same(4),
                        egui::Color32::from_rgba_unmultiplied(0x5B, 0x7C, 0xFF, 0x40),
                    );
                }
            }
            match self.icon_texture(ui.ctx(), &it.name) {
                Some(tex) => {
                    painter.image(tex.id(), r, uv_full(), egui::Color32::WHITE);
                }
                None => {
                    let (rr, gg, bb) = gui_icon::color_of(&it.name);
                    painter.rect_filled(
                        r,
                        egui::CornerRadius::same(3),
                        egui::Color32::from_rgb(rr, gg, bb),
                    );
                    let first = it.name.chars().next().unwrap_or('?').to_string();
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        first,
                        egui::FontId::proportional((box_side * 0.55).max(7.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
        }

        draw_taskbar(&painter, screen, canvas.scale);
        // 还有图标没取完：立刻再来一帧，让图标接着填
        if self.icons_pending {
            ui.ctx().request_repaint();
        }
        painter.rect_stroke(
            screen,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x64, 0x74, 0x94, 0x66)),
            egui::StrokeKind::Inside,
        );

        if let Some((name, icon_rect)) = hovered {
            draw_tooltip(&painter, rect, icon_rect, &name);
        }
    }


    /// 工具页：息屏（离开座位）+ 一块系统信息
    fn page_tools(&mut self, ui: &mut egui::Ui) {
        let sub = format!(
            "{} 台显示器 · {} · 关屏通道{}",
            self.scr_monitors,
            self.lay_screen,
            if self.scr_channel { "可用" } else { "不可用" }
        );
        page_head(ui, "工具", &sub, |ui| {
            let r = ui.add(btn("刷新", 12.5, 64.0));
            let r = r.on_hover_text("重新读显示器与息屏状态\n同 CLI：deskkit screen status");
            if r.clicked() {
                self.refresh_screen_status();
                self.status = "已刷新工具页状态".into();
            }
            ui.add_space(8.0);
            let r = ui.add(btn("打开布局目录", 12.5, 108.0));
            let r = with_cli(r, "layout_open_dir");
            if r.clicked() {
                win::open_folder(&store::dir().to_string_lossy());
            }
            ui.add_space(8.0);
            let r = ui.add(btn("打开数据目录", 12.5, 108.0));
            let r = with_cli(r, "settings_open_dir");
            if r.clicked() {
                win::open_folder(&self.root.to_string_lossy());
            }
        });

        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // ---------------- 息屏 ----------------
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("息屏")
                            .size(13.5)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("整屏变黑；不会关机、不会锁死，默认也不动显示器电源")
                            .size(11.5)
                            .color(pal().muted),
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let r = ui.add(btn_primary("息屏", 96.0, true));
                    let r = r.on_hover_text(format!(
                        "整屏变黑（默认不关显示器电源，最不容易退不出来）\n退出：按 ESC（或任意键 / 点一下鼠标）\n同 CLI：{}",
                        cli_map::cli_for("screen_off").unwrap_or("")
                    ));
                    if r.clicked() {
                        self.enter_blackout(ui.ctx());
                    }
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("息屏时长").size(12.0).color(pal().muted));
                    let mut hh = self.bo_secs / 3600;
                    let mut mm = (self.bo_secs / 60) % 60;
                    let mut ss = self.bo_secs % 60;
                    let r1 = ui.add(
                        egui::DragValue::new(&mut hh)
                            .range(0..=23)
                            .speed(0.2)
                            .suffix(" 时"),
                    );
                    let r2 = ui.add(
                        egui::DragValue::new(&mut mm)
                            .range(0..=59)
                            .speed(0.5)
                            .suffix(" 分"),
                    );
                    let r3 = ui.add(
                        egui::DragValue::new(&mut ss)
                            .range(0..=59)
                            .speed(0.5)
                            .suffix(" 秒"),
                    );
                    if r1.changed() || r2.changed() || r3.changed() {
                        self.bo_secs = hh * 3600 + mm * 60 + ss;
                        let _ = deskkit_win::screen::set_default_secs(self.bo_secs);
                        self.status = format!(
                            "默认息屏时长已设为 {} · 同 CLI：deskkit screen duration {}",
                            if self.bo_secs == 0 {
                                "不定时".to_string()
                            } else {
                                deskkit_win::screen::fmt_hms(self.bo_secs)
                            },
                            deskkit_win::screen::fmt_hms(self.bo_secs)
                        );
                    }
                    let hov = format!(
                        "三个框分别是 时 / 分 / 秒；全 0 = 不定时，一直黑到你按退出键\n改一下就会记住，和命令行同一个文件\n同 CLI：{}",
                        cli_map::cli_for("screen_duration").unwrap_or("")
                    );
                    let _ = r1.on_hover_text(&hov);
                    let _ = r2.on_hover_text(&hov);
                    let _ = r3.on_hover_text(&hov);

                    ui.add_space(10.0);
                    for (label, secs) in [("5 分钟", 300u64), ("30 分钟", 1800), ("1 小时", 3600)] {
                        if ui
                            .add(btn_small(label, 64.0))
                            .on_hover_text(format!(
                                "一键设为 {}\n同 CLI：deskkit screen duration {}",
                                label,
                                deskkit_win::screen::fmt_hms(secs)
                            ))
                            .clicked()
                        {
                            self.bo_secs = secs;
                            let _ = deskkit_win::screen::set_default_secs(secs);
                            self.status = format!("默认息屏时长已设为 {}", label);
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new("退出热键").size(12.0).color(pal().muted));
                    ui.add_space(6.0);
                    let before = self.bo_key;
                    egui::ComboBox::from_id_salt("screen_exit_key")
                        .selected_text(self.bo_key.label())
                        .width(96.0)
                        .show_ui(ui, |ui| {
                            for k in deskkit_win::screen::ExitKey::all() {
                                ui.selectable_value(&mut self.bo_key, k, k.label());
                            }
                        });
                    if self.bo_key != before {
                        let _ = deskkit_win::screen::set_exit_key(self.bo_key);
                        self.status = format!(
                            "退出热键已改为 {} · 同 CLI：deskkit screen key {}",
                            self.bo_key.label(),
                            self.bo_key.as_str()
                        );
                    }
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("息屏时按这个键退出（点鼠标也退）")
                            .size(11.0)
                            .color(pal().grey),
                    );
                });
                // 深度息屏：默认不关显示器电源（关了可能叫不醒，屏幕会黑在那儿）
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let r = ui.checkbox(&mut self.bo_power_off, "同时关闭显示器");
                    let r = r.on_hover_text(
                        "更省电，但有些机器关得掉、叫不醒（屏幕黑着按什么都没反应）。\n默认关闭：只盖全屏黑窗，ESC 一定退得出来",
                    );
                    if r.changed() {
                        let _ = deskkit_win::screen::set_power_off(self.bo_power_off);
                        self.status = if self.bo_power_off {
                            "已打开「同时关闭显示器」：退出时会尝试叫醒显示器".into()
                        } else {
                            "已关闭「同时关闭显示器」：只盖全屏黑窗，最稳妥".into()
                        };
                    }
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(if self.bo_power_off {
                            "退出时会叫醒显示器；叫不醒时按电源键进锁屏再回来"
                        } else {
                            "只黑屏、不动显示器电源 —— 最不容易「退不出来」"
                        })
                        .size(11.0)
                        .color(pal().grey),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(if self.bo_secs == 0 {
                            "当前：不定时，一直黑到你按退出键"
                        } else {
                            "当前：到时间会自动退出（没关过电源就不用叫醒）"
                        })
                        .size(11.0)
                        .color(pal().grey),
                    );
                });
                ui.add_space(8.0);
                kv_row(
                    ui,
                    "上次息屏",
                    &if self.scr_last.is_empty() {
                        "（本次还没用过）".to_string()
                    } else {
                        self.scr_last.clone()
                    },
                );
                kv_row(
                    ui,
                    "等价命令",
                    &format!(
                        "deskkit screen off --seconds {} --exit-key {}  ·  deskkit screen duration {}",
                        self.bo_secs,
                        self.bo_key.as_str(),
                        deskkit_win::screen::fmt_hms(self.bo_secs)
                    ),
                );

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("系统信息")
                            .size(13.5)
                            .strong()
                            .color(pal().text),
                    );
                });
                ui.add_space(6.0);
                kv_row(ui, "显示器", &format!("{} 台", self.scr_monitors));
                kv_row(ui, "主屏分辨率", &self.lay_screen);
                kv_row(
                    ui,
                    "缩放比例",
                    &format!("{:.0}%", deskkit_win::sys::dpi_scale() * 100.0),
                );
                kv_row(
                    ui,
                    "关屏通道",
                    if self.scr_channel {
                        "可用（有窗口接收 SC_MONITORPOWER）"
                    } else {
                        "不可用（远程桌面 / 虚拟显示环境常见）"
                    },
                );
                kv_row(ui, "布局目录", &store::dir().to_string_lossy());
                kv_row(ui, "数据目录", &self.root.to_string_lossy());
                kv_row(
                    ui,
                    "版本",
                    &format!("MemBase {} · MemBase.exe（egui · 纯 Rust）", env!("CARGO_PKG_VERSION")),
                );
                ui.add_space(8.0);
                note(
                    ui,
                    &format!(
                        "息屏 = 全屏纯黑 + 关显示器，不锁屏、不睡眠；按 {} 或点一下鼠标就回来。",
                        self.bo_key.label()
                    ),
                    pal().muted,
                );
                note(
                    ui,
                    "走的是系统自己的 WM_SYSCOMMAND / SC_MONITORPOWER，和开始菜单里的「睡眠」不是一回事。",
                    pal().muted,
                );
                note(
                    ui,
                    "远程桌面或虚拟显示器下关屏可能无效，此时界面会明说「关屏消息没有窗口接收」。",
                    pal().muted,
                );
            });
    }

    /// 命令行页：界面动作 → 等价命令的完整对照表
    fn page_cli(&mut self, ui: &mut egui::Ui) {
        let sub = format!(
            "{} 个界面动作 · 每个都有等价命令 · 加 --json 都是结构化输出",
            ACTIONS.len()
        );
        page_head(ui, "命令行", &sub, |ui| {
            let all: String = ACTIONS
                .iter()
                .map(|a| format!("{}  # {}", a.cli, a.label))
                .collect::<Vec<_>>()
                .join("\n");
            if ui
                .add(btn("复制全部命令", 12.5, 108.0))
                .on_hover_text("把这些命令一次拷到剪贴板")
                .clicked()
            {
                ui.ctx().copy_text(all);
                self.copied = Some("*".into());
                self.status = "已复制全部命令到剪贴板".into();
            }
            ui.add_space(8.0);
            if ui
                .add(btn("打开接口说明", 12.5, 112.0))
                .on_hover_text("deskkit help 的完整文本")
                .clicked()
            {
                self.status = "命令行速查：deskkit help / deskkit commands --json".into();
            }
        });

        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut last_page = "";
                for a in ACTIONS {
                    if a.page != last_page {
                        last_page = a.page;
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new(a.page)
                                    .size(12.0)
                                    .strong()
                                    .color(pal().muted),
                            );
                        });
                        ui.add_space(2.0);
                    }
                    let w = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::hover());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(rect, egui::CornerRadius::ZERO, pal().row_hover);
                    }
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - 1.0),
                            egui::vec2(rect.width(), 1.0),
                        ),
                        egui::CornerRadius::ZERO,
                        pal().line,
                    );
                    p.text(
                        egui::pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        elide(a.label, 22),
                        egui::FontId::proportional(12.5),
                        pal().text,
                    );
                    p.text(
                        egui::pos2(rect.left() + 214.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        elide(a.cli, 62),
                        egui::FontId::monospace(12.0),
                        pal().accent,
                    );
                    let c = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 44.0, rect.center().y),
                        egui::vec2(64.0, 24.0),
                    );
                    let label = if self.copied.as_deref() == Some(a.key) {
                        "已复制"
                    } else {
                        "复制"
                    };
                    if ui.put(c, btn_small(label, 64.0)).clicked() {
                        ui.ctx().copy_text(a.cli.to_string());
                        self.copied = Some(a.key.to_string());
                        self.status = format!("已复制：{}", a.cli);
                    }
                    let _ = resp.on_hover_text(format!("{}\n同 CLI：{}", a.note, a.cli));
                }

                ui.add_space(14.0);
                note(
                    ui,
                    "界面上的每个按钮，鼠标停上去都会写出它的等价命令；这张表也随 commands --json 一起导出。",
                    pal().muted,
                );
                note(
                    ui,
                    "需要动手的命令默认 dry-run：trim / layout restore / layout repair 都要显式 --execute。",
                    pal().amber,
                );
                note(
                    ui,
                    "退出码：0 成功 / 1 参数 / 2 无基准或布局 / 3 读写 / 4 动作失败 / 5 被拒绝。",
                    pal().muted,
                );
            });
    }

}

impl eframe::App for App {
    /// 窗口看不见的时候（最小化 / 被隐藏）eframe 只调这个，不调 `ui`，
    /// 所以息屏的退出判断必须挂在这里，否则黑屏就再也收不了场。
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.blackout.is_some() {
            self.blackout_tick(ctx);
            if self.blackout.is_some() {
                // 黑屏期间要持续轮询监听线程；窗口被隐藏时下一帧也靠它
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 顶栏的内存占用需要持续刷新，否则只有操作时才更新
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        // 后台扫描好了就换进界面；没好的话 50ms 后再看
        if self.scan_rx.is_some() {
            let got = self.scan_rx.as_ref().unwrap().try_recv();
            match got {
                Ok(s) => {
                    self.scan_rx = None;
                    self.apply_scan(s);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                Err(mpsc::TryRecvError::Disconnected) => self.scan_rx = None,
            }
        }
        // 每帧最多花 8ms 从系统取图标：一页几十个图标一次性取会把首帧卡住几百毫秒。
        // 取不到的这帧先画占位色块，下一帧继续；截图/自检模式不限制（要一次拍全）
        self.icon_deadline = if self.shot_path.is_some() {
            None
        } else {
            Some(std::time::Instant::now() + std::time::Duration::from_millis(8))
        };
        self.icons_pending = false;
        if self.themed_for != Some(self.dark) {
            apply_theme(&ctx, self.dark);
            self.themed_for = Some(self.dark);
        }
        if let Some(path) = self.shot_path.clone() {
            // 等 30 帧再截：弹窗有淡入动画，早截会拍到半透明的中间态
            if self.frames == 30 {
                ctx.request_screenshot(move |img| {
                    let _ = save_bmp(&img, &path);
                    std::process::exit(0);
                });
            }
            ctx.request_repaint();
        }
        // 调试：截图时顺便把确认弹窗打开，方便验证弹窗样式（boot = 开机采集弹窗）
        if self.frames == 2 {
            match std::env::var("DESKKIT_SHOT_MODAL").as_deref() {
                Ok("boot") => self.confirm_boot = true,
                Ok("layout") => self.confirm_restore = true,
                Ok(_) => self.confirm = true,
                Err(_) => {}
            }
        }
        // 自检：DESKKIT_BLACKOUT_TEST=1 时自动进一次息屏（不真关显示器）。
        // 帧计数一直走，这个自检才能脱离截图模式单独跑
        if self.bo_test && self.blackout.is_none() && self.frames == 3 {
            self.enter_blackout(&ctx);
        }
        self.frames += 1;
        // 息屏模式：整屏纯黑，除了 ESC/任意键之外什么都不画
        // （放在截图逻辑之后：否则帧计数停住，自检截图永远等不到第 30 帧）
        if self.blackout.is_some() {
            self.blackout_ui(ui, &ctx);
            return;
        }

        // ---------- 左侧导航 ----------
        egui::Panel::left("nav")
            .exact_size(196.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal().bg)
                    .inner_margin(egui::Margin::symmetric(8, 12)),
            )
            .show(ui, |ui| {
                self.nav(ui);
            });

        // ---------- 顶部：搜索 + 运行身份 ----------
        egui::Panel::top("topbar")
            .exact_size(54.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal().bg)
                    .inner_margin(egui::Margin::symmetric(14, 11)),
            )
            .show(ui, |ui| {
              ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(320.0, 32.0), egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, egui::CornerRadius::same(6), pal().field_bg);
                p.rect_stroke(
                    rect,
                    egui::CornerRadius::same(6),
                    egui::Stroke::new(1.0, pal().line),
                    egui::StrokeKind::Inside,
                );
                let c = egui::pos2(rect.left() + 16.0, rect.center().y);
                p.circle_stroke(c, 5.5, egui::Stroke::new(1.4, pal().muted));
                p.line_segment(
                    [
                        egui::pos2(c.x + 4.0, c.y + 4.0),
                        egui::pos2(c.x + 8.0, c.y + 8.0),
                    ],
                    egui::Stroke::new(1.4, pal().muted),
                );
                let edit = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 30.0, rect.top() + 4.0),
                    egui::pos2(rect.right() - 10.0, rect.bottom() - 4.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(edit)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .frame(egui::Frame::NONE)
                                .hint_text(if self.page == Page::Layouts {
                                    "搜索布局名称"
                                } else {
                                    "搜索名称或发布者"
                                })
                                .desired_width(edit.width()),
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 当前整机内存占用：百分比 + 已用 / 总可用
                    let mem = win::memory_status();
                    let total_gb = mem.ullTotalPhys as f64 / 1024.0 / 1024.0 / 1024.0;
                    let used_gb =
                        mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys) as f64
                            / 1024.0
                            / 1024.0
                            / 1024.0;
                    let pct = if total_gb > 0.0 {
                        used_gb / total_gb * 100.0
                    } else {
                        0.0
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "内存 {:.0}% · 已用 {:.1}/{:.1} GB",
                            pct, used_gb, total_gb
                        ))
                        .size(11.5)
                        .color(if pct >= 85.0 { pal().amber } else { pal().muted }),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "桌面 {} 个图标 · {} 份布局",
                            self.lay_icons
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "—".into()),
                            self.layouts.len()
                        ))
                        .size(11.5)
                        .color(pal().muted),
                    );
                    ui.add_space(12.0);
                    if self.is_admin {
                        ui.label(
                            egui::RichText::new(format!(
                                "线外 {} 项 · 可释放 {}",
                                self.rows.len(),
                                fmt_mb(self.selected_count().1)
                            ))
                            .size(11.5)
                            .color(pal().muted),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("以非管理员运行：恢复与开机采集会失败")
                                .size(11.5)
                                .color(pal().amber),
                        );
                    }
                });
              });
            });

        // ---------- 状态栏 ----------
        egui::Panel::bottom("status")
            .exact_size(30.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(pal().bg)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(egui::pos2(r.left(), r.top()), egui::vec2(r.width(), 1.0)),
                    egui::CornerRadius::ZERO,
                    pal().line,
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (_, mb) = self.selected_count();
                    let (pickable, picked) = self.pick_stats();
                    let protected_n = self.protected_count();
                    ui.label(
                        egui::RichText::new(format!(
                            "线外 {} 项 · 已勾选 {}/{} · 已保护 {} · 系统关键 {}",
                            self.rows.len(),
                            picked,
                            pickable,
                            protected_n,
                            self.skipped.len()
                        ))
                        .size(11.5)
                        .color(pal().muted),
                    );
                    if mb > 0.0 {
                        ui.label(
                            egui::RichText::new(format!("可释放约 {}", fmt_mb(mb)))
                                .size(11.5)
                                .color(pal().green),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.status.is_empty() {
                            ui.label(egui::RichText::new(&self.status).size(11.5).color(pal().accent));
                        }
                    });
                });
            });

        // ---------- 内容卡片（按页分发）----------
        egui::CentralPanel::default_margins()
            .frame(
                egui::Frame::new()
                    .fill(pal().card)
                    .corner_radius(egui::CornerRadius {
                        nw: 8,
                        ne: 0,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(egui::Margin::symmetric(0, 0)),
            )
            .show(ui, |ui| match self.page {
                Page::Processes => self.page_processes(ui),
                Page::Baseline => self.page_baseline(ui),
                Page::Layouts => self.page_layouts(ui),
                Page::Protect => self.page_protect(ui),
                Page::Boot => self.page_boot(ui),
                Page::Tools => self.page_tools(ui),
                Page::Cli => self.page_cli(ui),
                Page::Settings => self.page_settings(ui),
            });

        // ---------- 确认弹窗 ----------
        if self.confirm {
            let (n, mb) = self.selected_count();
            let mut do_it = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("confirm_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    // 手动铺底：这一层里 Frame 的 fill 不生效，直接画矩形最稳
                    ui.set_width(460.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("确认恢复到基准状态？")
                            .size(17.0)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "将处理你勾选的 {} 项进程/服务，预计释放 {}。",
                            n,
                            fmt_mb(mb)
                        ))
                        .size(13.0),
                    );
                    ui.label(
                        egui::RichText::new(
                            "窗口程序会先收到关闭请求（等同点 ✕），3 秒后仍未退出的才强制结束。",
                        )
                        .size(11.5),
                    );
                    ui.label(
                        egui::RichText::new("未保存的数据可能丢失；被系统判定为必需的会自己重启。")
                            .size(11.5)
                            .color(pal().amber),
                    );
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("受影响的项目：").size(11.5).color(pal().muted));
                    let mut shown = 0;
                    for r in &self.rows {
                        if shown >= 10 {
                            break;
                        }
                        if !r.pick || !can_pick(r, self.include_caution) {
                            continue;
                        }
                        ui.label(
                            egui::RichText::new(format!("  · {} {}", r.name, fmt_mb(r.mem_mb)))
                                .size(11.5)
                                .color(pal().text),
                        );
                        shown += 1;
                    }
                    // 这轮勾选等价于哪条命令：直接抄走就能在脚本里复现
                    let mut names: Vec<String> = Vec::new();
                    for r in &self.rows {
                        if r.pick && can_pick(r, self.include_caution) && names.len() < 8 {
                            names.push(r.name.clone());
                        }
                    }
                    let cli = format!(
                        "deskkit trim --execute --yes{} --only {}",
                        if self.include_caution { " --include-caution" } else { "" },
                        if names.is_empty() { "<名字>".to_string() } else { names.join(",") }
                    );
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("等价命令：").size(11.5).color(pal().muted));
                    ui.label(
                        egui::RichText::new(elide(&cli, 70))
                            .size(11.0)
                            .monospace()
                            .color(pal().accent),
                    )
                    .on_hover_text(cli);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("取消").size(13.5),
                                )
                                .min_size(egui::vec2(90.0, 32.0)),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("确认执行")
                                        .size(13.5)
                                        .color(pal().on_accent),
                                )
                                .fill(pal().red)
                                .min_size(egui::vec2(120.0, 32.0)),
                            )
                            .clicked()
                        {
                            do_it = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if cancel {
                self.confirm = false;
            }
            if do_it {
                self.execute();
            }
        }

        // ---------- 设基准确认 ----------
        if self.confirm_capture {
            let mut do_it = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("capture_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(430.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("把当前状态设为基准？")
                            .size(17.0)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("会扫描此刻所有进程和服务，存成一份新基准并立刻生效。")
                            .size(12.5),
                    );
                    ui.label(
                        egui::RichText::new(
                            "注意：现在开着的浏览器 / 游戏也会被算进基准，之后不会再被清理。",
                        )
                        .size(11.5)
                        .color(pal().amber),
                    );
                    ui.label(
                        egui::RichText::new("想留住原来的开机基准，请先点「导出基准」备份。")
                            .size(11.5)
                            .color(pal().muted),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("取消").size(13.5))
                                    .min_size(egui::vec2(90.0, 32.0)),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("确认设为基准")
                                        .size(13.5)
                                        .color(pal().on_accent),
                                )
                                .fill(pal().accent)
                                .min_size(egui::vec2(140.0, 32.0)),
                            )
                            .clicked()
                        {
                            do_it = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if cancel {
                self.confirm_capture = false;
            }
            if do_it {
                self.confirm_capture = false;
                self.capture_now();
            }
        }

        // ---------- 开机采集确认 ----------
        if self.confirm_boot {
            let mut do_it = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("boot_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(450.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("安排下次开机采集？")
                            .size(17.0)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "开机第 {} 秒会以 SYSTEM 权限自动快照一次，存成新基准并立刻生效。",
                            self.boot_at
                        ))
                        .size(12.5),
                    );
                    ui.label(
                        egui::RichText::new(
                            "采集完成后任务和程序副本会自动删除，不会常驻后台。",
                        )
                        .size(11.5)
                        .color(pal().muted),
                    );
                    if let Some(ms) = self.collect_ms {
                        ui.label(
                            egui::RichText::new(format!(
                                "采集本身约 {}，开机第 {} 秒开始，{}完成。",
                                fmt_dur(ms),
                                self.boot_at,
                                finish_hint(ms)
                            ))
                            .size(11.5)
                            .color(pal().muted),
                        );
                    }
                    ui.label(
                        egui::RichText::new(
                            "现在开着的程序不会算进这份基准 —— 那才是「干净开机」的样子。",
                        )
                        .size(11.5)
                        .color(pal().amber),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("取消").size(13.5))
                                    .min_size(egui::vec2(90.0, 32.0)),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("立即安排")
                                        .size(13.5)
                                        .color(pal().on_accent),
                                )
                                .fill(pal().accent)
                                .min_size(egui::vec2(120.0, 32.0)),
                            )
                            .clicked()
                        {
                            do_it = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if cancel {
                self.confirm_boot = false;
            }
            if do_it {
                self.confirm_boot = false;
                self.install_boot();
            }
        }

        // ---------- 恢复布局确认 ----------
        if self.confirm_restore {
            let mut do_it = false;
            let mut cancel = false;
            let (total, move_n, miss, extra) = match &self.plan_cache {
                Some((_, p)) => (p.total, p.to_move, p.missing.len(), p.extra.len()),
                None => (0, 0, 0, 0),
            };
            let name = self
                .current_layout()
                .map(|e| e.display_name().to_string())
                .unwrap_or_default();
            let cli = self.cli_hint("layout_restore");
            egui::Modal::new(egui::Id::unique("layout_restore_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(470.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("把图标摆回这份布局？")
                            .size(17.0)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "布局「{}」有 {} 个图标，其中 {} 个位置不一致。",
                            name, total, move_n
                        ))
                        .size(12.5),
                    );
                    if miss > 0 || extra > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} 个图标已不存在（跳过），{} 个是后来新增的（保持原位）。",
                                miss, extra
                            ))
                            .size(11.5)
                            .color(pal().muted),
                        );
                    }
                    ui.label(
                        egui::RichText::new(
                            "检测到「自动排列图标」开着的话会先关掉，否则位置写完会被立刻重排。",
                        )
                        .size(11.5)
                        .color(pal().amber),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("等价命令：").size(11.5).color(pal().muted),
                    );
                    ui.label(
                        egui::RichText::new(&cli)
                            .size(11.5)
                            .monospace()
                            .color(pal().accent),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("取消").size(13.5))
                                    .min_size(egui::vec2(90.0, 32.0)),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("恢复布局")
                                        .size(13.5)
                                        .color(pal().on_accent),
                                )
                                .fill(pal().accent)
                                .min_size(egui::vec2(120.0, 32.0)),
                            )
                            .clicked()
                        {
                            do_it = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if cancel {
                self.confirm_restore = false;
            }
            if do_it {
                self.confirm_restore = false;
                self.do_layout_restore();
            }
        }

        // ---------- 删除布局确认 ----------
        if self.confirm_delete {
            let mut do_it = false;
            let mut cancel = false;
            let name = self
                .current_layout()
                .map(|e| e.display_name().to_string())
                .unwrap_or_default();
            let file = self
                .current_layout()
                .map(|e| e.path.to_string_lossy().to_string())
                .unwrap_or_default();
            let cli = self.cli_hint("layout_delete");
            egui::Modal::new(egui::Id::unique("layout_delete_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(450.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("删除布局「{}」？", name))
                            .size(17.0)
                            .strong()
                            .color(pal().text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("只删这份布局文件，不动桌面上的图标。")
                            .size(12.5),
                    );
                    ui.label(
                        egui::RichText::new(&file)
                            .size(11.0)
                            .monospace()
                            .color(pal().muted),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("等价命令：").size(11.5).color(pal().muted),
                    );
                    ui.label(
                        egui::RichText::new(&cli)
                            .size(11.5)
                            .monospace()
                            .color(pal().accent),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("取消").size(13.5))
                                    .min_size(egui::vec2(90.0, 32.0)),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("删除")
                                        .size(13.5)
                                        .color(pal().on_accent),
                                )
                                .fill(pal().red)
                                .min_size(egui::vec2(110.0, 32.0)),
                            )
                            .clicked()
                        {
                            do_it = true;
                        }
                    });
                    ui.add_space(4.0);
                });
            if cancel {
                self.confirm_delete = false;
            }
            if do_it {
                self.confirm_delete = false;
                let idx = self.sel;
                self.do_layout_delete(idx);
            }
        }

        // ---------- 保存布局（输名字） ----------
        if let Some(dlg) = self.dialog.take() {
            match dlg {
                LayoutDialog::Save { mut name } => {
                    let mut do_it = false;
                    let mut cancel = false;
                    egui::Modal::new(egui::Id::unique("layout_save_modal"))
                        .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                        .show(&ctx, |ui| {
                            ui.set_width(400.0);
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("保存当前桌面布局")
                                    .size(17.0)
                                    .strong()
                                    .color(pal().text),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(
                                    "会记下此刻每个图标的名字和像素坐标，同名布局直接覆盖。",
                                )
                                .size(12.0)
                                .color(pal().muted),
                            );
                            ui.add_space(8.0);
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut name)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("例如 工作 / 双屏 / 投影"),
                            );
                            r.request_focus();
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "等价命令：deskkit layout save {}",
                                    if name.trim().is_empty() { "<名字>" } else { name.trim() }
                                ))
                                .size(11.0)
                                .monospace()
                                .color(pal().muted),
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("保存").size(13.5).color(pal().on_accent),
                                        )
                                        .fill(pal().accent)
                                        .min_size(egui::vec2(110.0, 32.0)),
                                    )
                                    .clicked()
                                {
                                    do_it = true;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(egui::RichText::new("取消").size(13.5))
                                            .min_size(egui::vec2(90.0, 32.0)),
                                    )
                                    .clicked()
                                {
                                    cancel = true;
                                }
                            });
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                do_it = true;
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                cancel = true;
                            }
                            ui.add_space(4.0);
                        });
                    if do_it {
                        self.do_layout_save(&name);
                    } else if !cancel {
                        self.dialog = Some(LayoutDialog::Save { name });
                    }
                }
                LayoutDialog::Repair => {
                    let mut do_it = false;
                    let mut cancel = false;
                    egui::Modal::new(egui::Id::unique("layout_repair_modal"))
                        .frame(
                    egui::Frame::new()
                        .fill(pal().panel)
                        .stroke(egui::Stroke::new(1.0, pal().line))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                        .show(&ctx, |ui| {
                            ui.set_width(470.0);
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("修复桌面布局缓存？")
                                    .size(17.0)
                                    .strong()
                                    .color(pal().text),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(
                                    "只在「每次重启都固定错乱同一批图标、拖回去也没用」时才需要。",
                                )
                                .size(12.5),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "会先把注册表里的 IconLayouts 备份成文件，再删掉它；重启后系统重建一份干净的，然后恢复一次布局即可。",
                                )
                                .size(11.5)
                                .color(pal().muted),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("当前缓存：{}", self.lay_cache))
                                    .size(11.5)
                                    .color(pal().muted),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("等价命令：").size(11.5).color(pal().muted),
                            );
                            ui.label(
                                egui::RichText::new(
                                    cli_map::cli_for("layout_repair").unwrap_or(""),
                                )
                                    .size(11.5)
                                    .monospace()
                                    .color(pal().accent),
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("备份并清理")
                                                .size(13.5)
                                                .color(pal().on_accent),
                                        )
                                        .fill(pal().amber)
                                        .min_size(egui::vec2(130.0, 32.0)),
                                    )
                                    .clicked()
                                {
                                    do_it = true;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(egui::RichText::new("取消").size(13.5))
                                            .min_size(egui::vec2(90.0, 32.0)),
                                    )
                                    .clicked()
                                {
                                    cancel = true;
                                }
                            });
                            ui.add_space(4.0);
                        });
                    if do_it {
                        self.do_layout_repair();
                    } else if !cancel {
                        self.dialog = Some(LayoutDialog::Repair);
                    }
                }
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    // 同一个 exe 两种身份：带参数 = 命令行模式，双击（无参数）= 界面。
    // 命令行模式不初始化 egui，所以计划任务不会闪窗口。
    if std::env::args_os().len() > 1 {
        win::attach_parent_console();
        // PowerShell 不会等待 GUI 子系统进程，可能提前关掉输出管道；
        // 这时 println! 会 panic，属于调用方行为，安静退出即可。
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !info.to_string().contains("failed printing to std") {
                default_hook(info);
            }
        }));
        let code = match std::panic::catch_unwind(deskkit_cli::run) {
            Ok(()) => 0,
            Err(_) => 3,
        };
        std::process::exit(code);
    }
    // DPI 感知 + COM 单线程套间：桌面图标坐标和 shell 接口都依赖它们
    deskkit_win::init();
    let icon = egui::IconData {
        rgba: include_bytes!("../../../assets/icon_256.rgba").to_vec(),
        width: 256,
        height: 256,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([980.0, 620.0])
            .with_title("MemBase · 内存基准线 + 桌面布局")
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "MemBase",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

// ---------------------------------------------------------------- 预览换算与绘制

/// 图标框尺寸（100% 缩放下约 50px）
const ICON_BOX: f32 = 50.0;

struct Canvas {
    screen_rect: egui::Rect,
    scale: f32,
    ox: f32,
    oy: f32,
}

impl Canvas {
    fn new(area: egui::Rect, layout: &Layout) -> Self {
        let sw = if layout.screen.w > 0 {
            layout.screen.w as f32
        } else {
            1920.0
        };
        let sh = if layout.screen.h > 0 {
            layout.screen.h as f32
        } else {
            1080.0
        };

        // 按坐标范围扩画布，兼容主屏左侧的负坐标副屏
        let mut min_x = 0.0f32;
        let mut min_y = 0.0f32;
        let mut max_x = sw;
        let mut max_y = sh;
        for it in &layout.icons {
            let x = it.x as f32;
            let y = it.y as f32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + ICON_BOX);
            max_y = max_y.max(y + ICON_BOX);
        }

        let vw = (max_x - min_x).max(1.0);
        let vh = (max_y - min_y).max(1.0);
        const PAD: f32 = 9.0;
        let scale = (((area.width() - PAD * 2.0) / vw).min((area.height() - PAD * 2.0) / vh))
            .max(0.02);
        let ox = area.left() + (area.width() - vw * scale) / 2.0 - min_x * scale;
        let oy = area.top() + (area.height() - vh * scale) / 2.0 - min_y * scale;

        Canvas {
            screen_rect: egui::Rect::from_min_max(
                egui::pos2(ox, oy),
                egui::pos2(ox + sw * scale, oy + sh * scale),
            ),
            scale,
            ox,
            oy,
        }
    }

    fn pos(&self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(self.ox + x * self.scale, self.oy + y * self.scale)
    }
}

fn uv_full() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

/// 等比裁切铺满（相当于 WPF 的 UniformToFill）
fn cover_uv(tex: egui::Vec2, dst: egui::Rect) -> egui::Rect {
    let mut uv = uv_full();
    if tex.x <= 0.0 || tex.y <= 0.0 || dst.height() <= 0.0 {
        return uv;
    }
    let tex_ar = tex.x / tex.y;
    let dst_ar = dst.width() / dst.height();
    if tex_ar > dst_ar {
        let frac = dst_ar / tex_ar;
        let dx = (1.0 - frac) / 2.0;
        uv.min.x = dx;
        uv.max.x = 1.0 - dx;
    } else {
        let frac = tex_ar / dst_ar;
        let dy = (1.0 - frac) / 2.0;
        uv.min.y = dy;
        uv.max.y = 1.0 - dy;
    }
    uv
}

fn mix_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    egui::Color32::from_rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn dashed_rect(
    painter: &egui::Painter,
    r: egui::Rect,
    stroke: egui::Stroke,
    dash: f32,
    gap: f32,
) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    for w in pts.windows(2) {
        painter.extend(egui::Shape::dashed_line(&[w[0], w[1]], stroke, dash, gap));
    }
}

/// 任务栏：开始按钮 / 搜索框 / 几个固定程序 / 时间
fn draw_taskbar(painter: &egui::Painter, screen: egui::Rect, scale: f32) {
    let tb = (52.0 * scale).max(4.0);
    let bar = egui::Rect::from_min_max(
        egui::pos2(screen.left(), screen.bottom() - tb),
        egui::pos2(screen.right(), screen.bottom()),
    );
    painter.rect_filled(
        bar,
        egui::CornerRadius::ZERO,
        egui::Color32::from_rgba_unmultiplied(0x0A, 0x0D, 0x14, 0xF0),
    );
    painter.line_segment(
        [
            egui::pos2(bar.left(), bar.top()),
            egui::pos2(bar.right(), bar.top()),
        ],
        egui::Stroke::new(scale.max(0.6), egui::Color32::from_rgba_unmultiplied(0x4A, 0x58, 0x76, 0x55)),
    );

    let cy = bar.center().y;
    let s = tb * 0.42;
    let start = egui::Rect::from_min_size(
        egui::pos2(bar.left() + tb * 0.28, cy - s / 2.0),
        egui::Vec2::splat(s),
    );
    painter.rect_filled(
        start,
        egui::CornerRadius::same((s * 0.22) as u8),
        egui::Color32::from_rgb(0x5B, 0x7C, 0xFF),
    );

    let search = egui::Rect::from_min_size(
        egui::pos2(start.right() + tb * 0.34, cy - tb * 0.19),
        egui::Vec2::new(screen.width() * 0.155, tb * 0.38),
    );
    painter.rect_filled(
        search,
        egui::CornerRadius::same((tb * 0.16) as u8),
        egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 0x1E),
    );

    let mut ax = search.right() + tb * 0.5;
    for i in 0..4 {
        let asz = tb * 0.4;
        let a = egui::Rect::from_min_size(egui::pos2(ax, cy - asz / 2.0), egui::Vec2::splat(asz));
        if a.right() > screen.right() - tb * 3.4 {
            break;
        }
        let alpha = if i == 0 { 0x40 } else { 0x1C };
        painter.rect_filled(
            a,
            egui::CornerRadius::same((asz * 0.2) as u8),
            egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, alpha),
        );
        ax += tb * 0.62;
    }

    let clock = egui::Rect::from_min_size(
        egui::pos2(
            screen.right() - tb * 0.42 - screen.width() * 0.045,
            cy - tb * 0.19,
        ),
        egui::Vec2::new(screen.width() * 0.045, tb * 0.38),
    );
    painter.rect_filled(
        clock,
        egui::CornerRadius::same((tb * 0.16) as u8),
        egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 0x1A),
    );
}

/// 悬停名称气泡：图标太小认不出时靠它
fn draw_tooltip(
    painter: &egui::Painter,
    area: egui::Rect,
    anchor: egui::Rect,
    name: &str,
) {
    let galley = painter.layout_no_wrap(
        name.to_string(),
        egui::FontId::proportional(11.5),
        egui::Color32::WHITE,
    );
    let tw = galley.size().x.min(230.0) + 16.0;
    let th = galley.size().y + 9.0;

    let mut x = anchor.center().x - tw / 2.0;
    let mut y = anchor.bottom() + 4.0;
    if y + th > area.bottom() - 4.0 {
        y = anchor.top() - th - 4.0;
    }
    x = x.max(area.left() + 4.0).min(area.right() - tw - 4.0);
    y = y.max(area.top() + 4.0).min(area.bottom() - th - 4.0);

    let pill = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(tw, th));
    painter.rect_filled(
        pill,
        egui::CornerRadius::same(5),
        egui::Color32::from_rgba_unmultiplied(0x1B, 0x20, 0x2C, 0xF2),
    );
    painter.rect_stroke(
        pill,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x3E, 0x49, 0x60, 0xAA)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::pos2(pill.left() + 8.0, pill.center().y - galley.size().y / 2.0),
        galley,
        egui::Color32::WHITE,
    );
}
