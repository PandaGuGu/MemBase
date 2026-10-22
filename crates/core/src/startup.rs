//! 启动项：模型、命令行解析、计划任务 XML 解析、分级建议。
//!
//! 为什么单独一层：启动项和「快照里出现的进程」是两件事。
//! 进程要等它跑起来才看得见，而启动项**还没跑**就已经写在系统里了——
//! MuMu 三件套、`crashpad_handler ×3` 这类东西，等它们出现在快照里就晚了。
//!
//! 这一层只做纯字符串处理和判定，注册表 / 文件夹 / 任务文件由平台层喂进来。

use crate::{tier_for, Kind, Tier};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum StartupSource {
    /// HKCU\...\Run
    UserRun,
    /// HKLM\...\Run（64 位视图）
    MachineRun,
    /// HKLM\...\Wow6432Node\...\Run
    MachineRun32,
    /// 用户「启动」文件夹
    UserFolder,
    /// 公共「启动」文件夹
    AllUsersFolder,
    /// 计划任务（跳过 Microsoft 自带）
    ScheduledTask,
}

impl StartupSource {
    pub fn key(&self) -> &'static str {
        match self {
            StartupSource::UserRun => "user-run",
            StartupSource::MachineRun => "machine-run",
            StartupSource::MachineRun32 => "machine-run32",
            StartupSource::UserFolder => "user-startup-folder",
            StartupSource::AllUsersFolder => "all-users-startup-folder",
            StartupSource::ScheduledTask => "scheduled-task",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            StartupSource::UserRun => "注册表·当前用户",
            StartupSource::MachineRun => "注册表·本机",
            StartupSource::MachineRun32 => "注册表·本机32位",
            StartupSource::UserFolder => "启动文件夹·当前用户",
            StartupSource::AllUsersFolder => "启动文件夹·公共",
            StartupSource::ScheduledTask => "计划任务",
        }
    }
    /// 禁用手段：注册表/文件夹走 StartupApproved，任务走 schtasks
    pub fn can_disable(&self) -> bool {
        true
    }
}

/// 平台层枚举出来的原始项
#[derive(Clone, Debug)]
pub struct StartupRaw {
    pub source: StartupSource,
    /// Run 值名 / 快捷方式文件名 / 任务路径
    pub name: String,
    /// 命令行（快捷方式给目标路径）
    pub command: String,
    /// 是否启用；`None` = 没有批准记录也读不到任务状态，按启用处理
    pub enabled: Option<bool>,
    /// 位置细节（任务路径、快捷方式完整路径、Run 键名）
    pub detail: String,
    /// 位于 `\Microsoft\` 下的系统计划任务。
    ///
    /// 这类任务里绝大多数是系统功能（不该动），但也有 VS 安装器 `BackgroundDownload`
    /// 这种一开机就吃掉 339 MB 的第三方后台下载 —— 2026-10-22 实测，它就是因为
    /// 「Microsoft 目录一律跳过」而四个处置入口全漏。所以照收，只是单独标出来，
    /// 默认不在清单里展开。
    pub system: bool,
}

#[derive(Clone, Debug)]
pub struct StartupItem {
    pub source: StartupSource,
    pub name: String,
    pub command: String,
    /// 从命令行解析出来的进程名（不含 .exe）
    pub exe: String,
    pub enabled: bool,
    pub tier: Tier,
    /// 这个名字在基准快照里出现过（说明它确实常驻，不是空挂）
    pub in_baseline: bool,
    /// 系统任务（`\Microsoft\` 下的计划任务）
    pub system: bool,
    pub suggestion: &'static str,
    pub note: String,
    pub detail: String,
}

/// 从命令行里抠出真正的可执行文件名（不含 `.exe`，保留原大小写）。
///
/// 处理这几种真实写法：
///   * `"C:\Program Files\MuMu\MuMuPlayer.exe" --mini`
///   * `C:\Program Files\App\app.exe -autorun`（没加引号）
///   * `rundll32.exe C:\Windows\system32\shell32.dll,Control_RunDLL` → `shell32`
///   * `C:\Windows\system32\cmd.exe /c "start x.exe"` → `cmd`
pub fn command_exe(command: &str) -> String {
    let cmd = command.trim();
    if cmd.is_empty() {
        return String::new();
    }
    // 1) 带引号：取第一个引号内的内容
    let first = if let Some(rest) = cmd.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else if cmd.contains(".exe") || cmd.contains(".dll") {
        // 2) 没引号但带扩展名：截到扩展名结尾（路径里可能有空格）
        match cut_at_extension(cmd) {
            Some(p) => p,
            None => cmd.split_whitespace().next().unwrap_or("").to_string(),
        }
    } else {
        cmd.split_whitespace().next().unwrap_or("").to_string()
    };
    // 4) 「by user disabled」这类占位串不是程序：任务被应用自己改写成禁用标记时会出现
    //    （本机 Google Updater 就是这么写的），别把 "by" 当成进程名。
    let lc = cmd.to_ascii_lowercase();
    let head: String = lc.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    if matches!(head.as_str(), "by user disabled" | "disabled by user") || lc == "disabled" {
        return String::new();
    }
    let base = file_stem(&first);
    // 3) 代理宿主：真正被拉起来的是参数里那个模块
    if base.eq_ignore_ascii_case("rundll32") {
        let after = cmd
            .split(char::is_whitespace)
            .nth(1)
            .unwrap_or("")
            .trim_matches('"');
        // 模块后面常跟 `,入口函数`（shell32.dll,Control_RunDLL），先切掉再取文件名
        let module = after.split(',').next().unwrap_or(after);
        let m = file_stem(module);
        if !m.is_empty() {
            return m;
        }
    }
    base
}

