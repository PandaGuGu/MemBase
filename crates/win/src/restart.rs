//! 局部重启：关掉再拉起，并**实测**回收了多少。
//!
//! 为什么要有这个：整机重启不可替代的只有内核池、句柄泄漏、驱动状态；
//! 可回收量的大头在**进程工作集**。explorer 跑一天能涨到 667 MB，重启后回到
//! 226 MB —— 这 440 MB 不需要重启整机就能拿回来。
//!
//! 三条硬规则：
//!   1. 关掉的动作复用 [`close`]，和 trim 完全同一套「先礼后兵 + 身份校验」；
//!   2. 只对**有可执行路径**的进程动手，杀了起不来的一律不做（判定在 core 里）；
//!   3. 报**实测值**：等进程起来、稳定下来再量一次工作集，
//!      不拿「重启前 − 理论值」糊弄人。回收量可能是负的（新进程冷启动更胖），如实写。
//!
//! 还有一个必须识别的现实：有些进程杀了会**自愈**（SearchHost 由外壳秒拉起，
//! 连带 6 个 webview2 一起回来）。这时不该再拉起一份，也不该报告成功，
//! 而要如实写「自愈重生，回收无效」。

use crate::close::{self, CloseOptions, Target};
use crate::win;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ProcInstance {
    pub pid: u32,
    pub created: u64,
    pub ws_mb: Option<f64>,
    pub path: String,
}

#[derive(Clone, Debug, Default)]
pub struct RestartReport {
    pub name: String,
    pub kind: &'static str,
    pub before_instances: u32,
    /// 读不到内存时这里是已知部分之和，`unknown` 记录有几个实例没读到
    pub before_mb: f64,
    pub before_unknown: u32,
    pub after_instances: u32,
    pub after_mb: f64,
    pub after_unknown: u32,
    /// before − after，可能为负（如实写）
    pub reclaimed_mb: f64,
    pub closed: u32,
    pub forced: u32,
    pub relaunched: bool,
    /// 关掉之后自己回来了（外壳级自愈），因此没有重复拉起
    pub revived: bool,
    pub ok: bool,
    pub message: String,
    pub errors: Vec<String>,
}

impl RestartReport {
    pub fn reclaimed_text(&self) -> String {
        if self.reclaimed_mb >= 0.0 {
            format!("回收 {:.1} MB", self.reclaimed_mb)
        } else {
            format!("反而多占 {:.1} MB", -self.reclaimed_mb)
        }
    }
}

/// 按名字取当前所有实例（工作集读不到时 `ws_mb = None`，不当 0）
pub fn processes_named(name: &str) -> Vec<ProcInstance> {
    let mut out = Vec::new();
    for pe in win::snapshot_processes() {
        let pid = pe.th32ProcessID;
        if pid == 0 || pid == 4 {
            continue;
        }
        let raw = win::from_wide_buf(&pe.szExeFile);
        let n = raw
            .strip_suffix(".exe")
            .or_else(|| raw.strip_suffix(".EXE"))
            .unwrap_or(&raw);
        if !n.eq_ignore_ascii_case(name) {
            continue;
        }
        let (created, ws_mb, path) = match win::open_process(
            pid,
            win::PROCESS_QUERY_LIMITED_INFORMATION | win::PROCESS_VM_READ,
        ) {
            Some(h) => {
                let created = win::process_created(h.0).unwrap_or(0);
                let ws = win::process_memory(h.0)
                    .map(|m| (m.WorkingSetSize as f64 / 1_048_576.0 * 100.0).round() / 100.0);
                (created, ws, win::process_image_path(h.0))
            }
            None => (0, None, String::new()),
        };
        out.push(ProcInstance {
            pid,
            created,
            ws_mb,
            path,
        });
    }
    out.sort_by_key(|p| p.pid);
    out
}

fn total_ws(list: &[ProcInstance]) -> (f64, u32) {
    let mut mb = 0.0;
    let mut unknown = 0;
    for p in list {
        match p.ws_mb {
            Some(v) => mb += v,
            None => unknown += 1,
        }
    }
    ((mb * 100.0).round() / 100.0, unknown)
}

fn wait_until<F: FnMut() -> bool>(timeout_ms: u32, mut f: F) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        win::sleep_ms(100);
    }
}

