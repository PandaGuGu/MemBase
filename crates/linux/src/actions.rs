//! 高层动作：保存 / 差异预览 / 恢复 —— CLI 和 GUI 共用（Linux 版）

use crate::desktop::Desktop;
use crate::shellview::ShellView;
use crate::sys;
use anyhow::Result;
use membase_core::layout::{IconPos, Layout, Plan, RestoreResult, Screen};
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

/// 恢复布局：Xfce 直接改 rc 文件并让 xfdesktop 重载
pub fn restore(layout: &Layout) -> Result<RestoreResult> {
    restore_wait(layout, std::time::Duration::from_millis(5600))
}

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

    // 显示器配置比对：坐标只在同一套屏下才成立
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

    // 登录后 xfdesktop 可能还没把图标写进 rc 文件：等它出现，最多等 wait。
    let mut shell = ShellView::try_open();
    let step = std::time::Duration::from_millis(700);
    let tries = (wait.as_millis() / step.as_millis().max(1)) as usize;
    let mut last_n = usize::MAX;
    let mut stable = 0usize;
    for _ in 0..tries {
        let n = d.snapshot().map(|s| s.len()).unwrap_or(0);
        // Linux 没有 shell 接口可等：图标数够了、或者连续几轮不再变化就动手，
        // 否则每次恢复都会白白等满 --wait（开机自启场景能等到 180 秒）。
        if n >= want.len() {
            break;
        }
        if n > 0 && n == last_n {
            stable += 1;
        } else {
            stable = 0;
        }
        last_n = n;
        if stable >= 3 {
            break;
        }
        std::thread::sleep(step);
        if shell.is_none() {
            shell = ShellView::try_open();
        }
    }

    if let Some(mut sv) = shell {
        result.refresh_guard =
            sv.set_folder_flags(crate::sys::FWF_NOENUMREFRESH, crate::sys::FWF_NOENUMREFRESH);
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
    }

    // 文件方式 + 多轮校验
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
    // 位置都写进 rc 了；xfdesktop 只在启动时读取网格，必须重启一次才看得见变化
    if moved > 0 {
        crate::desktop::reload_desktop_now();
    }
    Ok(result)
}
