//! 布局文件数据模型 —— 字段名与 C# / Python 版完全一致，三端共用同一份 JSON

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Screen {
    #[serde(default)]
    pub w: i32,
    #[serde(default)]
    pub h: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IconPos {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
}

/// 一块显示器的描述。`device` 用设备名（`\.\DISPLAY1`），
/// 不用 `DISPLAY1` 这种序号——Windows 每次开机可能重新编号。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct MonitorEntry {
    #[serde(default)]
    pub device: String,
    /// 虚拟屏坐标系里的 left/top/right/bottom
    #[serde(default)]
    pub rect: [i32; 4],
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub dpi: u32,
}

/// 保存布局时的显示器配置。坐标只在同一套配置下才有意义。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct DisplayInfo {
    /// 整个虚拟屏的 left/top/right/bottom（副屏在主屏左边时 left 是负数）
    #[serde(default)]
    pub virtual_screen: [i32; 4],
    #[serde(default)]
    pub monitors: Vec<MonitorEntry>,
}

impl DisplayInfo {
    pub fn width(&self) -> i32 {
        self.virtual_screen[2] - self.virtual_screen[0]
    }

    pub fn height(&self) -> i32 {
        self.virtual_screen[3] - self.virtual_screen[1]
    }

    /// 一句话描述，给界面和命令行看
    pub fn summary(&self) -> String {
        if self.monitors.is_empty() {
            return format!("{}×{}", self.width(), self.height());
        }
        let mut s = format!("{}×{} · {} 屏", self.width(), self.height(), self.monitors.len());
        for m in &self.monitors {
            s.push_str(&format!(
                " · {}×{}{}{}",
                m.rect[2] - m.rect[0],
                m.rect[3] - m.rect[1],
                if m.primary { "主" } else { "" },
                if m.dpi > 0 { format!("@{}dpi", m.dpi) } else { String::new() }
            ));
        }
        s
    }

    /// 硬件配置是否一致：屏数、每块屏的设备名、尺寸、主屏归属。
    /// 位置（left/top）也算——副屏换到另一边坐标就不成立了。
    pub fn same_hardware(&self, other: &Self) -> bool {
        if self.monitors.len() != other.monitors.len() {
            return false;
        }
        let mut mine: Vec<&MonitorEntry> = self.monitors.iter().collect();
        let mut theirs: Vec<&MonitorEntry> = other.monitors.iter().collect();
        let key = |m: &MonitorEntry| {
            (
                m.device.to_lowercase(),
                m.rect[2] - m.rect[0],
                m.rect[3] - m.rect[1],
                m.primary,
                m.dpi,
            )
        };
        mine.sort_by_key(|m| key(m));
        theirs.sort_by_key(|m| key(m));
        mine.iter().zip(theirs.iter()).all(|(a, b)| key(a) == key(b))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Layout {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub screen: Screen,
    #[serde(default)]
    pub auto_arrange: bool,
    #[serde(default)]
    pub icons: Vec<IconPos>,
    /// 保存时的显示器配置；老布局没有这个字段
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayInfo>,
}

impl Layout {
    pub fn screen_text(&self) -> String {
        if self.screen.w > 0 && self.screen.h > 0 {
            format!("{}×{}", self.screen.w, self.screen.h)
        } else {
            "分辨率未知".to_string()
        }
    }
}

/// 布局目录里的一个条目
#[derive(Debug, Clone)]
pub struct LayoutEntry {
    pub file_name: String,
    pub path: std::path::PathBuf,
    pub stamp: String,
    pub data: Layout,
}

impl LayoutEntry {
    pub fn display_name(&self) -> &str {
        if self.data.name.is_empty() {
            &self.file_name
        } else {
            &self.data.name
        }
    }

    pub fn count(&self) -> usize {
        self.data.icons.len()
    }
}

/// 一次恢复的结果
#[derive(Debug, Default, Clone)]
pub struct RestoreResult {
    /// 最终与布局一致（按名字能对上）的图标数
    pub restored: usize,
    /// 本次真正移动过的图标数
    pub moved: usize,
    /// 重试轮数
    pub rounds: usize,
    /// 是否用上了 shell 官方接口（摆得准，但扛不住 F5 —— 见 shellview.rs 头注释）
    pub used_shell_api: bool,
    /// 恢复前是否帮你关掉了自动排列
    pub auto_arrange_disabled: bool,
    /// 布局里有、桌面上找不到的
    pub missing: Vec<String>,
    /// 桌面上有、布局里没有的（保持原位）
    pub extra: Vec<String>,
    /// 多轮重试后仍然摆不到位的
    pub failed: Vec<String>,
    /// 是否成功关掉 shell 的「刷新时重新枚举」（关掉后按 F5 不会重排）
    pub refresh_guard: bool,
    /// 保存布局时的显示器配置与当前不一致（坐标可能不准）
    pub display_changed: bool,
    /// 两边的配置描述，给界面提示用
    pub saved_display: Option<String>,
    pub current_display: Option<String>,
}

/// 恢复前的差异预览
#[derive(Debug, Default, Clone)]
pub struct Plan {
    /// 需要移动的图标数
    pub to_move: usize,
    /// 布局里有、桌面上找不到的
    pub missing: Vec<String>,
    /// 桌面上有、布局里没有的
    pub extra: Vec<String>,
    /// 布局里的图标总数
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 能读懂_deskpos_写的布局文件() {
        // 这段 JSON 是 DeskPos（C# / Python / Rust 三端共用）实际落盘的格式，
        // 字段一个都不能改，否则旧布局会读不出来
        let text = r#"{
          "name": "工作",
          "created": "2026-10-21 08:59:00",
          "screen": { "w": 1920, "h": 1080 },
          "auto_arrange": false,
          "icons": [
            { "name": "此电脑", "x": 17, "y": 124 },
            { "name": "Microsoft Edge", "x": 112, "y": 124 }
          ]
        }"#;
        let l: Layout = serde_json::from_str(text).unwrap();
        assert_eq!(l.name, "工作");
        assert_eq!(l.screen.w, 1920);
        assert!(!l.auto_arrange);
        assert_eq!(l.icons.len(), 2);
        assert_eq!(l.icons[0].name, "此电脑");
        assert_eq!(l.icons[1].x, 112);
    }

    #[test]
    fn 缺字段也不炸() {
        let l: Layout = serde_json::from_str("{}").unwrap();
        assert!(l.icons.is_empty());
        assert_eq!(l.screen_text(), "分辨率未知");
    }

    #[test]
    fn 负坐标能存住() {
        // 副屏在主屏左侧时 x 是负数，不能被当成错误吞掉
        let text = r#"{"icons":[{"name":"副屏图标","x":-1180,"y":36}]}"#;
        let l: Layout = serde_json::from_str(text).unwrap();
        assert_eq!(l.icons[0].x, -1180);
        let back = serde_json::to_string(&l).unwrap();
        assert!(back.contains("-1180"));
    }

    #[test]
    fn 分辨率文本() {
        let mut l = Layout::default();
        l.screen.w = 2560;
        l.screen.h = 1440;
        assert_eq!(l.screen_text(), "2560×1440");
    }
}
