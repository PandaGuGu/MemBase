//! 纯逻辑层：分级、CSV 解析、diff、线外计划、基准线打包。
//!
//! 这一层不依赖任何平台 API、不含 unsafe，可以在 Windows / Linux / 容器里直接跑测试。
//! 平台相关的东西在 membase-win，命令行在 apps/cli，界面在 apps/gui。

pub mod bundle;
pub mod config;

/// crate 版本（用于 CLI 自描述与包 manifest）
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 默认数据目录
pub const DEFAULT_ROOT: &str = r"C:\ProgramData\MemBaseline";
/// 默认计划任务名
pub const DEFAULT_TASK: &str = "MemBaseline_CollectOnBoot";

/// 数据目录解析顺序：
///   1. 环境变量 MEMBASE_ROOT
///   2. %ProgramData%\MemBaseline（系统目录被重定向时也正确）
///   3. 内置默认值（跑在没有 ProgramData 的环境时兜底）
/// 命令行参数 --out 的优先级高于以上全部。
pub fn default_root() -> String {
    if let Ok(v) = std::env::var("MEMBASE_ROOT") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        let pd = pd.trim().trim_end_matches('\\');
        if !pd.is_empty() {
            return format!("{}\\MemBaseline", pd);
        }
    }
    DEFAULT_ROOT.to_string()
}

/// 计划任务名同样支持 MEMBASE_TASK 覆盖
pub fn default_task() -> String {
    if let Ok(v) = std::env::var("MEMBASE_TASK") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    DEFAULT_TASK.to_string()
}

use std::collections::BTreeMap;

// ---------------- 保护名单 ----------------

pub const CRITICAL_PROCS: &[&str] = &[
    "System",
    "Idle",
    "Registry",
    "Memory Compression",
    "Secure System",
    "csrss",
    "smss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "lsaiso",
    "svchost",
    "dwm",
    "fontdrvhost",
    "LogonUI",
    "UserOOBEBroker",
    "MsMpEng",
    "NisSrv",
    "MpDefenderCoreService",
    "MsSense",
    "SecurityHealthService",
    "SecurityHealthSystray",
    "SgrmBroker",
    "audiodg",
    "spoolsv",
    "sihost",
    "WUDFHost",
    "WmiPrvSE",
    "TrustedInstaller",
    "TiWorker",
];

pub const CAUTION_NAMES: &[&str] = &[
    "explorer",
    "taskhostw",
    "RuntimeBroker",
    "dllhost",
    "conhost",
    "ctfmon",
    "TextInputHost",
    "StartMenuExperienceHost",
    "SearchHost",
    "ShellExperienceHost",
    "ApplicationFrameHost",
    "SearchIndexer",
    "SearchProtocolHost",
    "SearchFilterHost",
    "backgroundTaskHost",
    "LockApp",
    "ShellHost",
    "smartscreen",
    "StartMenuHost",
    "Widgets",
    "WidgetService",
    "PhoneExperienceHost",
    "SystemSettings",
    "cmd",
    "powershell",
    "pwsh",
    "WindowsTerminal",
    "OpenConsole",
];

pub const CRITICAL_SERVICES: &[&str] = &[
    "RpcSs",
    "RpcEptMapper",
    "DcomLaunch",
    "PlugPlay",
    "Power",
    "ProfSvc",
    "Schedule",
    "EventLog",
    "Winmgmt",
    "WinDefend",
    "WdNisSvc",
    "Sense",
    "SecurityHealthService",
    "Audiosrv",
    "AudioEndpointBuilder",
    "Dhcp",
    "Dnscache",
    "NlaSvc",
    "netprofm",
    "WlanSvc",
    "LSM",
    "SamSs",
    "KeyIso",
    "CryptSvc",
    "gpsvc",
    "UserManager",
    "UsoSvc",
    "wuauserv",
    "BITS",
    "TrustedInstaller",
    "BrokerInfrastructure",
    "SystemEventsBroker",
    "CoreMessagingRegistrar",
    "StateRepository",
    "Themes",
    "mpssvc",
    "BFE",
    "WinRM",
    "SENS",
    "SysMain",
    "EventSystem",
    "WdiServiceHost",
    "hns",
    "LanmanServer",
    "LanmanWorkstation",
    "lmhosts",
    "DispBrokerDesktopSvc",
    "CDPSvc",
    "TextInputManagementService",
    "TabletInputService",
    "TokenBroker",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Process,
    Service,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// 第三方程序，默认档
    Suggested,
    /// Windows 组件，需要显式勾选「包含系统组件」
    Caution,
    /// 关键系统组件，永远不动
    Blocked,
}

