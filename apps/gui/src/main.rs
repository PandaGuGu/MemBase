//! MemBase GUI —— 开机内存基准线 · 一键复位
//!
//! 交互：列表里勾选 = 保护该项；点「恢复基准」把没勾选的线外项全部关掉。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use membase_core as core;
use membase_core::{Kind, Tier};
use membase_win::task::{self as boot, TaskInfo};
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
    /// 记进保护名单：永不处理
    protect: bool,
    /// 命中的保护条目（`进程:MuMu*` 这类通配也能命中）
    protect_by: String,
    /// 本次要恢复（终止/停止）—— 行右侧的勾选框
    pick: bool,
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

        for r in &self.rows {
            if !r.pick || !can_pick(r, self.include_caution) {
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

    /// 计划任务执行的是 CLI（参数 `collect ...`），所以优先找同目录的 CLI。
    /// 注意 Windows 文件名不区分大小写：`MemBase.exe` 和 `membase.exe` 是同一个文件，
    /// 所以分发时命令行版叫 `membase-cli.exe`；`membase.exe` 也认（放在别的目录时）。
    fn cli_exe(&self) -> Option<PathBuf> {
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Ok(cur) = std::env::current_exe() {
            let named_cli = cur
                .file_name()
                .map(|n| n.to_string_lossy().eq_ignore_ascii_case("membase.exe"))
                .unwrap_or(false);
            if named_cli {
                cands.push(cur.clone());
            }
            if let Some(dir) = cur.parent() {
                cands.push(dir.join("membase.exe"));
                cands.push(dir.join("membase-cli.exe"));
            }
        }
        cands.push(self.root.join("membase.exe"));
        cands
            .into_iter()
            .find(|p| p.is_file() && !boot::is_gui_exe(p))
    }

    /// 注册开机采集任务：拷一份 CLI 到数据目录，再挂计划任务
    fn install_boot(&mut self) {
        let src = match self.cli_exe() {
            Some(p) => p,
            None => {
                self.status =
                    "找不到命令行版 membase.exe：请把 CLI 和界面版放在同一目录".to_string();
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
                ui.separator();

                // ---- 批量操作只保留两个按钮：文字写的永远是「点下去会发生什么」 ----
                let (pickable, picked) = self.pick_stats();
                let all_picked = pickable > 0 && picked == pickable;
                let pick_label = if pickable == 0 {
                    "无可勾选项".to_string()
                } else if all_picked {
                    "清空勾选".to_string()
                } else {
                    format!("全选 {} 项", pickable)
                };
                let pick_btn = egui::Button::new(egui::RichText::new(pick_label).size(13.0))
                    .min_size(egui::vec2(88.0, 32.0));
                if ui
                    .add_enabled(pickable > 0, pick_btn)
                    .on_hover_text(if all_picked {
                        "取消所有勾选：这一轮什么都不恢复"
                    } else {
                        "勾选所有可处理项；已保护的和系统组件不受影响"
                    })
                    .clicked()
                {
                    self.set_all_pick(!all_picked);
                }

                if ui
                    .checkbox(&mut self.include_caution, "含系统组件")
                    .on_hover_text("放开「谨慎处理」那一栏，让它们也能被勾选；单行仍可自己改")
                    .changed()
                {
                    let on = self.include_caution;
                    for r in self.rows.iter_mut() {
                        if r.tier == Tier::Caution && !r.protect {
                            r.pick = on;
                        }
                    }
                }

                ui.separator();

                let protected_n = self.protected_count();
                let all_protected = !self.rows.is_empty() && protected_n == self.rows.len();
                let (p_label, p_hint, p_color) = if protected_n == 0 {
                    (
                        "全部保护".to_string(),
                        "把列表里的项全部记进保护名单，以后永不处理（同时取消勾选）",
                        TEXT,
                    )
                } else if all_protected {
                    (
                        "取消全部保护".to_string(),
                        "把这些项从保护名单里去掉，恢复默认勾选；通配规则不受影响",
                        TEXT,
                    )
                } else {
                    (
                        "全部保护".to_string(),
                        "把剩下的也记进保护名单；想只清掉已有的保护，点旁边那个「已保护 N」",
                        TEXT,
                    )
                };
                let p_btn = egui::Button::new(egui::RichText::new(p_label).size(13.0).color(p_color))
                    .fill(if protected_n > 0 { CARD_HL } else { CARD })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if protected_n > 0 { GREEN } else { LINE },
                    ))
                    .min_size(egui::vec2(104.0, 32.0));
                if ui
                    .add_enabled(!self.rows.is_empty(), p_btn)
                    .on_hover_text(p_hint)
                    .clicked()
                {
                    self.set_all_protect(!all_protected);
                    self.status = if all_protected {
                        "已取消全部保护".to_string()
                    } else {
                        format!("已把 {} 项记为保护", self.rows.len())
                    };
                }

                // 已有保护但没保护全：给一个看得见出口的小按钮，点一下清空这几位
                if protected_n > 0 && !all_protected {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("已保护 {} · 清空", protected_n))
                                    .size(11.5)
                                    .color(GREEN),
                            )
                            .fill(PANEL)
                            .stroke(egui::Stroke::new(1.0, LINE))
                            .min_size(egui::vec2(0.0, 26.0)),
                        )
                        .on_hover_text("把这 %d 项从保护名单里去掉".replace("%d", &protected_n.to_string()))
                        .clicked()
                    {
                        self.set_all_protect(false);
                        self.status = "已取消全部保护".to_string();
                    }
                }

                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("搜索名字")
                        .desired_width(150.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (n, mb) = self.selected_count();
                    let enabled = n > 0 && self.is_admin;
                    let text = if n == 0 {
                        "未勾选任何项".to_string()
                    } else {
                        format!("恢复选中 {} 项 · {}", n, fmt_mb(mb))
                    };
                    let btn = egui::Button::new(
                        egui::RichText::new(text)
                            .size(13.5)
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if enabled { RED } else { CARD })
                    .min_size(egui::vec2(196.0, 34.0));
                    let resp = ui.add_enabled(enabled, btn);
                    let resp = if enabled {
                        resp.on_hover_text("只会处理右边勾了「恢复」的行")
                    } else if n == 0 {
                        resp.on_hover_text("先在列表里勾上要恢复的项")
                    } else {
                        resp.on_hover_text("需要以管理员身份运行")
                    };
                    if resp.clicked() {
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

            // 第三行：开机采集 —— 把 `membase install --at 60` 做成按钮
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("开机采集")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add(
                    egui::DragValue::new(&mut self.boot_at)
                        .range(5..=600)
                        .speed(5.0)
                        .prefix("开机第 ")
                        .suffix(" 秒"),
                )
                .on_hover_text("开机后等多久再快照；太早桌面的启动项还没起来");

                // 采集耗时：优先用基准里记的 CollectMs，老基准则用本次刷新的实测值
                let cost_text = match self.collect_ms {
                    Some(ms) => format!("采集约 {}", fmt_dur(ms)),
                    None => "采集耗时未知".to_string(),
                };
                ui.label(egui::RichText::new(cost_text).size(11.0).color(MUTED))
                    .on_hover_text(match self.collect_ms {
                        Some(ms) => format!(
                            "枚举全部进程与服务约 {} 毫秒，写盘另计（通常 1–3 毫秒）。\n数据来源：{}\n开机第 {} 秒开始，{}完成，任务随即自毁。",
                            ms,
                            if self.collect_from_base {
                                "基准快照里的记录"
                            } else {
                                "本次刷新实测（这份基准采集时还没记录耗时）"
                            },
                            self.boot_at,
                            finish_hint(ms)
                        ),
                        None => "还没有可用的耗时记录".to_string(),
                    });

                let can = self.is_admin;
                let label = if self.boot.installed {
                    "重新安排"
                } else {
                    "安排下次开机采集"
                };
                let btn = egui::Button::new(
                    egui::RichText::new(label)
                        .size(12.5)
                        .color(egui::Color32::WHITE),
                )
                .fill(if can { ACCENT } else { CARD })
                .min_size(egui::vec2(136.0, 30.0));
                let resp = ui.add_enabled(can, btn);
                let resp = if can {
                    resp.on_hover_text("注册一个 SYSTEM 权限的开机任务，采完自动销毁")
                } else {
                    resp.on_hover_text("需要以管理员身份运行才能挂开机任务")
                };
                if resp.clicked() {
                    self.confirm_boot = true;
                }

                if self.boot.installed
                    && ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("取消安排").size(12.5),
                            )
                            .min_size(egui::vec2(84.0, 30.0)),
                        )
                        .clicked()
                {
                    self.remove_boot();
                }

                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if self.boot.installed {
                            let at = if self.boot.at_sec >= 5 {
                                self.boot.at_sec
                            } else {
                                self.boot_at
                            };
                            ui.label(
                                egui::RichText::new(format!(
                                    "已安排：下次开机第 {} 秒采集",
                                    at
                                ))
                                .size(11.0)
                                .color(GREEN),
                            )
                            .on_hover_text(format!(
                                "采完自动移除任务与程序副本\n任务程序：{}",
                                self.boot.exe
                            ));
                        } else {
                            ui.label(
                                egui::RichText::new(
                                    "未安排：基准来自手动采集或上一次开机",
                                )
                                .size(11.0)
                                .color(GREY),
                            );
                        }
                    },
                );
            });
            ui.add_space(6.0);
        });

        // ---------- 状态栏 ----------
        egui::Panel::bottom("status").exact_size(34.0).show(ui, |ui| {
            ui.add_space(6.0);
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
            let mut picks: Vec<(usize, bool)> = Vec::new();
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
                        let fill = if r.protect {
                            BG
                        } else if r.pick {
                            CARD_HL
                        } else {
                            CARD
                        };
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0, LINE))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let mut p = r.protect;
                                    if ui
                                        .checkbox(&mut p, "保护")
                                        .on_hover_text("勾上 = 永不处理，并写进保护名单")
                                        .changed()
                                    {
                                        toggled.push((i, p));
                                    }
                                    let tag = if r.kind == Kind::Process { "进程" } else { "服务" };
                                    ui.label(egui::RichText::new(tag).size(11.0).color(MUTED));
                                    let name_color = if r.protect || !r.pick { MUTED } else { TEXT };
                                    ui.label(
                                        egui::RichText::new(&r.name)
                                            .size(14.0)
                                            .strong()
                                            .color(name_color),
                                    );
                                    if r.protect {
                                        let glob = r.protect_by
                                            != core::protect_key(r.kind, &r.name);
                                        ui.label(
                                            egui::RichText::new(if glob {
                                                "通配保护"
                                            } else {
                                                "已保护"
                                            })
                                            .size(10.5)
                                            .color(GREEN),
                                        )
                                        .on_hover_text(format!(
                                            "保护名单条目：{}",
                                            r.protect_by
                                        ));
                                    }
                                    if dim {
                                        ui.label(
                                            egui::RichText::new("系统组件 · 默认不选")
                                                .size(10.5)
                                                .color(GREY),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // 右侧勾选框：勾上 = 本次恢复（终止进程 / 停止服务）
                                            let mut pick = r.pick;
                                            let resp = ui.add_enabled(
                                                can_pick(r, self.include_caution),
                                                egui::Checkbox::new(&mut pick, "恢复"),
                                            );
                                            if resp.changed() {
                                                picks.push((i, pick));
                                            }
                                            resp.on_hover_text(if r.protect {
                                                "已保护的项目不会被处理"
                                            } else if dim {
                                                "系统组件：先勾上工具栏的「含系统组件」"
                                            } else {
                                                "勾上 = 本次恢复时终止它"
                                            });
                                            ui.label(
                                                egui::RichText::new(fmt_mb(r.mem_mb))
                                                    .size(13.0)
                                                    .color(if r.mem_mb > 200.0 && r.pick {
                                                        RED
                                                    } else {
                                                        MUTED
                                                    }),
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
                        if v {
                            r.pick = false;
                        }
                    }
                }
                self.save_whitelist();
            }
            for (i, v) in picks {
                if let Some(r) = self.rows.get_mut(i) {
                    r.pick = v && !r.protect;
                }
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
                            "将终止你勾选的 {} 项进程/服务，预计释放 {}。",
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
                        if !r.pick || !can_pick(r, self.include_caution) {
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

        // ---------- 开机采集确认 ----------
        if self.confirm_boot {
            let mut do_it = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("boot_modal"))
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 18)))
                .show(&ctx, |ui| {
                    let bg = ui.max_rect().expand2(egui::vec2(20.0, 18.0));
                    ui.painter()
                        .rect_filled(bg, egui::CornerRadius::same(10), PANEL);
                    painter_outline(ui, bg);
                    ui.set_width(450.0);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("安排下次开机采集？")
                            .size(17.0)
                            .strong()
                            .color(egui::Color32::WHITE),
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
                        .color(MUTED),
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
                            .color(MUTED),
                        );
                    }
                    ui.label(
                        egui::RichText::new(
                            "现在开着的程序不会算进这份基准 —— 那才是「干净开机」的样子。",
                        )
                        .size(11.5)
                        .color(AMBER),
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
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(ACCENT)
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