/// 重启一个进程：关掉（先礼后兵）→ 必要时按快照里的 `Path` 拉起 → 等稳定 → 实测。
///
/// `exe_path` 建议用快照里记的路径（和用户当初启动的是同一个），
/// 空串或文件不存在时直接失败，不做任何猜测。
pub fn restart_process(
    name: &str,
    exe_path: &str,
    opts: CloseOptions,
    protected: &[u32],
    settle_ms: u32,
) -> RestartReport {
    let mut rep = RestartReport {
        name: name.to_string(),
        kind: "process",
        ..Default::default()
    };

    let before = processes_named(name);
    if before.is_empty() {
        rep.message = format!("没找到正在运行的 {}", name);
        return rep;
    }
    let (bmb, bunk) = total_ws(&before);
    rep.before_instances = before.len() as u32;
    rep.before_mb = bmb;
    rep.before_unknown = bunk;

    // 路径：优先调用方给的，其次从活着的实例里捡一个
    let exe = if !exe_path.trim().is_empty() {
        exe_path.trim().to_string()
    } else {
        before
            .iter()
            .map(|p| p.path.clone())
            .find(|p| !p.is_empty())
            .unwrap_or_default()
    };
    if exe.is_empty() || !Path::new(&exe).exists() {
        rep.message = format!(
            "拿不到能用的可执行路径（{}），不做重启：杀掉就起不来了",
            if exe.is_empty() { "空" } else { exe.as_str() }
        );
        return rep;
    }

    let targets: Vec<Target> = before
        .iter()
        .map(|p| Target {
            pid: p.pid,
            created: p.created,
        })
        .collect();
    let cr = close::close_group(name, &targets, opts, protected);
    rep.closed = cr.closed;
    rep.forced = cr.forced;
    rep.errors = cr.errors.clone();

    if cr.remaining > 0 {
        rep.message = format!("还有 {} 个实例关不掉，未拉起（避免出现两份）", cr.remaining);
        let after = processes_named(name);
        let (amb, aunk) = total_ws(&after);
        rep.after_instances = after.len() as u32;
        rep.after_mb = amb;
        rep.after_unknown = aunk;
        rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
        return rep;
    }

    // 自愈检查：外壳级进程（SearchHost / explorer / 托盘宿主）会在几百毫秒内回来
    let came_back = wait_until(800, || !processes_named(name).is_empty());
    if came_back {
        rep.revived = true;
        rep.message = format!(
            "{} 被关掉后自己又起来了（外壳级自愈），没有重复拉起",
            name
        );
    } else {
        let mut cmd = std::process::Command::new(&exe);
        if let Some(dir) = Path::new(&exe).parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(win::CREATE_NO_WINDOW);
        }
        match cmd.spawn() {
            Ok(_) => rep.relaunched = true,
            Err(e) => {
                rep.errors.push(format!("拉起失败: {}", e));
                rep.message = format!("已关闭，但拉起失败：{}", e);
                return rep;
            }
        }
    }

    // 等它把界面 / 工作集铺开，再实测
    wait_until(settle_ms.max(300), || !processes_named(name).is_empty());
    win::sleep_ms(settle_ms.max(300));

    let after = processes_named(name);
    let (amb, aunk) = total_ws(&after);
    rep.after_instances = after.len() as u32;
    rep.after_mb = amb;
    rep.after_unknown = aunk;
    rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
    rep.ok = rep.after_instances > 0;

    if !rep.ok {
        rep.message = "重启后没检测到实例，需要你手动确认".to_string();
    } else if rep.revived {
        // 自愈：新实例是外壳拉起的，回收量就是「关掉又回来」的净值
        rep.message = format!(
            "{}（自愈重生，{} 个实例 / {:.1} MB → {} 个 / {:.1} MB）",
            rep.reclaimed_text(),
            rep.before_instances,
            rep.before_mb,
            rep.after_instances,
            rep.after_mb
        );
    } else {
        rep.message = format!(
            "{}（{} 个实例 / {:.1} MB → {} 个 / {:.1} MB）",
            rep.reclaimed_text(),
            rep.before_instances,
            rep.before_mb,
            rep.after_instances,
            rep.after_mb
        );
    }
    rep
}

/// 只关不拉：给「关掉功能开关后本来就该退场」的进程用（比如 CrossDeviceResume）。
///
/// 回来后仍然如实报告（`revived = true`）——有些进程由外壳托管，杀了会重生，
/// 那种情况下「关掉」并不能换来内存，必须让调用方看见。
pub fn close_only(name: &str, opts: CloseOptions, protected: &[u32], settle_ms: u32) -> RestartReport {
    let mut rep = RestartReport {
        name: name.to_string(),
        kind: "process",
        ..Default::default()
    };
    let before = processes_named(name);
    if before.is_empty() {
        rep.message = format!("{} 现在没在跑", name);
        rep.ok = true;
        return rep;
    }
    let (bmb, bunk) = total_ws(&before);
    rep.before_instances = before.len() as u32;
    rep.before_mb = bmb;
    rep.before_unknown = bunk;

    let targets: Vec<Target> = before
        .iter()
        .map(|p| Target {
            pid: p.pid,
            created: p.created,
        })
        .collect();
    let cr = close::close_group(name, &targets, opts, protected);
    rep.closed = cr.closed;
    rep.forced = cr.forced;
    rep.errors = cr.errors.clone();

    // 有的宿主会在几百毫秒内被外壳拉回来，所以先等到「有结果」再安静等一会
    let _ = wait_until(settle_ms.max(300), || !processes_named(name).is_empty());
    win::sleep_ms(settle_ms.clamp(300, 1500));

    let after = processes_named(name);
    let (amb, aunk) = total_ws(&after);
    rep.after_instances = after.len() as u32;
    rep.after_mb = amb;
    rep.after_unknown = aunk;
    rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
    rep.revived = after_instances_revived(&before, &after, cr.remaining);
    rep.ok = cr.remaining == 0;
    rep.message = if rep.revived {
        format!(
            "关掉后自己又起来了（{} 个 / {:.1} MB → {} 个 / {:.1} MB），这次没省下内存",
            rep.before_instances, rep.before_mb, rep.after_instances, rep.after_mb
        )
    } else if rep.after_instances == 0 {
        format!(
            "已退出（{} 个 / {:.1} MB → 0），{}",
            rep.before_instances,
            rep.before_mb,
            rep.reclaimed_text()
        )
    } else {
        format!(
            "还有 {} 个没关掉（{}）",
            rep.after_instances,
            rep.reclaimed_text()
        )
    };
    rep
}