fn contains_ci(list: &[&str], name: &str) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(name))
}

pub fn tier_for(kind: Kind, name: &str, path: &str) -> Tier {
    match kind {
        Kind::Process => {
            if contains_ci(CRITICAL_PROCS, name) {
                Tier::Blocked
            } else if contains_ci(CAUTION_NAMES, name) {
                Tier::Caution
            } else if path.trim().is_empty() {
                Tier::Caution
            } else {
                let lp = path.to_ascii_lowercase();
                if lp.contains(":\\windows\\") || lp.contains("\\windowsapps\\") {
                    Tier::Caution
                } else {
                    Tier::Suggested
                }
            }
        }
        Kind::Service => {
            if contains_ci(CRITICAL_SERVICES, name) {
                Tier::Blocked
            } else {
                Tier::Suggested
            }
        }
    }
}

// ---------------- 通配符 ----------------

pub fn wildcard_match(text: &str, pat: &str) -> bool {
    let s: Vec<char> = text.to_ascii_lowercase().chars().collect();
    let p: Vec<char> = pat.to_ascii_lowercase().chars().collect();
    fn go(s: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        match p[0] {
            '*' => go(s, &p[1..]) || (!s.is_empty() && go(&s[1..], p)),
            '?' => !s.is_empty() && go(&s[1..], &p[1..]),
            c => !s.is_empty() && s[0] == c && go(&s[1..], &p[1..]),
        }
    }
    if !pat.contains('*') && !pat.contains('?') {
        return text.eq_ignore_ascii_case(pat);
    }
    go(&s, &p)
}

// ---------------- CSV ----------------

