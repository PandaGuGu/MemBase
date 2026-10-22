//! 已知功能开关（表驱动，只写 HKCU）+ 证据统计 + 局部重启的护栏判定。
//!
//! 为什么是「表」而不是「任意注册表编辑器」：
//! 改注册表能修好的问题只是「系统功能默认开着」这一类，而随便开一个注册表编辑器
//! 给用户，等于把系统配置的责任转嫁出去。这里只收**想清楚了**的开关：
//!   * 每条都知道关掉什么、撤销时写回什么；
//!   * 只写 `HKCU`（当前用户），不碰 `HKLM` 策略；
//!   * 每条都配一个「证据进程」——功能活着它就在跑，功能真关掉它才该消失。
//!
//! 证据必须按**父进程链**归属：`msedgewebview2` 既可能是 Edge 的，也可能是
//! SearchHost 的。只按名字数会把 Edge 的渲染进程算成「搜索亮点」，那是最容易
//! 得出「关了没效果」假结论的地方。

/// 一个开关值：写哪个子键、哪个值名、关掉写什么、撤销写回什么。
pub struct FeatureValue {
    /// HKCU 下的子键路径
    pub key_path: &'static str,
    pub value: &'static str,
    /// 「关掉」要写的 DWORD
    pub off: u32,
    /// 「撤销」要写的 DWORD；`None` = 删掉这个值，回到系统默认
    pub on: Option<u32>,
}

/// 一个已知功能开关
pub struct FeatureSwitch {
    /// CLI 与界面用的稳定键
    pub key: &'static str,
    pub label: &'static str,
    pub values: &'static [FeatureValue],
    /// 证据进程名：功能活着它就在跑
    pub evidence_proc: &'static str,
    /// 证据进程的宿主名（沿父进程链向上找）；空串 = 证据进程自己就是宿主
    pub host_proc: &'static str,
    pub note: &'static str,
    /// 真机实测结论（空串 = 还没实测过）
    pub measured: &'static str,
}

const SEARCH_SETTINGS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\SearchSettings";
const MOBILITY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Mobility";

pub const FEATURE_SWITCHES: &[FeatureSwitch] = &[
    FeatureSwitch {
        key: "search-highlights",
        label: "搜索亮点（搜索框里的 Bing 网页内容）",
        values: &[FeatureValue {
            key_path: SEARCH_SETTINGS_KEY,
            value: "IsDynamicSearchBoxEnabled",
            off: 0,
            on: Some(1),
        }],
        evidence_proc: "msedgewebview2",
        host_proc: "SearchHost",
        note: "只写 HKCU；SearchHost 自己会被外壳拉起，改完不必手动启动",
        measured: "值已为 0，6 个 webview2 仍在；终止 SearchHost 后 2 秒自愈并重新拉起同样 6 个；而且 SearchSettings 子树连管理员写入都返回「拒绝访问」——这台机器上这个开关既不需要也改不动",
    },
    FeatureSwitch {
        key: "cross-device-resume",
        label: "跨设备继续（手机上继续）",
        values: &[FeatureValue {
            key_path: MOBILITY_KEY,
            value: "CrossDeviceEnabled",
            off: 0,
            on: Some(1),
        }],
        evidence_proc: "CrossDeviceResume",
        host_proc: "",
        note: "CBS 系统应用自带；关掉后要重启 CrossDeviceResume 才会消失",
        measured: "2026-10-22 复核：注册表值确为 0，但下次开机 T+60s 它又起来 70.7 MB —— 关开关只让它当场退出，不阻止自启，不能按「省 71 MB」算",
    },
];

pub fn feature_by_key(key: &str) -> Option<&'static FeatureSwitch> {
    FEATURE_SWITCHES.iter().find(|f| f.key.eq_ignore_ascii_case(key))
}

/// 所有开关键，给 `feature list` / 报错提示用
pub fn feature_keys() -> Vec<&'static str> {
    FEATURE_SWITCHES.iter().map(|f| f.key).collect()
}

