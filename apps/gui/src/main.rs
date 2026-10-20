//! MemBase GUI —— 开机内存基准线 · 一键复位
//!
//! 交互：列表里勾选 = 保护该项；点「恢复基准」把没勾选的线外项全部关掉。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use membase_core as core;
use membase_core::{Kind, Tier};
use membase_win::task::{self as boot, TaskInfo};
use membase_win::{close, snapshot, win};
use std::path::PathBuf;

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

/// 主题偏好落在 %APPDATA%\MemBase\theme.txt，和 config.json 同目录
fn theme_path() -> PathBuf {
    membase_core::config::config_path().with_file_name("theme.txt")
}

fn load_theme() -> bool {
    std::fs::read_to_string(theme_path())
        .map(|t| t.trim().eq_ignore_ascii_case("dark"))
        .unwrap_or(false)
}

fn save_theme(dark: bool) {
    let p = theme_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(p, if dark { "dark" } else { "light" });
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

/// 左侧导航的页面
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Processes,
    Baseline,
    Protect,
    Boot,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Processes => "进程",
            Page::Baseline => "基准",
            Page::Protect => "保护名单",
            Page::Boot => "开机采集",
            Page::Settings => "设置",
        }
    }
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
    /// 已经按哪个主题灌过 Visuals（None = 还没灌过）
    themed_for: Option<bool>,
    /// 当前页（左侧导航）
    page: Page,
    /// 深色主题（落在 %APPDATA%\MemBase\theme.txt）
    dark: bool,
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
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let head: String = chars[..max_chars.saturating_sub(1)].iter().collect();
    format!("{}…", head)
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
        let root = PathBuf::from(membase_core::config::data_root(None));
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
            shot_path: std::env::var("MEMBASE_SHOT").ok(),
            themed_for: None,
            page: Page::Processes,
            dark,
        };
        app.reload();
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

    fn reload(&mut self) {
        let base_dir = match membase_win::compare::find_latest_baseline(&self.root) {
            Some(p) => p,
            None => {
                self.status = format!("没有基准快照，先跑一次 membase collect（{}）", self.root.display());
                self.base_line = "（没有找到基准快照）".into();
                return;
            }
        };

        let sysj = std::fs::read_to_string(base_dir.join("system.json")).unwrap_or_default();
        let bn = json_num(&sysj, "UptimeSeconds").unwrap_or(0.0);
        let used = json_num(&sysj, "UsedPhysicalMemoryMB").unwrap_or(0.0);
        let load = json_num(&sysj, "MemoryLoadPercent").unwrap_or(0.0);
        let captured = json_num(&sysj, "CapturedAtLocal").unwrap_or(0.0);
        let _ = captured;
        let collect_ms = json_num(&sysj, "CollectMs").unwrap_or(0.0);
        self.collect_ms = if collect_ms > 0.0 {
            Some(collect_ms as u64)
        } else {
            None
        };
        self.collect_from_base = self.collect_ms.is_some();
        let cap_text = extract_captured(&sysj);
        // 把来源目录名也显示出来：这几个数字全部读自它的 system.json，改文件界面就会跟着变
        let run_name = base_dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| base_dir.display().to_string());
        self.base_line = format!("基准快照：{}", cap_text);
        self.base_meta = format!(
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
        if self.collect_ms.is_none() {
            self.collect_ms = Some(snap.collect_ms);
            self.collect_from_base = false;
        }
        let protected = win::protected_pids();
        let wl = self.whitelist();

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

        self.rows = rows;
        self.skipped = skipped;
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
        let cfg = membase_core::config::Config {
            data_root: Some(text.clone()),
        };
        match membase_core::config::save(&cfg) {
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
        let dir = match membase_win::compare::find_latest_baseline(&self.root) {
            Some(d) => d,
            None => {
                self.status = "没有基准可以导出".into();
                return;
            }
        };
        let captured =
            membase_core::bundle::snapshot_meta(&dir, "CapturedAtLocal").unwrap_or_default();
        let mut stamp = membase_core::bundle::stamp_from_local(&captured);
        if stamp.is_empty() {
            stamp = membase_core::bundle::timestamp();
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
        match membase_core::bundle::export_baseline(&dir, &path) {
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
        match membase_core::bundle::verify_package(&path) {
            Ok((n, crc, captured)) => {
                match membase_core::bundle::import_baseline(&self.root, &path, "imported") {
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

fn painter_outline(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(10),
        egui::Stroke::new(1.0, pal().line),
        egui::StrokeKind::Outside,
    );
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
        for p in [Page::Processes, Page::Baseline, Page::Protect, Page::Boot] {
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
        let sub = format!(
            "线外 {} 项 · 已保护 {} · 系统关键 {}",
            self.rows.len(),
            self.protected_count(),
            self.skipped.len()
        );
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
                resp.on_hover_text("只会处理「恢复」那列勾上的行")
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
                .on_hover_text(if all_picked {
                    "取消所有勾选：这一轮什么都不恢复"
                } else {
                    "勾选所有可处理项；已保护的和系统组件不受影响"
                })
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
                .on_hover_text(if all_protected {
                    "把这些项从保护名单里去掉；通配规则不受影响"
                } else {
                    "把列表里的项全部记进保护名单，以后永不处理"
                })
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
                .on_hover_text("放开「谨慎」那一档，让系统组件也能被勾选")
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
            if ui.add(btn("刷新对比", 12.5, 84.0)).clicked() {
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
                                "没有线外项：当前状态和基准一致"
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
                        "勾上 = 永不处理，并写进保护名单".to_owned()
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
            if ui.add(btn_primary("扫描当前状态设为基准", 168.0, true)).clicked() {
                self.confirm_capture = true;
            }
            ui.add_space(8.0);
            if ui
                .add(btn("导出基准", 12.5, 84.0))
                .on_hover_text("打包成 .mbz，单文件就能拷走")
                .clicked()
            {
                self.export_baseline();
            }
            ui.add_space(8.0);
            if ui
                .add(btn("导入基准", 12.5, 84.0))
                .on_hover_text("从 .mbz 还原一份基准")
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
                .on_hover_text("删掉 protect.txt 里所有规则，通配规则一并删除")
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
                    if ui.put(c, btn("移除", 12.0, 76.0)).clicked() {
                        remove = Some(it.clone());
                    }
                    ui.add_space(2.0);
                }
            });
        if let Some(k) = remove {
            self.remove_whitelist_entry(&k);
        }
    }

    /// 开机采集页：把 `membase install --at N` 做成按钮
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
                resp.on_hover_text("注册一个 SYSTEM 权限的开机任务，采完自动销毁")
            } else {
                resp.on_hover_text("需要以管理员身份运行才能挂开机任务")
            };
            if resp.clicked() {
                self.confirm_boot = true;
            }
            if self.boot.installed {
                ui.add_space(8.0);
                if ui
                    .add(btn("取消安排", 12.5, 88.0))
                    .on_hover_text("移除开机计划任务")
                    .clicked()
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
            if ui.add(btn("打开数据目录", 12.5, 118.0)).clicked() {
                win::open_folder(&self.root.to_string_lossy());
            }
            ui.add_space(8.0);
            if ui.add(btn("更改数据目录…", 12.5, 128.0)).clicked() {
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
                    &format!("MemBase {} · membase-ui（egui）", env!("CARGO_PKG_VERSION")),
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
                        .clicked()
                    {
                        self.dark = false;
                    }
                    if ui
                        .selectable_label(self.dark, egui::RichText::new("深色").size(12.5))
                        .clicked()
                    {
                        self.dark = true;
                    }
                    if self.dark != dark_mode() {
                        set_dark_mode(self.dark);
                        save_theme(self.dark);
                    }
                });
                ui.add_space(6.0);
                note(ui, "三档分级：建议（第三方程序）、谨慎（Windows 组件，需勾「含系统组件」）、禁止（关键系统组件，永不处理）。", pal().muted);
                note(ui, "先礼后兵：有窗口的先发关闭消息（等同点 ✕），宽限期后仍未退出的才强制结束。", pal().muted);
                note(ui, "「恢复选中」点下去前还有一次确认；命令行版本默认 dry-run，只有显式 --execute 才动手。", pal().muted);
            });
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 顶栏的内存占用需要持续刷新，否则只有操作时才更新
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
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
            self.frames += 1;
            ctx.request_repaint();
        }
        // 调试：截图时顺便把确认弹窗打开，方便验证弹窗样式（boot = 开机采集弹窗）
        if self.frames == 2 {
            match std::env::var("MEMBASE_SHOT_MODAL").as_deref() {
                Ok("boot") => self.confirm_boot = true,
                Ok(_) => self.confirm = true,
                Err(_) => {}
            }
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
                                .hint_text("搜索名称或发布者")
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
                Page::Protect => self.page_protect(ui),
                Page::Boot => self.page_boot(ui),
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
                    let bg = ui.max_rect().expand2(egui::vec2(20.0, 18.0));
                    ui.painter().rect_filled(bg, egui::CornerRadius::same(10), pal().panel);
                    painter_outline(ui, bg);
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
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 18)))
                .show(&ctx, |ui| {
                    let bg = ui.max_rect().expand2(egui::vec2(20.0, 18.0));
                    ui.painter()
                        .rect_filled(bg, egui::CornerRadius::same(10), pal().panel);
                    painter_outline(ui, bg);
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
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 18)))
                .show(&ctx, |ui| {
                    let bg = ui.max_rect().expand2(egui::vec2(20.0, 18.0));
                    ui.painter()
                        .rect_filled(bg, egui::CornerRadius::same(10), pal().panel);
                    painter_outline(ui, bg);
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
        let code = match std::panic::catch_unwind(membase_cli::run) {
            Ok(()) => 0,
            Err(_) => 3,
        };
        std::process::exit(code);
    }
    let icon = egui::IconData {
        rgba: include_bytes!("../../../assets/icon_256.rgba").to_vec(),
        width: 256,
        height: 256,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([920.0, 600.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "MemBase",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}