/// 找到第一个 `.exe` / `.dll` 的结尾位置并截断
fn cut_at_extension(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    for ext in [".exe", ".dll", ".com", ".bat", ".cmd"] {
        if let Some(i) = lower.find(ext) {
            return Some(s[..i + ext.len()].to_string());
        }
    }
    None
}

/// 路径最后一段，去掉扩展名
pub fn file_stem(path: &str) -> String {
    let p = path.trim().trim_matches('"');
    let last = p.rsplit(['\\', '/']).next().unwrap_or(p);
    let lower = last.to_ascii_lowercase();
    for ext in [".exe", ".dll", ".lnk", ".url", ".bat", ".cmd", ".com"] {
        if lower.ends_with(ext) && last.len() > ext.len() {
            return last[..last.len() - ext.len()].to_string();
        }
    }
    last.to_string()
}

/// 计划任务 XML 里我们关心的部分
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TaskAction {
    pub command: String,
    pub arguments: String,
    /// `<Settings><Enabled>`；缺省按启用
    pub enabled: bool,
    /// 触发方式：Boot / Logon / Daily / ...（取第一个）
    pub trigger: String,
    /// `<Hidden>true</Hidden>`
    pub hidden: bool,
}

/// 解析计划任务 XML。只做标签提取，不引 XML 库——任务文件格式很稳定，
/// 而且这里只需要 4 个字段（命令、参数、是否启用、触发方式）。
pub fn parse_task_xml(xml: &str) -> Option<TaskAction> {
    if !xml.contains("<Task") {
        return None;
    }
    let command = tag_text(xml, "Command")?;
    let mut out = TaskAction {
        command: command.trim().to_string(),
        arguments: tag_text(xml, "Arguments").unwrap_or_default().trim().to_string(),
        enabled: true,
        trigger: String::new(),
        hidden: false,
    };
    // Settings 里的 Enabled 才是任务总开关；Triggers 里的是单个触发器
    if let Some(settings) = section(xml, "Settings") {
        if let Some(v) = tag_text(settings, "Enabled") {
            out.enabled = !v.trim().eq_ignore_ascii_case("false");
        }
        out.hidden = tag_text(settings, "Hidden")
            .map(|v| v.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    }
    for t in [
        "BootTrigger",
        "LogonTrigger",
        "RegistrationTrigger",
        "DailyTrigger",
        "WeeklyTrigger",
        "TimeTrigger",
        "CalendarTrigger",
    ] {
        if xml.contains(&format!("<{}", t)) {
            out.trigger = t.trim_end_matches("Trigger").to_string();
            break;
        }
    }
    Some(out)
}

fn tag_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// 取某个标签包裹的整段（用于只看 Settings 里的 Enabled）
fn section<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open);
    let (s, e) = match start {
        Some(i) => (i + open.len(), xml[i..].find(&close)? + i),
        None => {
            // 带属性的形式：<Settings ...>
            let i = xml.find(&format!("<{} ", tag))?;
            let body_start = i + xml[i..].find('>')? + 1;
            let end = xml[body_start..].find(&close)? + body_start;
            (body_start, end)
        }
    };
    Some(&xml[s..e])
}

