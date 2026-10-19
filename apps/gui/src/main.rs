//! MemBase GUI —— 开机内存基准线 · 一键复位
//!
//! 交互：列表里勾选 = 保护该项；点「恢复基准」把没勾选的线外项全部关掉。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use membase_core as core;
use membase_core::{Kind, Tier};
use membase_win::{snapshot, win};
use std::path::PathBuf;

const BG: egui::Color32 = egui::Color32::from_rgb(13, 18, 26);
const PANEL: egui::Color32 = egui::Color32::from_rgb(20, 27, 38);
const CARD: egui::Color32 = egui::Color32::from_rgb(26, 34, 47);
const CARD_HL: egui::Color32 = egui::Color32::from_rgb(32, 43, 58);
const LINE: egui::Color32 = egui::Color32::from_rgb(44, 57, 74);
const TEXT: egui::Color32 = egui::Color32::from_rgb(230, 236, 245);
const MUTED: egui::Color32 = egui::Color32::from_rgb(148, 163, 184);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(56, 189, 248);
const RED: egui::Color32 = egui::Color32::from_rgb(248, 113, 113);
const AMBER: egui::Color32 = egui::Color32::from_rgb(251, 191, 36);
const GREEN: egui::Color32 = egui::Color32::from_rgb(52, 211, 153);
const GREY: egui::Color32 = egui::Color32::from_rgb(100, 116, 139);

struct Row {
    kind: Kind,
    name: String,
    pids: Vec<u32>,
    instances: u32,
    mem_mb: f64,
    path: String,
    tier: Tier,
    reason: String,
    protect: bool,
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
    is_admin: bool,
    protect_file: PathBuf,
    frames: u32,
    shot_path: Option<String>,
    themed: bool,
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

fn fmt_mb(mb: f64) -> String {    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let root = PathBuf::from(membase_core::config::data_root(None));
        let protect_file = root.join("protect.txt");
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
            is_admin: win::is_admin(),
            protect_file,
            frames: 0,
            shot_path: std::env::var("MEMBASE_SHOT").ok(),
            themed: false,
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
        let items: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.protect)
            .map(|r| core::protect_key(r.kind, &r.name))
            .collect();
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
        let cap_text = extract_captured(&sysj);
        self.base_line = format!("基准快照：{}", cap_text);
        self.base_meta = format!(
            "开机第 {} 秒采集 · 已用 {:.0} MB · 占用率 {:.0}%",
            bn as u64, used, load
        );

        let base_procs = core::parse_by_name(
            &std::fs::read_to_string(base_dir.join("processes_by_name.csv")).unwrap_or_default(),
        );
        let base_svc = core::parse_service_states(
            &std::fs::read_to_string(base_dir.join("services.csv")).unwrap_or_default(),
        );

        let snap = snapshot::snapshot_now(true);
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
                pids: it.pids,
                instances: it.instances,
                mem_mb: it.mem_mb,
                path: it.path,
                tier: it.tier,
                reason: it.reason,
                protect: it.protect,
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
        let mut n = 0;
        let mut mb = 0.0;
        for r in &self.rows {
            if r.protect || r.tier == Tier::Blocked {
                continue;
            }
            if r.tier == Tier::Caution && !self.include_caution {
                continue;
            }
            n += 1;
            mb += r.mem_mb;
        }
        (n, mb)
    }

    fn execute(&mut self) {
        let protected = win::protected_pids();
        let mut ok = 0usize;
        let mut fail = 0usize;
        let mut details: Vec<String> = Vec::new();

        for r in &self.rows {
            if r.protect || r.tier == Tier::Blocked {
                continue;
            }
            if r.tier == Tier::Caution && !self.include_caution {
                continue;
            }
            match r.kind {
                Kind::Process => {
                    let mut hit = 0;
                    for &pid in &r.pids {
                        if protected.contains(&pid) {
                            continue;
                        }
                        match win::terminate_process(pid) {
                            Ok(()) => {
                                hit += 1;
                                ok += 1;
                            }
                            Err(e) => {
                                fail += 1;
                                details.push(format!("{}: {}", r.name, e));
                            }
                        }
                    }
                    if hit == 0 && r.pids.is_empty() {
                        details.push(format!("{}: 没有可终止的进程实例", r.name));
                    }
                }
                Kind::Service => match win::stop_service(&r.name) {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        fail += 1;
                        details.push(format!("{}: {}", r.name, e));
                    }
                },
            }
        }

        self.confirm = false;
        self.status = format!("已执行：成功 {} 个动作，失败 {} 个", ok, fail);
        if let Some(first) = details.first() {
            self.status.push_str(&format!("（首个失败：{}）", first));
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
                    "已把当前状态设为基准：{} 进程 / {} MB / 占用率 {}%",
                    snap.procs.len(),
                    snap.used_phys_mb,
                    snap.load_pct
                );
                self.reload();
            }
            Err(e) => self.status = format!("设基准失败：{e}"),
        }
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