// ---------------- 证据统计 ----------------

/// 进程树的一个节点（只带判定要用的字段）
#[derive(Clone, Debug)]
pub struct ProcNode {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    /// `None` = 没读到（未提权），不要当 0
    pub ws_mb: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeatureEvidence {
    /// 归属这个功能的实例数（父进程链能追到宿主）
    pub instances: u32,
    /// 这些实例的工作集合计（读不到的跳过）
    pub mem_mb: f64,
    /// 同名但归属别处的实例数（比如 Edge 自己的 webview2）
    pub foreign: u32,
    /// 宿主进程在不在跑
    pub host_running: bool,
}

const MAX_DEPTH: usize = 8;

/// 数这个功能还活着多少。
///
/// 归属判定：证据进程的父进程链上（最多 8 层）能追到 `host_proc` 才算数；
/// `host_proc` 为空时只看名字。追不到的计入 `foreign`，不参与内存合计——
/// 免得把用户的 Edge 算到「搜索亮点」头上。
pub fn feature_evidence(sw: &FeatureSwitch, procs: &[ProcNode]) -> FeatureEvidence {
    let by_pid: std::collections::HashMap<u32, &ProcNode> =
        procs.iter().map(|p| (p.pid, p)).collect();

    let mut ev = FeatureEvidence::default();
    let host_wanted = !sw.host_proc.is_empty();
    ev.host_running = host_wanted
        && procs
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(sw.host_proc));

    for p in procs {
        if !p.name.eq_ignore_ascii_case(sw.evidence_proc) {
            continue;
        }
        if host_wanted && !ancestor_matches(p, sw.host_proc, &by_pid) {
            ev.foreign += 1;
            continue;
        }
        ev.instances += 1;
        ev.mem_mb += p.ws_mb.unwrap_or(0.0);
    }
    ev.mem_mb = (ev.mem_mb * 100.0).round() / 100.0;
    ev
}

/// 沿父进程链向上找宿主。链断了（父进程已退出 / 拿到 PPID 0）就算追不到。
fn ancestor_matches(
    node: &ProcNode,
    host: &str,
    by_pid: &std::collections::HashMap<u32, &ProcNode>,
) -> bool {
    let mut cur = node.ppid;
    for _ in 0..MAX_DEPTH {
        if cur == 0 {
            return false;
        }
        let Some(p) = by_pid.get(&cur) else {
            return false;
        };
        if p.name.eq_ignore_ascii_case(host) {
            return true;
        }
        cur = p.ppid;
    }
    false
}

// ---------------- 开关状态与结论 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchState {
    /// 值存在且不是「关」
    On,
    /// 值已经是「关」
    Off,
    /// 值不存在（从没设过，走系统默认）
    Missing,
}

impl SwitchState {
    pub fn name(&self) -> &'static str {
        match self {
            SwitchState::On => "on",
            SwitchState::Off => "off",
            SwitchState::Missing => "default",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            SwitchState::On => "开着",
            SwitchState::Off => "已关",
            SwitchState::Missing => "未设置",
        }
    }
}

/// 开关状态 × 证据，得出「还能不能靠这个开关回收内存」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FeatureVerdict {
    /// 开关开着，证据在跑：关掉它有机会回收
    Active,
    /// 开关已经关了，实例还在：得重启宿主才能确认，别直接报「已解决」
    OffButAlive,
    /// 开关关着，证据也没有：已经在省了
    Off,
    /// 开关开着但没看到证据：可能没触发，也可能不是这个进程在撑
    Idle,
}

impl FeatureVerdict {
    pub fn name(&self) -> &'static str {
        match self {
            FeatureVerdict::Active => "active",
            FeatureVerdict::OffButAlive => "off_but_alive",
            FeatureVerdict::Off => "off",
            FeatureVerdict::Idle => "idle",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            FeatureVerdict::Active => "可关",
            FeatureVerdict::OffButAlive => "已关但仍在跑",
            FeatureVerdict::Off => "已关",
            FeatureVerdict::Idle => "没看到实例",
        }
    }
}

