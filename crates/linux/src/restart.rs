//! 局部重启（Linux）：关掉再按原路径拉起，并**实测**回收了多少。

use crate::close::{self, CloseOptions, Target};
use crate::procfs;
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
    pub before_mb: f64,
    pub before_unknown: u32,
    pub after_instances: u32,
    pub after_mb: f64,
    pub after_unknown: u32,
    pub reclaimed_mb: f64,
    pub closed: u32,
    pub forced: u32,
    pub relaunched: bool,
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

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn processes_named(name: &str) -> Vec<ProcInstance> {
    let mut out = Vec::new();
    for p in procfs::list() {
        if p.pid <= 2 || !p.name.eq_ignore_ascii_case(name) {
            continue;
        }
        out.push(ProcInstance {
            pid: p.pid,
            created: p.start_ticks,
            ws_mb: if p.rss_kb > 0 {
                Some((p.rss_kb as f64 / 1024.0 * 100.0).round() / 100.0)
            } else {
                None
            },
            path: p.exe_path(),
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

fn has_new_instance(name: &str, before: &[ProcInstance]) -> bool {
    processes_named(name).iter().any(|n| {
        !before
            .iter()
            .any(|b| b.pid == n.pid && b.created == n.created)
    })
}

fn live_survivors(name: &str, before: &[ProcInstance]) -> u32 {
    processes_named(name)
        .iter()
        .filter(|n| {
            before
                .iter()
                .any(|b| b.pid == n.pid && b.created == n.created)
        })
        .count() as u32
}

fn spawn_image(_name: &str, exe: &str) -> Result<(), String> {
    let dir = Path::new(exe).parent().map(|p| p.to_path_buf());
    let mut cmd = std::process::Command::new(exe);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

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

    if cr.closed + cr.forced == 0 {
        let after = processes_named(name);
        let (amb, aunk) = total_ws(&after);
        rep.after_instances = after.len() as u32;
        rep.after_mb = amb;
        rep.after_unknown = aunk;
        rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
        rep.ok = rep.after_instances > 0;
        rep.message = if cr.skipped > 0 {
            format!(
                "没有关掉任何实例：{} 个被跳过（{}），这一项其实没动",
                cr.skipped,
                cr.errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "保护集合".into())
            )
        } else if !after.is_empty() {
            format!(
                "没有关掉任何实例，{} 还在跑（权限不足？），这次没动它",
                name
            )
        } else {
            "动手之前它就已经退出了，不需要重启".to_string()
        };
        return rep;
    }

    let came_back = wait_until(800, || has_new_instance(name, &before));
    if came_back {
        rep.revived = true;
        rep.message = format!("{} 被关掉后自己又起来了（会话级自愈），没有重复拉起", name);
    } else if live_survivors(name, &before) > 0 {
        rep.message = format!("{} 关掉后旧实例还在跑，没有再拉一份（重启未生效）", name);
        let after = processes_named(name);
        let (amb, aunk) = total_ws(&after);
        rep.after_instances = after.len() as u32;
        rep.after_mb = amb;
        rep.after_unknown = aunk;
        rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
        rep.ok = rep.after_instances > 0;
        return rep;
    } else {
        match spawn_image(name, &exe) {
            Ok(()) => rep.relaunched = true,
            Err(e) => {
                rep.errors.push(format!("拉起失败: {}", e));
                rep.message = format!("已关闭，但拉起失败：{}", e);
                return rep;
            }
        }
    }

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

/// 只关不拉（开关关闭后本来就该退场的进程）
pub fn close_only(
    name: &str,
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

    let _ = wait_until(settle_ms.max(300), || !processes_named(name).is_empty());
    win::sleep_ms(settle_ms.clamp(300, 1500));

    let after = processes_named(name);
    let (amb, aunk) = total_ws(&after);
    rep.after_instances = after.len() as u32;
    rep.after_mb = amb;
    rep.after_unknown = aunk;
    rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
    rep.revived = !after.is_empty()
        && cr.remaining == 0
        && after.iter().all(|a| {
            !before
                .iter()
                .any(|b| b.pid == a.pid && b.created == a.created)
        });
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

/// 重启一个服务：`systemctl restart`，量宿主进程前后工作集。
pub fn restart_service(name: &str, settle_ms: u32) -> RestartReport {
    let mut rep = RestartReport {
        name: name.to_string(),
        kind: "service",
        ..Default::default()
    };

    let before_pid = win::service_pid(name);
    rep.before_instances = if before_pid.is_some() { 1 } else { 0 };
    rep.before_mb = before_pid.and_then(win::process_ws_mb).unwrap_or(0.0);

    if before_pid.is_none() {
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

    let up = wait_until(15_000, || win::service_pid(name).is_some());
    if !up {
        rep.message = "启动后 15 秒内没检测到宿主进程".to_string();
        return rep;
    }
    win::sleep_ms(settle_ms.max(300));
    let after_pid = win::service_pid(name);
    rep.after_instances = if after_pid.is_some() { 1 } else { 0 };
    rep.after_mb = after_pid.and_then(win::process_ws_mb).unwrap_or(0.0);
    rep.reclaimed_mb = round2(rep.before_mb - rep.after_mb);
    rep.ok = rep.after_instances > 0;
    rep.message = format!(
        "{}（宿主 {:.1} MB → {:.1} MB；若该 unit 与别的服务共享进程，变化不全是它）",
        rep.reclaimed_text(),
        rep.before_mb,
        rep.after_mb
    );
    rep
}
