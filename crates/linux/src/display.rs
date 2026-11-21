//! 显示器指纹（Linux）：`xrandr --listmonitors` 解析成和 Windows 版同构的结构。

use membase_core::layout::{DisplayInfo, MonitorEntry};
use std::process::Command;

fn dpi() -> u32 {
    if let Ok(out) = Command::new("xrdb").arg("-query").output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Xft.dpi:") {
                if let Ok(n) = v.trim().parse::<f32>() {
                    return n.round().max(0.0) as u32;
                }
            }
        }
    }
    96
}

/// 解析 `1920/344x1080/193+0+0`
fn parse_geom(s: &str) -> Option<(i32, i32, i32, i32)> {
    let xpos = s.find('x')?;
    let w: i32 = s[..xpos].split('/').next()?.parse().ok()?;
    let rest = &s[xpos + 1..];
    let h_end = rest
        .find(|c: char| c == '/' || c == '+' || c == '-')
        .unwrap_or(rest.len());
    let h: i32 = rest[..h_end].parse().ok()?;
    let mut tail = &rest[h_end..];
    if let Some(stripped) = tail.strip_prefix('/') {
        // 跳过物理毫米，找到第一个坐标符号
        let idx = stripped.find(['+', '-'])?;
        tail = &stripped[idx..];
    }
    // +X+Y
    let mut nums: Vec<i32> = Vec::new();
    let mut cur = String::new();
    for c in tail.chars() {
        if c == '+' || c == '-' {
            if !cur.is_empty() {
                nums.push(cur.parse().ok()?);
                cur.clear();
            }
            cur.push(c);
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        nums.push(cur.parse().ok()?);
    }
    if nums.len() < 2 {
        return None;
    }
    Some((nums[0], nums[1], w, h))
}

/// 当前显示器配置。拿不到详细信息时退化成「只知道总分辨率」。
pub fn fingerprint() -> DisplayInfo {
    let dpi = dpi();
    let mut monitors: Vec<MonitorEntry> = Vec::new();
    if let Ok(out) = Command::new("xrandr").arg("--listmonitors").output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines().skip(1) {
                let mut fields = line.split_whitespace();
                let _idx = fields.next();
                let head = fields.next().unwrap_or("");
                let geom = fields.next().unwrap_or("");
                let name = fields.next().unwrap_or(head).to_string();
                let primary = head.contains('*');
                if let Some((x, y, w, h)) = parse_geom(geom) {
                    monitors.push(MonitorEntry {
                        device: name.trim_start_matches(['+', '*']).to_string(),
                        rect: [x, y, x + w, y + h],
                        primary,
                        dpi,
                    });
                }
            }
        }
    }
    if monitors.is_empty() {
        let (w, h) = crate::sys::screen_size();
        monitors.push(MonitorEntry {
            device: "screen0".to_string(),
            rect: [0, 0, w, h],
            primary: true,
            dpi,
        });
    }
    monitors.sort_by(|a, b| a.device.to_lowercase().cmp(&b.device.to_lowercase()));

    let left = monitors.iter().map(|m| m.rect[0]).min().unwrap_or(0);
    let top = monitors.iter().map(|m| m.rect[1]).min().unwrap_or(0);
    let right = monitors.iter().map(|m| m.rect[2]).max().unwrap_or(0);
    let bottom = monitors.iter().map(|m| m.rect[3]).max().unwrap_or(0);

    DisplayInfo {
        virtual_screen: [left, top, right, bottom],
        monitors,
    }
}