/// 把原始项折成给人看的清单：解析 exe、分级、判断是否在基准里。
pub fn build_startup_items(raw: &[StartupRaw], baseline: &BTreeSet<String>) -> Vec<StartupItem> {
    let mut out: Vec<StartupItem> = Vec::new();
    for r in raw {
        let exe = command_exe(&r.command);
        let tier = tier_for(Kind::Process, &exe, &r.command);
        let enabled = r.enabled.unwrap_or(true);
        let in_baseline = !exe.is_empty()
            && baseline
                .iter()
                .any(|b| b.eq_ignore_ascii_case(&exe));
        let suggestion = match (enabled, tier) {
            (false, _) => "already-off",
            (true, Tier::Blocked) => "keep",
            (true, Tier::Caution) => "review",
            (true, Tier::Suggested) => "disable",
        };        let mut note = String::new();
        if in_baseline {
            note.push_str("基准里出现过；");
        }
        if r.source == StartupSource::ScheduledTask {
            if let Some(t) = r.detail.rsplit('\\').next() {
                if !t.is_empty() && t != r.name {
                    note.push_str(&format!("任务 {}；", t));
                }
            }
        }
        if exe.is_empty() {
            note.push_str("命令行解析不出可执行文件；");
        }
        if r.system {
            note.push_str("系统任务；");
        }
        out.push(StartupItem {
            source: r.source,
            name: r.name.clone(),
            command: r.command.clone(),
            exe,
            enabled,
            tier,
            in_baseline,
            system: r.system,
            suggestion,
            note: note.trim_end_matches('；').to_string(),
            detail: r.detail.clone(),
        });
    }
    // 建议处理的排前面，其次按来源、名字
    out.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

fn rank(it: &StartupItem) -> u8 {
    match (it.enabled, it.suggestion) {
        (false, _) => 3,
        (true, "disable") => 0,
        (true, "review") => 1,
        _ => 2,
    }
}

impl StartupItem {
    /// 建议动作的中文说法（界面 / 终端共用）
    pub fn suggestion_cn(&self) -> &'static str {
        match self.suggestion {
            "disable" => "建议禁用",
            "review" => "需人工确认",
            "already-off" => "已禁用",
            "keep" => "保持启用",
            _ => "—",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 引号路径() {
        assert_eq!(
            command_exe(r#""C:\Program Files\MuMu\MuMuPlayer.exe" --mini"#),
            "MuMuPlayer"
        );
    }

    #[test]
    fn 不加引号带空格的路径() {
        assert_eq!(
            command_exe(r"C:\Program Files\App\app.exe -autorun"),
            "app"
        );
    }

    #[test]
    fn rundll32_取被调起的模块() {
        assert_eq!(
            command_exe(r"C:\Windows\system32\rundll32.exe C:\Windows\system32\shell32.dll,Control_RunDLL"),
            "shell32"
        );
    }

    #[test]
    fn 环境变量前缀与裸命令() {
        assert_eq!(command_exe(r"%ProgramFiles%\Foo\Bar.exe /s"), "Bar");
        assert_eq!(command_exe("code"), "code");
    }

    #[test]
    fn 空的命令行不炸() {
        assert_eq!(command_exe(""), "");
        assert_eq!(command_exe("   "), "");
    }

    /// 任务被应用自己改写成「by user disabled」时，别把 by 当进程名
    #[test]
    fn 占位串不是程序() {
        assert_eq!(command_exe("by user disabled"), "");
        assert_eq!(command_exe("by user disabled --wake --system"), "");
        assert_eq!(command_exe("Disabled"), "");
        // 真正的裸命令保留
        assert_eq!(command_exe("code"), "code");
    }

    #[test]
    fn 计划任务_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><URI>\MuMu\MuMuAutoStart</URI></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><Enabled>true</Enabled><Hidden>false</Hidden></Settings>
  <Actions Context="Author"><Exec><Command>"C:\Program Files\MuMu\MuMuPlayer.exe"</Command><Arguments>--auto</Arguments></Exec></Actions>
</Task>"#;
        let t = parse_task_xml(xml).expect("应该能解析");
        assert_eq!(t.command, r#""C:\Program Files\MuMu\MuMuPlayer.exe""#);
        assert_eq!(t.arguments, "--auto");
        assert!(t.enabled);
        assert!(!t.hidden);
        assert_eq!(t.trigger, "Logon");
    }

    #[test]
    fn 禁用态任务与没有设置段() {
        let xml = r#"<Task><Settings><Enabled>false</Enabled></Settings><Actions><Exec><Command>C:\a\b.exe</Command></Exec></Actions></Task>"#;
        let t = parse_task_xml(xml).unwrap();
        assert!(!t.enabled);
        assert_eq!(t.trigger, "");
        assert!(parse_task_xml("<html>no</html>").is_none());
    }

    #[test]
    fn 建议与基准命中() {
        let raw = vec![
            StartupRaw {
                source: StartupSource::UserRun,
                name: "MuMu".into(),
                command: r#""C:\Program Files\MuMu\MuMuPlayer.exe" -mini"#.into(),
                enabled: Some(true),
                detail: "HKCU\\...\\Run".into(),
                system: false,
            },
            StartupRaw {
                source: StartupSource::ScheduledTask,
                name: "\\Update\\Updater".into(),
                command: r"C:\Program Files\Up\up.exe /bg".into(),
                enabled: Some(false),
                detail: "\\Update\\Updater".into(),
                system: false,
            },
            StartupRaw {
                source: StartupSource::MachineRun,
                name: "Sec".into(),
                command: r"C:\Windows\System32\lsass.exe".into(),
                enabled: Some(true),
                detail: "HKLM\\...\\Run".into(),
                system: false,
            },
        ];
        let mut base = BTreeSet::new();
        base.insert("MuMuPlayer".to_string());
        let items = build_startup_items(&raw, &base);
        // 建议处理的排最前
        assert_eq!(items[0].name, "MuMu");
        assert_eq!(items[0].suggestion, "disable");
        assert!(items[0].in_baseline, "MuMuPlayer 在基准里出现过");
        assert_eq!(items.last().unwrap().name, "\\Update\\Updater");
        assert_eq!(items.last().unwrap().suggestion, "already-off");
        // lsass 是关键组件，不该建议关
        let sec = items.iter().find(|i| i.name == "Sec").unwrap();
        assert_eq!(sec.tier, Tier::Blocked);
        assert_eq!(sec.suggestion, "keep");
    }
}
