//! 关闭编排（Linux）：先 SIGTERM（等同于点 ✕ / 礼貌退出），宽限期后再 SIGKILL。
//!
//! 与 Windows 版同一套顺序和统计口径：身份复核 → 礼貌关闭 → 共用一个宽限期 → 强杀/如实上报。

use crate::win;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub pid: u32,
    /// 开机以来的时钟滴答；0 = 未知，退回只看 PID
    pub created: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Group<'a> {
    pub name: &'a str,
    pub targets: &'a [Target],
}

#[derive(Clone, Copy, Debug)]
pub struct CloseOptions {
    pub graceful: bool,
    pub grace_ms: u32,
    pub force: bool,
}

impl Default for CloseOptions {
    fn default() -> Self {
        CloseOptions {
            graceful: true,
            grace_ms: 3000,
            force: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CloseReport {
    pub requested: u32,
    pub closed: u32,
    pub forced: u32,
    pub exited: u32,
    pub skipped: u32,
    pub remaining: u32,
    pub errors: Vec<String>,
}

impl CloseReport {
    pub fn done(&self) -> bool {
        self.remaining == 0 && self.closed + self.forced > 0
    }

    pub fn one_line(&self, name: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.closed > 0 {
            parts.push(format!("礼貌关闭 {}", self.closed));
        }
        if self.forced > 0 {
            parts.push(format!("强制 {}", self.forced));
        }
        if self.exited > 0 {
            parts.push(format!("已退出 {}", self.exited));
        }
        if self.skipped > 0 {
            parts.push(format!("跳过 {}", self.skipped));
        }
        if self.remaining > 0 {
            parts.push(format!("仍未退出 {}", self.remaining));
        }
        if parts.is_empty() {
            parts.push("没有可处理实例".into());
        }
        let mut s = format!("{}: {}", name, parts.join("，"));
        if let Some(e) = self.errors.first() {
            s.push_str(&format!("（{}）", e));
        }
        s
    }
}

pub fn close_groups(groups: &[Group], opts: CloseOptions, protected: &[u32]) -> Vec<CloseReport> {
    let mut reports: Vec<CloseReport> = groups.iter().map(|_| CloseReport::default()).collect();
    let mut alive: Vec<Vec<Target>> = groups.iter().map(|_| Vec::new()).collect();

    for (i, g) in groups.iter().enumerate() {
        for t in g.targets {
            if protected.contains(&t.pid) {
                reports[i].skipped += 1;
                reports[i]
                    .errors
                    .push(format!("pid {} 属于本工具自身进程树", t.pid));
                continue;
            }
            match win::process_create_time(t.pid) {
                None => reports[i].exited += 1,
                Some(now) if t.created != 0 && now != t.created => {
                    reports[i].skipped += 1;
                    reports[i]
                        .errors
                        .push(format!("pid {} 已被系统复用，跳过", t.pid));
                }
                Some(_) => alive[i].push(*t),
            }
        }
    }

    let all_pids: Vec<u32> = alive.iter().flat_map(|v| v.iter().map(|t| t.pid)).collect();
    let hit: HashMap<u32, usize> = if opts.graceful && !all_pids.is_empty() {
        win::request_close_for(&all_pids)
    } else {
        HashMap::new()
    };

    let mut waiting: Vec<Vec<Target>> = groups.iter().map(|_| Vec::new()).collect();
    for (i, list) in alive.iter().enumerate() {
        for t in list {
            if hit.get(&t.pid).copied().unwrap_or(0) > 0 {
                reports[i].requested += 1;
                waiting[i].push(*t);
            } else {
                match win::terminate_process_checked(t.pid, t.created) {
                    Ok(()) => reports[i].forced += 1,
                    Err(e) => {
                        if win::process_alive_as(t.pid, t.created) {
                            reports[i].remaining += 1;
                        } else {
                            reports[i].exited += 1;
                        }
                        reports[i].errors.push(format!("pid {}: {}", t.pid, e));
                    }
                }
            }
        }
    }

    let any_waiting = waiting.iter().any(|v| !v.is_empty());
    if any_waiting && opts.grace_ms > 0 {
        let deadline = Instant::now() + Duration::from_millis(opts.grace_ms as u64);
        while Instant::now() < deadline {
            let still = waiting
                .iter()
                .flatten()
                .any(|t| win::process_alive_as(t.pid, t.created));
            if !still {
                break;
            }
            win::sleep_ms(100);
        }
    }

    for (i, list) in waiting.iter().enumerate() {
        for t in list {
            if !win::process_alive_as(t.pid, t.created) {
                reports[i].closed += 1;
                continue;
            }
            if !opts.force {
                reports[i].remaining += 1;
                continue;
            }
            match win::terminate_process_checked(t.pid, t.created) {
                Ok(()) => reports[i].forced += 1,
                Err(e) => {
                    if win::process_alive_as(t.pid, t.created) {
                        reports[i].remaining += 1;
                    } else {
                        reports[i].closed += 1;
                    }
                    reports[i].errors.push(format!("pid {}: {}", t.pid, e));
                }
            }
        }
    }

    reports
}

pub fn close_group(
    name: &str,
    targets: &[Target],
    opts: CloseOptions,
    protected: &[u32],
) -> CloseReport {
    let mut out = close_groups(&[Group { name, targets }], opts, protected);
    out.pop().unwrap_or_default()
}