/// 主题必须每帧都可能是「系统主题」，所以两种主题都写一遍，并强制用暗色
fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = egui::Color32::from_rgb(9, 13, 19);
    v.override_text_color = Some(TEXT);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.inactive.bg_fill = CARD;
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.hovered.bg_fill = CARD_HL;
    v.widgets.hovered.weak_bg_fill = CARD_HL;
    v.widgets.active.bg_fill = CARD_HL;
    v.widgets.active.weak_bg_fill = CARD_HL;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
    ctx.all_styles_mut(move |s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
    });
}

fn draw_logo(p: &egui::Painter, center: egui::Pos2, size: f32) {    p.circle_filled(center, size * 0.5, ACCENT);
    let bar_w = size * 0.13;
    let base = center.y + size * 0.24;
    let heights = [0.26f32, 0.38, 0.50];
    for (i, h) in heights.iter().enumerate() {
        let x = center.x - size * 0.26 + i as f32 * (bar_w * 1.9);
        let top = base - size * h;
        let r = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(bar_w, base - top));
        p.rect_filled(r, egui::CornerRadius::same(1), egui::Color32::WHITE);
    }
}

fn painter_outline(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(10),
        egui::Stroke::new(1.0, LINE),
        egui::StrokeKind::Outside,
    );
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.themed {
            apply_theme(&ctx);
            self.themed = true;
        }
        if let Some(path) = self.shot_path.clone() {
            if self.frames == 3 {
                ctx.request_screenshot(move |img| {
                    let _ = save_bmp(&img, &path);
                    std::process::exit(0);
                });
            }
            self.frames += 1;
            ctx.request_repaint();
        }
        // 调试：截图时顺便把确认弹窗打开，方便验证弹窗样式
        if self.frames == 2 && std::env::var("MEMBASE_SHOT_MODAL").is_ok() {
            self.confirm = true;
        }
        // ---------- 品牌栏（全部用 painter 画，避免嵌套布局错位）----------
        egui::Panel::top("brand")
            .exact_size(66.0)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let p = ui.painter();
                let cy = rect.center().y;
                draw_logo(p, egui::pos2(rect.left() + 30.0, cy), 30.0);
                p.text(
                    egui::pos2(rect.left() + 56.0, cy - 9.0),
                    egui::Align2::LEFT_CENTER,
                    "MemBase",
                    egui::FontId::proportional(20.0),
                    egui::Color32::WHITE,
                );
                p.text(
                    egui::pos2(rect.left() + 56.0, cy + 12.0),
                    egui::Align2::LEFT_CENTER,
                    "开机内存基准线 · 一键复位",
                    egui::FontId::proportional(11.5),
                    MUTED,
                );
                p.text(
                    egui::pos2(rect.right() - 14.0, cy - 9.0),
                    egui::Align2::RIGHT_CENTER,
                    &self.base_line,
                    egui::FontId::proportional(11.5),
                    MUTED,
                );
                p.text(
                    egui::pos2(rect.right() - 14.0, cy + 11.0),
                    egui::Align2::RIGHT_CENTER,
                    &self.base_meta,
                    egui::FontId::proportional(11.5),
                    GREY,
                );
                if !self.is_admin {
                    p.text(
                        egui::pos2(rect.right() - 14.0, cy + 28.0),
                        egui::Align2::RIGHT_CENTER,
                        "以非管理员运行：部分动作会失败",
                        egui::FontId::proportional(10.5),
                        AMBER,
                    );
                }
            });

        // ---------- 工具栏 ----------
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("刷新对比").size(13.5))
                            .min_size(egui::vec2(92.0, 32.0)),
                    )
                    .clicked()
                {
                    self.reload();
                    self.status = format!("已刷新：线外 {} 项", self.rows.len());
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("全部保护").size(13.5))
                            .min_size(egui::vec2(84.0, 32.0)),
                    )
                    .clicked()
                {
                    for r in self.rows.iter_mut() {
                        r.protect = true;
                    }
                    self.save_whitelist();
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("全部取消").size(13.5))
                            .min_size(egui::vec2(84.0, 32.0)),
                    )
                    .clicked()
                {
                    for r in self.rows.iter_mut() {
                        r.protect = false;
                    }
                    self.save_whitelist();
                }
                ui.checkbox(&mut self.include_caution, "包含系统组件");
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索名字")
                        .desired_width(150.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (n, mb) = self.selected_count();
                    let enabled = n > 0 && self.is_admin;
                    let btn = egui::Button::new(
                        egui::RichText::new(format!("恢复基准（{} 项 · {}）", n, fmt_mb(mb)))
                            .size(14.0)
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if enabled { RED } else { CARD })
                    .min_size(egui::vec2(210.0, 34.0));
                    if ui.add_enabled(enabled, btn).clicked() {
                        self.confirm = true;
                    }
                });
            });

            // 第二行：基准线本身的维护
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("扫描当前状态设为基准").size(12.5))
                            .min_size(egui::vec2(168.0, 30.0)),
                    )
                    .clicked()
                {
                    self.confirm_capture = true;
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("导出基准").size(12.5))
                            .min_size(egui::vec2(88.0, 30.0)),
                    )
                    .clicked()
                {
                    self.export_baseline();
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("导入基准").size(12.5))
                            .min_size(egui::vec2(88.0, 30.0)),
                    )
                    .clicked()
                {
                    self.import_baseline();
                }
                ui.separator();
                let dir_text = self.root.to_string_lossy().to_string();
                ui.label(egui::RichText::new("数据目录：").size(11.0).color(GREY));
                ui.label(
                    egui::RichText::new(elide(&dir_text, 42))
                        .size(11.0)
                        .color(MUTED),
                )
                .on_hover_text(&dir_text);
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("更改…").size(12.0))
                            .min_size(egui::vec2(56.0, 30.0)),
                    )
                    .clicked()
                {
                    self.choose_dir();
                }
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("打开").size(12.0))
                            .min_size(egui::vec2(56.0, 30.0)),
                    )
                    .clicked()
                {
                    win::open_folder(&self.root.to_string_lossy());
                }
            });
            ui.add_space(6.0);
        });

        // ---------- 状态栏 ----------
        egui::Panel::bottom("status").exact_size(34.0).show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (n, mb) = self.selected_count();
                let protected_n = self.rows.iter().filter(|r| r.protect).count();
                ui.label(
                    egui::RichText::new(format!(
                        "线外 {} 项 · 将处理 {} · 已保护 {} · 系统关键 {}",
                        self.rows.len(),
                        n,
                        protected_n,
                        self.skipped.len()
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                if mb > 0.0 {
                    ui.label(
                        egui::RichText::new(format!("可释放约 {}", fmt_mb(mb)))
                            .size(12.0)
                            .color(GREEN),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.status.is_empty() {
                        ui.label(egui::RichText::new(&self.status).size(12.0).color(ACCENT));
                    }
                });
            });
        });

        // ---------- 列表 ----------
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let rows = std::mem::take(&mut self.rows);
            let mut toggled: Vec<(usize, bool)> = Vec::new();
            let search = self.search.to_lowercase();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut last_tier: Option<Tier> = None;
                    for (i, r) in rows.iter().enumerate() {
                        if !search.is_empty() && !r.name.to_lowercase().contains(&search) {
                            continue;
                        }
                        if last_tier != Some(r.tier) {
                            last_tier = Some(r.tier);
                            let (title, color) = match r.tier {
                                Tier::Suggested => ("建议处理 · 第三方程序", AMBER),
                                Tier::Caution => ("谨慎处理 · Windows 组件", GREY),
                                Tier::Blocked => ("禁止处理 · 关键系统组件", GREY),
                            };
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(title).size(12.0).color(color));
                            ui.add_space(2.0);
                        }

                        let dim = r.tier == Tier::Caution && !self.include_caution;
                        let fill = if r.protect { CARD_HL } else { CARD };
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0, LINE))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let mut p = r.protect;
                                    if ui.checkbox(&mut p, "").changed() {
                                        toggled.push((i, p));
                                    }
                                    let tag = if r.kind == Kind::Process { "进程" } else { "服务" };
                                    ui.label(egui::RichText::new(tag).size(11.0).color(MUTED));
                                    ui.label(
                                        egui::RichText::new(&r.name)
                                            .size(14.0)
                                            .strong()
                                            .color(if r.protect { MUTED } else { TEXT }),
                                    );
                                    if r.protect {
                                        ui.label(
                                            egui::RichText::new("已保护").size(10.5).color(GREEN),
                                        );
                                    }
                                    if dim {
                                        ui.label(
                                            egui::RichText::new("需勾选「包含系统组件」")
                                                .size(10.5)
                                                .color(GREY),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(fmt_mb(r.mem_mb))
                                                    .size(13.0)
                                                    .color(if r.mem_mb > 200.0 { RED } else { MUTED }),
                                            );
                                            if r.instances > 1 {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{} 实例",
                                                        r.instances
                                                    ))
                                                    .size(11.0)
                                                    .color(GREY),
                                                );
                                            }
                                        },
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&r.reason).size(11.0).color(GREY),
                                    );
                                    if !r.path.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&r.path).size(10.5).color(GREY),
                                        );
                                    }
                                });
                            });
                        ui.add_space(4.0);
                    }

                    if !self.skipped.is_empty() {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!(
                                "已自动跳过 {} 项（自身进程 / 关键系统组件，永不处理）",
                                self.skipped.len()
                            ))
                            .size(11.5)
                            .color(GREY),
                        );
                        for s in self.skipped.iter().take(12) {
                            ui.label(
                                egui::RichText::new(format!(
                                    "  · {}  {}",
                                    s.name,
                                    fmt_mb(s.mem_mb)
                                ))
                                .size(11.0)
                                .color(GREY),
                            );
                        }
                    }
                });

            self.rows = rows;
            if !toggled.is_empty() {
                for (i, v) in toggled {
                    if let Some(r) = self.rows.get_mut(i) {
                        r.protect = v;
                    }
                }
                self.save_whitelist();
            }
        });

        // ---------- 确认弹窗 ----------
        if self.confirm {
            let (n, mb) = self.selected_count();
            let mut do_it = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("confirm_modal"))
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .stroke(egui::Stroke::new(1.0, LINE))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(20, 18)),
                )
                .show(&ctx, |ui| {
                    // 手动铺底：这一层里 Frame 的 fill 不生效，直接画矩形最稳
                    let bg = ui.max_rect().expand2(egui::vec2(20.0, 18.0));
                    ui.painter().rect_filled(bg, egui::CornerRadius::same(10), PANEL);
                    painter_outline(ui, bg);
                    ui.set_width(460.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("确认恢复到基准状态？")
                            .size(17.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "将终止 {} 项基线之外的进程/服务，预计释放 {}。",
                            n,
                            fmt_mb(mb)
                        ))
                        .size(13.0),
                    );
                    ui.label(
                        egui::RichText::new("未保存的数据可能丢失；被系统判定为必需的会自己重启。")
                            .size(11.5)
                            .color(AMBER),
                    );
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("受影响的项目：").size(11.5).color(MUTED));
                    let mut shown = 0;
                    for r in &self.rows {
                        if shown >= 10 {
                            break;
                        }
                        if r.protect || r.tier == Tier::Blocked {
                            continue;
                        }
                        if r.tier == Tier::Caution && !self.include_caution {
                            continue;
                        }
                        ui.label(
                            egui::RichText::new(format!("  · {} {}", r.name, fmt_mb(r.mem_mb)))
                                .size(11.5)
                                .color(TEXT),
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
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(RED)
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
                        .rect_filled(bg, egui::CornerRadius::same(10), PANEL);
                    painter_outline(ui, bg);
                    ui.set_width(430.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("把当前状态设为基准？")
                            .size(17.0)
                            .strong()
                            .color(egui::Color32::WHITE),
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
                        .color(AMBER),
                    );
                    ui.label(
                        egui::RichText::new("想留住原来的开机基准，请先点「导出基准」备份。")
                            .size(11.5)
                            .color(MUTED),
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
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(ACCENT)
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
    }
}

fn main() -> eframe::Result<()> {
    let icon = egui::IconData {
        rgba: include_bytes!("../../../assets/icon_256.rgba").to_vec(),
        width: 256,
        height: 256,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "MemBase",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