/// 多个值（一条开关可能对应几个注册表值）合并成一个状态：
/// 只要有一个还是「开」，整体就算开。
pub fn combine_state(states: &[SwitchState]) -> SwitchState {
    if states.contains(&SwitchState::On) {
        SwitchState::On
    } else if states.iter().all(|s| *s == SwitchState::Missing) {
        SwitchState::Missing
    } else {
        SwitchState::Off
    }
}

pub fn feature_verdict(state: SwitchState, ev: &FeatureEvidence) -> FeatureVerdict {
    match (state, ev.instances > 0) {
        (SwitchState::On, true) => FeatureVerdict::Active,
        (SwitchState::On, false) => FeatureVerdict::Idle,
        (_, true) => FeatureVerdict::OffButAlive,
        (_, false) => FeatureVerdict::Off,
    }
}

// ---------------- 局部重启的护栏 ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RestartKind {
    Process,
    Service,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RestartCheck {
    pub allowed: bool,
    pub reason: &'static str,
}

const OK: RestartCheck = RestartCheck {
    allowed: true,
    reason: "",
};

/// 判断这一项能不能「关掉再拉起」。
///
/// 局部重启只对「有关闭路径、能自己起来」的东西有意义：
///   * 关键系统组件（`Tier::Blocked`）一律不做；
///   * 进程必须有可执行路径，否则杀了就起不来（快照里 Path 常为空）；
///   * `svchost` 这类共享宿主不走进程重启，得走服务；
///   * `explorer` 允许，但要求显式 `--yes`（会闪一下任务栏、关掉已打开的文件夹）。
pub fn restart_check(kind: RestartKind, name: &str, path: &str, tier: crate::Tier) -> RestartCheck {
    if tier == crate::Tier::Blocked {
        return RestartCheck {
            allowed: false,
            reason: "关键系统组件，不做重启",
        };
    }
    match kind {
        RestartKind::Process => {
            if path.trim().is_empty() {
                return RestartCheck {
                    allowed: false,
                    reason: "拿不到可执行路径，关掉就起不来了",
                };
            }
            if name.eq_ignore_ascii_case("svchost")
                || name.eq_ignore_ascii_case("dllhost")
                || name.eq_ignore_ascii_case("WmiPrvSE")
                || name.eq_ignore_ascii_case("RuntimeBroker")
            {
                return RestartCheck {
                    allowed: false,
                    reason: "共享宿主进程，应按服务处理，不走进程重启",
                };
            }
            OK
        }
        RestartKind::Service => OK,
    }
}

