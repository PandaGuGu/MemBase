//! 高层动作：保存 / 差异预览 / 恢复 —— CLI 和 GUI 共用

use crate::desktop::Desktop;
use membase_core::layout::{IconPos, Layout, Plan, RestoreResult, Screen};
use crate::shellview::ShellView;
use crate::sys;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// 抓一份当前桌面布局
pub fn snapshot(name: &str) -> Result<Layout> {
    let d = Desktop::open()?;
    let icons = d.snapshot()?;
    let (w, h) = sys::screen_size();
    Ok(Layout {
        name: name.to_string(),
        created: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        screen: Screen { w, h },
        auto_arrange: d.auto_arrange(),
        icons,
        display: Some(crate::display::fingerprint()),
    })
}

/// 当前桌面的所有图标位置
pub fn current_positions() -> Result<Vec<IconPos>> {
    Desktop::open()?.snapshot()
}

/// 只看差异，不改动桌面
pub fn plan(layout: &Layout) -> Result<Plan> {
    let d = Desktop::open()?;
    let cur = d.snapshot()?;

    let saved: HashMap<String, &IconPos> = layout
        .icons
        .iter()
        .filter(|i| !i.name.is_empty())
        .map(|i| (i.name.to_lowercase(), i))
        .collect();
    let now: HashMap<String, &IconPos> = cur.iter().map(|i| (i.name.to_lowercase(), i)).collect();

    let mut out = Plan {
        total: saved.len(),
        ..Default::default()
    };
    for (k, want) in &saved {
        match now.get(k) {
            Some(cur) => {
                if cur.x != want.x || cur.y != want.y {
                    out.to_move += 1;
                }
            }
            None => out.missing.push(want.name.clone()),
        }
    }
    let saved_keys: HashSet<&String> = saved.keys().collect();
    for (k, it) in &now {
        if !saved_keys.contains(k) {
            out.extra.push(it.name.clone());
        }
    }
    Ok(out)
}

/// 当前和布局不一致的图标「现在在哪」（给预览画虚线用）
pub fn ghost(layout: &Layout) -> Result<Vec<IconPos>> {
    let cur = current_positions()?;
    let saved: HashMap<String, &IconPos> = layout
        .icons
        .iter()
        .filter(|i| !i.name.is_empty())
        .map(|i| (i.name.to_lowercase(), i))
        .collect();

    let mut out = Vec::new();
    for it in cur {
        if let Some(want) = saved.get(&it.name.to_lowercase()) {
            if want.x != it.x || want.y != it.y {
                out.push(it);
            }
        }
    }
    Ok(out)
}

/// 恢复布局：优先 shell 官方接口（摆得准），不可用时退回内存写入。
/// 两条路都只保证「当前会话」——刷新/重启资源管理器会被 shell 的布局模型覆盖，
/// 详见 shellview.rs 头注释。
pub fn restore(layout: &Layout) -> Result<RestoreResult> {
    restore_wait(layout, std::time::Duration::from_millis(5600))
}