pub fn csv_parse(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut row: Vec<String> = Vec::new();
    let mut in_q = false;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if in_q {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    it.next();
                    field.push('"');
                } else {
                    in_q = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => in_q = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                }
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    if !(row.len() == 1 && row[0].is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

pub fn col_index(header: &[String], name: &str) -> Option<usize> {
    header.iter().position(|h| h.eq_ignore_ascii_case(name))
}

// ---------------- 基准数据 / diff ----------------

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcAgg {
    pub instances: u32,
    pub ws_mb: f64,
    pub priv_mb: f64,
}

pub fn parse_by_name(text: &str) -> BTreeMap<String, ProcAgg> {
    let mut map = BTreeMap::new();
    let rows = csv_parse(text);
    if rows.is_empty() {
        return map;
    }
    let h = &rows[0];
    let (i_n, i_i, i_w, i_p) = (
        col_index(h, "ProcessName"),
        col_index(h, "Instances"),
        col_index(h, "WorkingSetMB"),
        col_index(h, "PrivateMB"),
    );
    if let (Some(a), Some(b), Some(c), Some(d)) = (i_n, i_i, i_w, i_p) {
        for r in rows.iter().skip(1) {
            let name = r.get(a).cloned().unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            map.insert(
                name,
                ProcAgg {
                    instances: r.get(b).and_then(|v| v.parse().ok()).unwrap_or(0),
                    ws_mb: r.get(c).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    priv_mb: r.get(d).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                },
            );
        }
    }
    map
}

pub fn parse_service_states(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let rows = csv_parse(text);
    if rows.is_empty() {
        return map;
    }
    let h = &rows[0];
    if let (Some(a), Some(b)) = (col_index(h, "ServiceName"), col_index(h, "State")) {
        for r in rows.iter().skip(1) {
            let n = r.get(a).cloned().unwrap_or_default();
            if n.is_empty() {
                continue;
            }
            map.insert(n, r.get(b).cloned().unwrap_or_default());
        }
    }
    map
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeltaKind {
    New,
    Gone,
    Up,
    Down,
    Same,
}

#[derive(Clone, Debug)]
pub struct ProcDelta {
    pub name: String,
    pub kind: DeltaKind,
    pub base_inst: u32,
    pub live_inst: u32,
    pub base_ws: f64,
    pub live_ws: f64,
    pub delta: f64,
}

pub fn diff_procs(
    base: &BTreeMap<String, ProcAgg>,
    live: &BTreeMap<String, ProcAgg>,
) -> Vec<ProcDelta> {
    let mut names: Vec<&String> = base.keys().collect();
    for k in live.keys() {
        if !base.contains_key(k) {
            names.push(k);
        }
    }
    let mut out: Vec<ProcDelta> = names
        .into_iter()
        .map(|n| {
            let b = base.get(n).copied().unwrap_or_default();
            let l = live.get(n).copied().unwrap_or_default();
            let delta = l.ws_mb - b.ws_mb;
            let kind = if b.instances == 0 {
                DeltaKind::New
            } else if l.instances == 0 {
                DeltaKind::Gone
            } else if delta > 0.5 {
                DeltaKind::Up
            } else if delta < -0.5 {
                DeltaKind::Down
            } else {
                DeltaKind::Same
            };
            ProcDelta {
                name: n.clone(),
                kind,
                base_inst: b.instances,
                live_inst: l.instances,
                base_ws: b.ws_mb,
                live_ws: l.ws_mb,
                delta: (delta * 100.0).round() / 100.0,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.delta
            .partial_cmp(&a.delta)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

#[derive(Clone, Debug)]
pub struct SvcChange {
    pub name: String,
    pub change: String,
    pub start_mode: String,
}

pub fn diff_services(
    base: &BTreeMap<String, String>,
    live: &BTreeMap<String, (String, String)>,
) -> Vec<SvcChange> {
    let mut out = Vec::new();
    for (name, (state, mode)) in live.iter() {
        let b = base.get(name).map(|s| s.as_str()).unwrap_or("");
        if b == state {
            continue;
        }
        let change = if b.is_empty() {
            "新增服务".to_string()
        } else {
            format!("{} → {}", b, state)
        };
        out.push(SvcChange {
            name: name.clone(),
            change,
            start_mode: mode.clone(),
        });
    }
    out
}

// ---------------- 保护偏好持久化 ----------------

pub fn parse_whitelist(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

pub fn serialize_whitelist(items: &[String]) -> String {
    let mut s = String::from("# MemBase 保护名单（每行一项，格式 进程:名字 或 服务:名字）\n");
    let mut v = items.to_vec();
    v.sort();
    for i in v {
        s.push_str(&i);
        s.push('\n');
    }
    s
}

pub fn protect_key(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Process => format!("进程:{}", name),
        Kind::Service => format!("服务:{}", name),
    }
}

// ---------------- 线外清单（GUI 和 CLI 共用同一套判定） ----------------

#[derive(Clone, Debug)]
pub struct LiveProc {
    pub name: String,
    pub pid: u32,
    pub ws_mb: f64,
    pub priv_mb: f64,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct LiveSvc {
    pub name: String,
    pub state: String,
    pub start_mode: String,
    pub pid: u32,
    pub ws_mb: f64,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct PlanItem {
    pub kind: Kind,
    pub name: String,
    pub pids: Vec<u32>,
    pub instances: u32,
    pub mem_mb: f64,
    pub path: String,
    pub tier: Tier,
    pub reason: String,
    pub protect: bool,
    pub start_mode: String,
}

impl PlanItem {
    pub fn actionable(&self, include_caution: bool) -> bool {
        if self.protect || self.tier == Tier::Blocked {
            return false;
        }
        self.tier == Tier::Suggested || include_caution
    }
}

/// 生成「基线之外」清单。
/// 规则与 GUI / PowerShell 版一致：
///   * 进程按名字判定，名字在基准里就不算线外
///   * 自身进程树（protected_pids）里的进程直接不出现
///   * 服务只算「现在 Running 且基准里不是 Running」的
///   * 服务启动方式不是 Manual 的降级到谨慎档
pub fn build_plan(
    base_procs: &BTreeMap<String, ProcAgg>,
    base_svc: &BTreeMap<String, String>,
    live_procs: &[LiveProc],
    live_svcs: &[LiveSvc],
    protected_pids: &[u32],
    whitelist: &[String],
    min_mb: f64,
) -> Vec<PlanItem> {
    let mut agg: BTreeMap<String, (u32, f64, Vec<u32>, String)> = BTreeMap::new();
    for p in live_procs {
        let e = agg
            .entry(p.name.clone())
            .or_insert((0, 0.0, Vec::new(), String::new()));
        e.0 += 1;
        e.1 += p.ws_mb;
        e.2.push(p.pid);
        if e.3.is_empty() && !p.path.is_empty() {
            e.3 = p.path.clone();
        }
    }

    let mut out: Vec<PlanItem> = Vec::new();

    for (name, (inst, ws, pids, path)) in agg {
        if base_procs.contains_key(&name) {
            continue;
        }
        if pids.iter().any(|p| protected_pids.contains(p)) {
            continue;
        }
        let tier = tier_for(Kind::Process, &name, &path);
        if tier == Tier::Suggested && ws < min_mb {
            continue;
        }
        out.push(PlanItem {
            kind: Kind::Process,
            name: name.clone(),
            pids,
            instances: inst,
            mem_mb: ws,
            path,
            tier,
            reason: "基准里没有这个进程".to_string(),
            protect: whitelist.contains(&protect_key(Kind::Process, &name)),
            start_mode: String::new(),
        });
    }

    for s in live_svcs {
        if s.state != "Running" {
            continue;
        }
        if base_svc.get(&s.name).map(|x| x.as_str()) == Some("Running") {
            continue;
        }
        let mut tier = tier_for(Kind::Service, &s.name, &s.path);
        if tier != Tier::Blocked && s.start_mode != "Manual" {
            tier = Tier::Caution;
        }
        out.push(PlanItem {
            kind: Kind::Service,
            name: s.name.clone(),
            pids: if s.pid > 0 { vec![s.pid] } else { Vec::new() },
            instances: 1,
            mem_mb: s.ws_mb,
            path: s.path.clone(),
            tier,
            reason: format!("开机时未运行 · 启动方式 {}", s.start_mode),
            protect: whitelist.contains(&protect_key(Kind::Service, &s.name)),
            start_mode: s.start_mode.clone(),
        });
    }

    out.sort_by(|a, b| {
        let rank = |t: Tier| match t {
            Tier::Suggested => 0,
            Tier::Caution => 1,
            Tier::Blocked => 2,
        };
        rank(a.tier).cmp(&rank(b.tier)).then(
            b.mem_mb
                .partial_cmp(&a.mem_mb)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    out
}

pub fn plan_actionable(plan: &[PlanItem], include_caution: bool) -> Vec<&PlanItem> {
    plan.iter().filter(|p| p.actionable(include_caution)).collect()
}

pub fn plan_totals(plan: &[PlanItem], include_caution: bool) -> (usize, f64) {
    let items = plan_actionable(plan, include_caution);
    let mb = items.iter().map(|p| p.mem_mb).sum();
    (items.len(), mb)
}

pub fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Suggested => "suggested",
        Tier::Caution => "caution",
        Tier::Blocked => "blocked",
    }
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Process => "process",
        Kind::Service => "service",
    }
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn agg(i: u32, ws: f64) -> ProcAgg {
        ProcAgg {
            instances: i,
            ws_mb: ws,
            priv_mb: ws / 2.0,
        }
    }

    #[test]
    fn critical_process_is_blocked_even_with_normal_path() {
        assert_eq!(
            tier_for(Kind::Process, "lsass", "C:\\Windows\\System32\\lsass.exe"),
            Tier::Blocked
        );
        assert_eq!(tier_for(Kind::Process, "svchost", ""), Tier::Blocked);
    }

    #[test]
    fn third_party_is_suggested() {
        assert_eq!(
            tier_for(Kind::Process, "HYP", "H:\\miHoYo Launcher\\HYP.exe"),
            Tier::Suggested
        );
    }

    #[test]
    fn windows_component_is_caution() {
        assert_eq!(
            tier_for(Kind::Process, "taskhostw", "C:\\Windows\\System32\\taskhostw.exe"),
            Tier::Caution
        );
        // UWP 组件也在负责档
        assert_eq!(
            tier_for(Kind::Process, "SearchHost", "C:\\Windows\\SystemApps\\x\\SearchHost.exe"),
            Tier::Caution
        );
    }

    #[test]
    fn unknown_path_is_conservative() {
        assert_eq!(tier_for(Kind::Process, "mystery", ""), Tier::Caution);
    }

    #[test]
    fn critical_service_is_blocked() {
        assert_eq!(tier_for(Kind::Service, "RpcSs", ""), Tier::Blocked);
        assert_eq!(tier_for(Kind::Service, "SensorService", ""), Tier::Suggested);
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("MuMuRemoteBackend", "MuMu*"));
        assert!(wildcard_match("sandbox-cli-gc", "sandbox-*-gc"));
        assert!(wildcard_match("HYP", "hyp"));
        assert!(!wildcard_match("HYPHelper", "HYP"));
        assert!(wildcard_match("a1b", "a?b"));
    }

    #[test]
    fn csv_handles_quotes_and_commas() {
        let text = "Name,Path\r\nfoo,\"C:\\a,b\\x.exe\"\r\nbar,\"say \"\"hi\"\"\"\r\n";
        let rows = csv_parse(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "C:\\a,b\\x.exe");
        assert_eq!(rows[2][1], "say \"hi\"");
    }

    #[test]
    fn csv_strips_bom_and_empty_lines() {
        let text = "\u{feff}A,B\n1,2\n\n3,4\n";
        let rows = csv_parse(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], "A");
    }

    #[test]
    fn diff_marks_new_gone_up_down() {
        let mut base = BTreeMap::new();
        base.insert("stay".to_string(), agg(1, 100.0));
        base.insert("gone".to_string(), agg(1, 50.0));
        base.insert("up".to_string(), agg(1, 20.0));

        let mut live = BTreeMap::new();
        live.insert("stay".to_string(), agg(1, 100.0));
        live.insert("up".to_string(), agg(2, 80.0));
        live.insert("brandnew".to_string(), agg(3, 200.0));

        let d = diff_procs(&base, &live);
        let find = |n: &str| d.iter().find(|x| x.name == n).unwrap();
        assert_eq!(find("brandnew").kind, DeltaKind::New);
        assert_eq!(find("gone").kind, DeltaKind::Gone);
        assert_eq!(find("up").kind, DeltaKind::Up);
        assert_eq!(find("up").delta, 60.0);
        assert_eq!(find("stay").kind, DeltaKind::Same);
        // 排序：增长最多的排最前
        assert_eq!(d[0].name, "brandnew");
    }

    #[test]
    fn service_diff_reports_state_change() {
        let mut base = BTreeMap::new();
        base.insert("UsoSvc".to_string(), "Stopped".to_string());
        base.insert("Spooler".to_string(), "Running".to_string());

        let mut live = BTreeMap::new();
        live.insert(
            "UsoSvc".to_string(),
            ("Running".to_string(), "Auto".to_string()),
        );
        live.insert(
            "Spooler".to_string(),
            ("Running".to_string(), "Auto".to_string()),
        );

        let c = diff_services(&base, &live);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "UsoSvc");
        assert!(c[0].change.contains("Stopped"));
    }

    #[test]
    fn whitelist_roundtrip() {
        let items = vec!["进程:chrome".to_string(), "服务:SensorService".to_string()];
        let text = serialize_whitelist(&items);
        let back = parse_whitelist(&text);
        assert_eq!(back.len(), 2);
        assert!(back.contains(&"进程:chrome".to_string()));
    }

    #[test]
    fn protect_key_is_stable() {
        assert_eq!(protect_key(Kind::Process, "chrome"), "进程:chrome");
        assert_eq!(protect_key(Kind::Service, "DPS"), "服务:DPS");
    }

    // ---- build_plan ----

    fn lp(name: &str, pid: u32, ws: f64, path: &str) -> LiveProc {
        LiveProc {
            name: name.into(),
            pid,
            ws_mb: ws,
            priv_mb: ws / 2.0,
            path: path.into(),
        }
    }

    fn ls(name: &str, state: &str, mode: &str, pid: u32, ws: f64) -> LiveSvc {
        LiveSvc {
            name: name.into(),
            state: state.into(),
            start_mode: mode.into(),
            pid,
            ws_mb: ws,
            path: String::new(),
        }
    }

    #[test]
    fn plan_skips_baseline_and_protected() {
        let mut base = BTreeMap::new();
        base.insert("explorer".to_string(), agg(1, 200.0));

        let live = vec![
            lp("explorer", 100, 220.0, "C:\\Windows\\explorer.exe"), // 基准里有
            lp("WorkBuddy", 200, 500.0, "F:\\app\\WB.exe"),          // 受保护
            lp("HYP", 300, 300.0, "H:\\miHoYo\\HYP.exe"),            // 线外
        ];
        let plan = build_plan(&base, &BTreeMap::new(), &live, &[], &[200], &[], 10.0);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "HYP");
        assert_eq!(plan[0].tier, Tier::Suggested);
    }

    #[test]
    fn plan_aggregates_instances() {
        let live = vec![
            lp("chrome", 1, 100.0, "C:\\Program Files\\chrome.exe"),
            lp("chrome", 2, 50.0, "C:\\Program Files\\chrome.exe"),
        ];
        let plan = build_plan(&BTreeMap::new(), &BTreeMap::new(), &live, &[], &[], &[], 1.0);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].instances, 2);
        assert_eq!(plan[0].mem_mb, 150.0);
        assert_eq!(plan[0].pids.len(), 2);
    }

    #[test]
    fn plan_service_needs_to_be_running() {
        let mut base = BTreeMap::new();
        base.insert("Spooler".to_string(), "Running".to_string());
        let live = vec![
            ls("Spooler", "Running", "Auto", 0, 10.0),   // 基准就在跑
            ls("WbioSrvc", "Stopped", "Auto", 0, 10.0),  // 没在跑
            ls("SensorService", "Running", "Manual", 0, 29.0),
        ];
        let plan = build_plan(&BTreeMap::new(), &base, &[], &live, &[], &[], 1.0);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "SensorService");
        assert_eq!(plan[0].tier, Tier::Suggested);
    }

    #[test]
    fn plan_auto_service_becomes_caution() {
        let live = vec![ls("CDPUserSvc_abc", "Running", "Auto", 0, 50.0)];
        let plan = build_plan(&BTreeMap::new(), &BTreeMap::new(), &[], &live, &[], &[], 1.0);
        assert_eq!(plan[0].tier, Tier::Caution);
    }

    #[test]
    fn plan_min_mb_only_filters_suggested() {
        let live = vec![
            lp("tiny", 1, 3.0, "D:\\tool\\tiny.exe"),
            lp("big", 2, 300.0, "D:\\tool\\big.exe"),
            lp("taskhostw", 3, 1.0, "C:\\Windows\\System32\\taskhostw.exe"),
        ];
        let plan = build_plan(&BTreeMap::new(), &BTreeMap::new(), &live, &[], &[], &[], 20.0);
        let names: Vec<&str> = plan.iter().map(|p| p.name.as_str()).collect();
        assert!(!names.contains(&"tiny"));
        assert!(names.contains(&"big"));
        // 谨慎档不受 min_mb 影响
        assert!(names.contains(&"taskhostw"));
    }

    #[test]
    fn plan_whitelist_and_totals() {
        let live = vec![
            lp("a", 1, 100.0, "D:\\a.exe"),
            lp("b", 2, 50.0, "D:\\b.exe"),
            lp("taskhostw", 3, 30.0, "C:\\Windows\\System32\\taskhostw.exe"),
        ];
        let wl = vec![protect_key(Kind::Process, "b")];
        let plan = build_plan(&BTreeMap::new(), &BTreeMap::new(), &live, &[], &[], &wl, 1.0);
        assert!(plan.iter().find(|p| p.name == "b").unwrap().protect);
        let (n, mb) = plan_totals(&plan, false);
        assert_eq!(n, 1);
        assert_eq!(mb, 100.0);
        let (n2, mb2) = plan_totals(&plan, true);
        assert_eq!(n2, 2);
        assert_eq!(mb2, 130.0);
    }

    #[test]
    fn blocked_items_are_never_actionable() {
        let live = vec![lp("lsass", 7, 40.0, "C:\\Windows\\System32\\lsass.exe")];
        let plan = build_plan(&BTreeMap::new(), &BTreeMap::new(), &live, &[], &[], &[], 1.0);
        assert_eq!(plan[0].tier, Tier::Blocked);
        assert!(!plan[0].actionable(true));
        assert_eq!(plan_actionable(&plan, true).len(), 0);
    }
}