/// 这一项是否必须显式 `--yes` 才动（不吃交互式确认）。
/// explorer 重启会关掉所有已打开的资源管理器窗口，属于「用户看得见」的动作。
pub fn restart_needs_yes(name: &str) -> bool {
    name.eq_ignore_ascii_case("explorer") || name.eq_ignore_ascii_case("SearchHost")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(pid: u32, ppid: u32, name: &str, ws: Option<f64>) -> ProcNode {
        ProcNode {
            pid,
            ppid,
            name: name.into(),
            ws_mb: ws,
        }
    }

    fn sw(key: &str) -> &'static FeatureSwitch {
        feature_by_key(key).expect("开关表里应该有这个键")
    }

    #[test]
    fn 开关表里的键唯一() {
        let mut keys = feature_keys();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
        assert!(feature_by_key("search-highlights").is_some());
    }

    #[test]
    fn 证据按父进程链归属() {
        // SearchHost(100) → webview2(200) → webview2(201/202) 子进程
        let procs = vec![
            node(100, 1, "SearchHost", Some(150.0)),
            node(200, 100, "msedgewebview2", Some(120.0)),
            node(201, 200, "msedgewebview2", Some(30.0)),
            node(202, 200, "msedgewebview2", Some(20.0)),
            // 另一个程序的 webview2，不该算进来
            node(300, 999, "msedgewebview2", Some(500.0)),
        ];
        let ev = feature_evidence(sw("search-highlights"), &procs);
        assert_eq!(ev.instances, 3);
        assert_eq!(ev.mem_mb, 170.0);
        assert_eq!(ev.foreign, 1);
        assert!(ev.host_running);
    }

    #[test]
    fn 读不到内存不冒充零() {
        let procs = vec![
            node(100, 1, "SearchHost", None),
            node(200, 100, "msedgewebview2", None),
        ];
        let ev = feature_evidence(sw("search-highlights"), &procs);
        assert_eq!(ev.instances, 1);
        assert_eq!(ev.mem_mb, 0.0);
    }

    #[test]
    fn 自证宿主时只看名字() {
        let procs = vec![
            node(400, 1, "CrossDeviceResume", Some(71.0)),
            node(401, 1, "CrossDeviceResume", Some(9.0)),
            node(500, 1, "SearchHost", Some(150.0)),
        ];
        let ev = feature_evidence(sw("cross-device-resume"), &procs);
        assert_eq!(ev.instances, 2);
        assert_eq!(ev.mem_mb, 80.0);
        assert_eq!(ev.foreign, 0);
    }

    #[test]
    fn 父进程已退出的孤儿实例仍按名字算() {
        // 宿主先退了、webview2 还赖着：链断在 999，但名字对得上且没有宿主要求
        let procs = vec![node(700, 999, "CrossDeviceResume", Some(5.0))];
        let ev = feature_evidence(sw("cross-device-resume"), &procs);
        assert_eq!(ev.instances, 1);
    }

    #[test]
    fn 结论区分已关但仍在跑() {
        let alive = FeatureEvidence {
            instances: 6,
            mem_mb: 369.0,
            foreign: 0,
            host_running: true,
        };
        let gone = FeatureEvidence::default();
        assert_eq!(feature_verdict(SwitchState::On, &alive), FeatureVerdict::Active);
        assert_eq!(feature_verdict(SwitchState::On, &gone), FeatureVerdict::Idle);
        assert_eq!(
            feature_verdict(SwitchState::Off, &alive),
            FeatureVerdict::OffButAlive
        );
        assert_eq!(feature_verdict(SwitchState::Missing, &gone), FeatureVerdict::Off);
    }

    #[test]
    fn 多值合并取最悲观() {
        assert_eq!(
            combine_state(&[SwitchState::Off, SwitchState::On]),
            SwitchState::On
        );
        assert_eq!(
            combine_state(&[SwitchState::Missing, SwitchState::Missing]),
            SwitchState::Missing
        );
        assert_eq!(
            combine_state(&[SwitchState::Missing, SwitchState::Off]),
            SwitchState::Off
        );
    }

    #[test]
    fn 重启护栏() {
        use crate::Tier;
        assert!(
            restart_check(RestartKind::Process, "chrome", r"C:\x\chrome.exe", Tier::Suggested)
                .allowed
        );
        // 没路径 → 关掉起不来
        assert!(!restart_check(RestartKind::Process, "ghost", "", Tier::Caution).allowed);
        // svchost 走服务
        assert!(!restart_check(RestartKind::Process, "svchost", r"C:\Windows\svchost.exe", Tier::Suggested).allowed);
        // 关键组件
        assert!(!restart_check(RestartKind::Process, "lsass", r"C:\Windows\lsass.exe", Tier::Blocked).allowed);
        assert!(restart_check(RestartKind::Service, "DPS", "", Tier::Suggested).allowed);
        assert!(restart_needs_yes("explorer"));
        assert!(!restart_needs_yes("notepad"));
    }
}