/// 关掉之后出现的实例，是「新的」（PID/创建时间跟关之前不一样）就算自愈重生
fn after_instances_revived(before: &[ProcInstance], after: &[ProcInstance], remaining: u32) -> bool {
    if after.is_empty() || remaining > 0 {
        return false;
    }
    after
        .iter()
        .all(|a| !before.iter().any(|b| b.pid == a.pid && b.created == a.created))
}

/// 重启一个服务：`stop` + `start`，量宿主进程前后工作集。
///
/// 注意 svchost 是共享宿主：宿主工作集的变化**不全是**这一个服务的，
/// 所以报告里会标注「共享宿主」，不把它当精确收益。
pub fn restart_service(name: &str, settle_ms: u32) -> RestartReport {
    let mut rep = RestartReport {
        name: name.to_string(),
        kind: "service",
        ..Default::default()
    };

    let before_pid = win::service_pid(name);
    rep.before_instances = if before_pid.is_some() { 1 } else { 0 };
    rep.before_mb = before_pid
        .and_then(win::process_ws_mb)
        .unwrap_or(0.0);

    if before_pid.is_none() {
        // 停止态：直接启动，收益无从对比
        match win::start_service(name) {
            Ok(()) => {
                rep.relaunched = true;
                let after_pid = win::service_pid(name);
                rep.after_instances = if after_pid.is_some() { 1 } else { 0 };
                rep.after_mb = after_pid.and_then(win::process_ws_mb).unwrap_or(0.0);
                rep.ok = rep.after_instances > 0;
                rep.message = format!("服务原本是停止态，已启动（{:.1} MB）", rep.after_mb);
            }
            Err(e) => {
                rep.errors.push(e.clone());
                rep.message = format!("启动失败：{}", e);
            }
        }
        return rep;
    }

    if let Err(e) = win::stop_service(name) {
        rep.errors.push(e.clone());
        rep.message = format!("停止失败：{}", e);
        return rep;
    }
    let stopped = wait_until(15_000, || win::service_pid(name).is_none());
    if !stopped {
        rep.message = "停止超时（服务没有在 15 秒内退出），未执行启动".to_string();
        rep.errors.push("stop timeout".into());
        return rep;
    }
    rep.closed = 1;

    if let Err(e) = win::start_service(name) {
        rep.errors.push(e.clone());
        rep.message = format!("已停止，但启动失败：{}", e);
        return rep;
    }
    rep.relaunched = true;
    wait_until(15_000, || win::service_pid(name).is_some());
    win::sleep_ms(settle_ms.max(300));

    let after_pid = win::service_pid(name);
    rep.after_instances = if after_pid.is_some() { 1 } else { 0 };
    rep.after_mb = after_pid.and_then(win::process_ws_mb).unwrap_or(0.0);
    rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
    rep.ok = rep.after_instances > 0;

    let shared = after_pid
        .and_then(|pid| {
            win::open_process(pid, win::PROCESS_QUERY_LIMITED_INFORMATION)
                .map(|h| win::process_image_path(h.0))
        })
        .map(|p| p.to_ascii_lowercase().ends_with("svchost.exe"))
        .unwrap_or(false);
    rep.message = format!(
        "{}（宿主 {:.1} MB → {:.1} MB{}）",
        rep.reclaimed_text(),
        rep.before_mb,
        rep.after_mb,
        if shared {
            "，共享 svchost 宿主，变化不全是这个服务"
        } else {
            ""
        }
    );
    rep
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 未知内存不参与合计() {
        let list = vec![
            ProcInstance {
                pid: 1,
                created: 0,
                ws_mb: Some(10.5),
                path: String::new(),
            },
            ProcInstance {
                pid: 2,
                created: 0,
                ws_mb: None,
                path: String::new(),
            },
        ];
        let (mb, unknown) = total_ws(&list);
        assert_eq!(mb, 10.5);
        assert_eq!(unknown, 1);
    }

    #[test]
    fn 回收量为负时如实写() {
        let r = RestartReport {
            reclaimed_mb: -12.3,
            ..Default::default()
        };
        assert!(r.reclaimed_text().contains("反而多占 12.3"));
    }
}
