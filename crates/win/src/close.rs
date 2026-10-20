//! 关闭编排：先礼后兵，且只动「确实是刚才那个」进程。
//!
//! trim / GUI 执行按钮共用这一个入口，界面和命令行不许各写一套。
//!
//! 顺序：
//!   1. 身份复核 —— 拿 (PID, 创建时间) 重新打开进程，PID 换了人就跳过；
//!   2. 有可见窗口的，发 WM_CLOSE，等同用户点 ✕，给它时间保存 / 弹确认框；
//!   3. 没有窗口的（后台常驻、更新器）没有 UI 可交互，直接终止；
//!   4. 宽限期到点仍未退出的，才在身份校验下强杀——`force: false` 时干脆留着并如实上报。
//!
//! 多组一起关时只等**一个**宽限期：否则 20 组串行等待就是 20 × 3 秒。

use crate::win;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 一个待关闭的进程实例
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub pid: u32,
    /// 创建时间（FILETIME 100ns）；0 = 未知，退回只看 PID
    pub created: u64,
}

/// 一组同名的待关闭实例
#[derive(Clone, Copy, Debug)]
pub struct Group<'a> {
    pub name: &'a str,
    pub targets: &'a [Target],
}

#[derive(Clone, Copy, Debug)]
pub struct CloseOptions {
    /// 有窗口的进程先发 WM_CLOSE 再等；false = 直接终止（旧行为）
    pub graceful: bool,
    /// 发完 WM_CLOSE 后等多久再考虑强杀
    pub grace_ms: u32,
    /// 宽限期后是否允许强杀。false = 只礼不兵
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
    /// 实际发出 WM_CLOSE 的实例数
    pub requested: u32,
    /// 收到消息后自己退出的实例数
    pub closed: u32,
    /// 被强制终止的实例数（含本来就没有窗口的进程）
    pub forced: u32,
    /// 动手前就已经不在的实例数
    pub exited: u32,
    /// 身份对不上（PID 被复用 / 自身进程树）而跳过的实例数
    pub skipped: u32,
    /// 宽限期后仍在运行、且不允许或没能强杀的实例数
    pub remaining: u32,
    pub errors: Vec<String>,
}

impl CloseReport {
    /// 有动作生效，且没有残留
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

/// 一次关多组，共用同一个宽限期。返回的报告与 `groups` 一一对应。
pub fn close_groups(
    groups: &[Group],
    opts: CloseOptions,
    protected: &[u32],
) -> Vec<CloseReport> {
    let mut reports: Vec<CloseReport> = groups.iter().map(|_| CloseReport::default()).collect();
    let mut alive: Vec<Vec<Target>> = groups.iter().map(|_| Vec::new()).collect();

    // 1) 身份复核：先确认「还是那个进程」，再决定要不要动手
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

    // 2) 有窗口的：一次枚举全部发 WM_CLOSE（等价用户点 ✕）
    let all_pids: Vec<u32> = alive
        .iter()
        .flat_map(|v| v.iter().map(|t| t.pid))
        .collect();
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
                // 3) 没有窗口可关：没有 UI 就没有「保存 / 确认」可等，直接终止（同样校验身份）
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

    // 4) 给窗口应用留出保存 / 弹框的时间（所有组共用一个 deadline）
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

    // 5) 宽限期到点还在的：按策略强杀或如实上报
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

/// 关闭一组。单组调用 `close_groups` 的薄封装。
pub fn close_group(
    name: &str,
    targets: &[Target],
    opts: CloseOptions,
    protected: &[u32],
) -> CloseReport {
    let mut out = close_groups(
        &[Group {
            name,
            targets,
        }],
        opts,
        protected,
    );
    out.pop().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 礼貌关闭计入已处理() {
        let r = CloseReport {
            requested: 2,
            closed: 1,
            forced: 1,
            ..Default::default()
        };
        assert!(r.done());
        let line = r.one_line("chrome");
        assert!(line.contains("礼貌关闭 1"), "{}", line);
        assert!(line.contains("强制 1"), "{}", line);
    }

    #[test]
    fn 还有残留就不算完成() {
        let r = CloseReport {
            requested: 1,
            remaining: 1,
            ..Default::default()
        };
        assert!(!r.done());
        assert!(r.one_line("code").contains("仍未退出"));
    }

    #[test]
    fn 空目标不会误报成功() {
        let rep = close_group("nothing", &[], CloseOptions::default(), &[]);
        assert!(!rep.done());
        assert!(rep.one_line("nothing").contains("没有可处理实例"));
    }

    #[test]
    fn 自身进程树一律跳过() {
        let me = std::process::id();
        let rep = close_group(
            "membase",
            &[Target {
                pid: me,
                created: 0,
            }],
            CloseOptions::default(),
            &[me],
        );
        assert_eq!(rep.skipped, 1);
        assert_eq!(rep.forced, 0);
    }

    /// 没有可见顶层窗口的进程（测试进程自己就是）不该被发消息：
    /// 这条是确定性的，CI 里也能跑。
    #[test]
    fn 没有窗口的进程不参与礼貌关闭() {
        let me = std::process::id();
        assert!(
            win::request_close_for(&[me]).is_empty(),
            "测试宿主自己不该有可见顶层窗口"
        );
    }

    /// 真窗口上验一次「能找到窗口、消息确实入队」。
    /// 只断言「找到了并发出去了」，不断言进程一定退出：
    /// Win11 不少系统程序（含 charmap）会先关窗口、再拖一会儿才退，
    /// 那属于程序自己的收尾行为，不该让测试变成看运气的。
    ///   cargo test -p membase-win -- --ignored
    #[test]
    #[ignore = "需要真实桌面会话：会短暂弹出一个字符映射表窗口"]
    fn 能向窗口应用发出关闭请求() {
        let exe = r"C:\Windows\System32\charmap.exe";
        if !std::path::Path::new(exe).exists() {
            return; // 精简版系统上没这程序，跳过
        }
        let mut child = std::process::Command::new(exe)
            .spawn()
            .expect("启动 charmap 失败");
        let pid = child.id();
        let created = win::process_create_time(pid).unwrap_or(0);

        let mut hit = std::collections::HashMap::new();
        for _ in 0..50 {
            hit = win::request_close_for(&[pid]);
            if !hit.is_empty() {
                break;
            }
            win::sleep_ms(100);
        }
        assert!(!hit.is_empty(), "没找到 charmap 的可见窗口");

        // 收尾：这个进程可能还赖着，别把它留在桌面上
        let rep = close_group(
            "charmap",
            &[Target { pid, created }],
            CloseOptions {
                grace_ms: 1000,
                force: true,
                graceful: true,
            },
            &[],
        );
        eprintln!("{}", rep.one_line("charmap"));
        if win::process_alive_as(pid, created) {
            let _ = win::terminate_process(pid);
        }
        let _ = child.wait();
    }
}