/// `wait` 是等 shell 接口就绪的总时长上限。登录任务用得上：
/// explorer 刚起来时 `IShellWindows` 还没注册桌面视图，太急就会退回内存写入。
pub fn restore_wait(layout: &Layout, wait: std::time::Duration) -> Result<RestoreResult> {
    let d = Desktop::open()?;

    let auto_off = if d.auto_arrange() {
        d.disable_auto_arrange()
    } else {
        false
    };
    let mut result = RestoreResult {
        auto_arrange_disabled: auto_off,
        ..Default::default()
    };

    // 显示器配置比对：坐标只在同一套屏下才成立，配置变了要说清楚，别让图标跑到看不见的地方
    let current_display = crate::display::fingerprint();
    if let Some(saved) = &layout.display {
        if !saved.same_hardware(&current_display) {
            result.display_changed = true;
            result.saved_display = Some(saved.summary());
            result.current_display = Some(current_display.summary());
        }
    }

    // 名字 -> 目标坐标（大小写不敏感）
    let mut want: HashMap<String, (i32, i32)> = HashMap::new();
    for it in &layout.icons {
        if it.name.is_empty() {
            continue;
        }
        want.insert(it.name.to_lowercase(), (it.x, it.y));
    }
    if want.is_empty() {
        return Ok(result);
    }

    // 以当前桌面的图标名为基准算 missing / extra
    let cur = d.snapshot()?;
    let now: HashSet<String> = cur.iter().map(|i| i.name.to_lowercase()).collect();
    for it in &layout.icons {
        if it.name.is_empty() {
            continue;
        }
        if !now.contains(&it.name.to_lowercase()) {
            result.missing.push(it.name.clone());
        }
    }
    let saved_keys: HashSet<String> = want.keys().cloned().collect();
    for it in &cur {
        if !saved_keys.contains(&it.name.to_lowercase()) {
            result.extra.push(it.name.clone());
        }
    }

    // ---------- 首选：shell 官方接口 ----------
    // explorer 刚重启时 COM 还没注册好，first try 经常拿不到，隔一会儿再试几次 ——
    // 退化成内存写入的话，刷新就白干了。
    // 实测：强杀 explorer 后要 4～20 秒 COM 才重新注册好，太短的等待会退回内存写入，
    // 而内存写入在按 F5 时一定被系统重排 —— 等于白干。所以这里肯等。
    // 两个条件都要满足才动手：
    //   1) shell 官方接口能用；
    //   2) 桌面图标已经加载出来（登录后第 30 秒往往还没加载完）。
    // 只满足 1) 时枚举结果是空的，位置写不进去，却会被当成「本来就不需要动」而静默成功 ——
    // 2026-10-23 重启实测就栽在这里：任务 9:27:25 跑了、退出码 0、桌面一个没动。
    let mut shell = ShellView::try_open();
    let step = std::time::Duration::from_millis(700);
    let tries = (wait.as_millis() / step.as_millis().max(1)) as usize;
    let mut last_n = usize::MAX;
    let mut stable = 0usize;
    for _ in 0..tries {
        let n = d.snapshot().map(|s| s.len()).unwrap_or(0);
        if shell.is_some() && n >= want.len() {
            break;
        }
        // 图标数量连续几次不再变化，说明加载完了（可能确实少几个被删掉的）
        if n > 0 && n == last_n {
            stable += 1;
        } else {
            stable = 0;
        }
        last_n = n;
        if shell.is_some() && stable >= 5 {
            break;
        }
        std::thread::sleep(step);
        if shell.is_none() {
            shell = ShellView::try_open();
        }
    }
    if let Some(mut sv) = shell {
        // 关掉「刷新时重新枚举」：这是「按 F5 图标被打回原位」的唯一开关。
        // 实测同一份布局：置位后按 F5 = 0 差异，清位后按 F5 = 66 项错位。
        result.refresh_guard = sv.set_folder_flags(
            crate::sys::FWF_NOENUMREFRESH,
            crate::sys::FWF_NOENUMREFRESH,
        );
        // 同步写注册表：运行时的标志在 explorer 被强杀时会丢，写进 FFlags 才能跨重启保留
        if result.refresh_guard {
            if let Some(f) = crate::repair::read_fflags() {
                let _ = crate::repair::write_fflags(f | crate::sys::FWF_NOENUMREFRESH);
            }
        }
        if let Ok((moved, failed, rounds)) = sv.apply(&want, 4) {
            result.used_shell_api = true;
            result.moved = moved;
            result.rounds = rounds;
            result.failed = failed;
            result.restored = want
                .len()
                .saturating_sub(result.missing.len() + result.failed.len());
            return Ok(result);
        }
        // 失败就走下面的内存方式
    }

    // ---------- 退路：内存写入 + 多轮校验 ----------
    let mut moved = 0usize;
    let mut rounds = 0usize;
    for round in 0..4 {
        let idx = d.index_map()?;
        let mut wrote = 0usize;
        for it in &layout.icons {
            if it.name.is_empty() {
                continue;
            }
            let Some(&i) = idx.get(&it.name.to_lowercase()) else {
                continue;
            };
            let (cx, cy) = d.item_pos(i)?;
            if cx == it.x && cy == it.y {
                continue;
            }
            d.set_pos(i, it.x, it.y)?;
            wrote += 1;
            if round == 0 {
                moved += 1;
            }
        }
        if wrote == 0 {
            break;
        }
        rounds += 1;
        std::thread::sleep(std::time::Duration::from_millis(140));
    }

    // 核对
    let idx = d.index_map()?;
    let mut ok = 0usize;
    let mut failed = Vec::new();
    for it in &layout.icons {
        if it.name.is_empty() {
            continue;
        }
        let Some(&i) = idx.get(&it.name.to_lowercase()) else {
            continue;
        };
        let (cx, cy) = d.item_pos(i)?;
        if cx == it.x && cy == it.y {
            ok += 1;
        } else {
            failed.push(it.name.clone());
        }
    }

    result.restored = ok;
    result.moved = moved;
    result.failed = failed;
    result.rounds = rounds.max(1);
    Ok(result)
}
