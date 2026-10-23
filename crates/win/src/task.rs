//! 开机采集计划任务：注册 / 查询 / 删除。
//!
//! CLI 的 `install` / `uninstall` 与 GUI 的按钮共用这里的实现，
//! 任务 XML 只生成一次，避免两个前端各自拼一份、慢慢漂移。

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::win::CREATE_NO_WINDOW;

/// 默认任务名；采集完成后任务会自毁，所以常年为空是正常状态。
/// 直接复用 core 里的常量，避免两个 crate 各存一份字符串。
pub use membase_core::DEFAULT_TASK;

/// 只读的任务快照，给 GUI 显示用
#[derive(Clone, Debug, Default)]
pub struct TaskInfo {
    pub task: String,
    /// 任务是否已注册
    pub installed: bool,
    /// 从任务参数里解析出的开机偏移秒数；解析不到为 0
    pub at_sec: u64,
    /// 双采样时第二次采集的开机偏移秒数（没有就是单点采集）
    pub also_sec: Option<u64>,
    /// 任务实际执行的 exe
    pub exe: String,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn task_xml(exe: &Path, out: &Path, at_sec: u64, also_sec: Option<u64>, task: &str) -> String {
    let exe_s = xml_escape(&exe.to_string_lossy());
    let also_arg = match also_sec {
        Some(s) => format!(" --also {}", s),
        None => String::new(),
    };
    let args_s = xml_escape(&format!(
        "collect --out \"{}\" --at {}{} --self-destruct --task {} --json",
        out.display(),
        at_sec,
        also_arg,
        task
    ));
    let description = match also_sec {
        Some(s) => format!(
            "开机第 {at} 秒与第 {s} 秒各采集一次内存/进程/服务基准线（双采样），采集完成后自毁（membase）",
            at = at_sec,
            s = s
        ),
        None => format!(
            "开机第 {at} 秒采集一次内存/进程/服务基准线，采集完成后自毁（membase）",
            at = at_sec
        ),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{desc}</Description>
  </RegistrationInfo>
  <Triggers>
    <BootTrigger><Enabled>true</Enabled></BootTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-18</UserId>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT20M</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command><Arguments>{args}</Arguments></Exec>
  </Actions>
</Task>
"#,
        desc = xml_escape(&description),
        exe = exe_s,
        args = args_s
    )
}

/// 注册开机采集任务，并把 `src_exe` 复制成 `<out>\membase.exe`。
///
/// 界面版和命令行版是同一个程序（带参数走命令行、无参数开界面），
/// 所以这里不需要再区分 PE 子系统：任务参数 `collect ...` 不会开窗口。
pub fn install(
    src_exe: &Path,
    out: &Path,
    at_sec: u64,
    also_sec: Option<u64>,
    task: &str,
) -> Result<PathBuf, String> {
    let task = if task.trim().is_empty() {
        DEFAULT_TASK
    } else {
        task.trim()
    };
    if !src_exe.is_file() {
        return Err(format!("找不到可执行文件：{}", src_exe.display()));
    }
    fs::create_dir_all(out).map_err(|e| format!("建目录失败: {e}"))?;
    let target = out.join("membase.exe");

    // 自己拷自己会失败（win 上占用），先比路径
    let same = match (src_exe.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::copy(src_exe, &target).map_err(|e| format!("复制 exe 失败: {e}"))?;
    }

    let xml = task_xml(&target, out, at_sec, also_sec, task);
    let tmp = std::env::temp_dir().join(format!("membase_task_{}.xml", std::process::id()));
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    fs::write(&tmp, &bytes).map_err(|e| format!("写 XML 失败: {e}"))?;

    // 静音：schtasks 会往 stdout 打「成功: 已创建计划任务…」，
    // 混进 --json 的流里就没法解析了，结果一律用返回值表达
    let st = Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            task,
            "/XML",
            &tmp.to_string_lossy(),
            "/F",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_file(&tmp);

    match st {
        Ok(s) if s.success() => Ok(target),
        Ok(_) => Err("注册失败：需要管理员权限".into()),
        Err(e) => Err(format!("调用 schtasks 失败: {e}")),
    }
}

/// 删除任务；任务本来就不存在也算成功
pub fn uninstall(task: &str) -> Result<(), String> {
    let task = if task.trim().is_empty() {
        DEFAULT_TASK
    } else {
        task.trim()
    };
    let st = Command::new("schtasks")
        .args(["/Delete", "/TN", task, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match st {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err("任务不存在或权限不足".into()),
        Err(e) => Err(format!("调用 schtasks 失败: {e}")),
    }
}

fn decode(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u);
    }
    let s = String::from_utf8_lossy(bytes).to_string();
    // 没 BOM 但满是 NUL，说明是 UTF-16LE
    let nuls = s.chars().filter(|c| *c == '\0').count();
    if nuls > 0 && nuls * 4 > s.chars().count() {
        let u: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u);
    }
    s
}

fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let i = xml.find(&open)?;
    let rest = &xml[i + open.len()..];
    let j = rest.find(&close)?;
    Some(xml_unescape(rest[..j].trim()))
}

fn extract_at(args: &str) -> Option<u64> {
    extract_flag(args, "--at")
}

fn extract_also(args: &str) -> Option<u64> {
    extract_flag(args, "--also")
}

fn extract_flag(args: &str, flag: &str) -> Option<u64> {
    let i = args.find(flag)?;
    args[i + flag.len()..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

/// 查任务当前状态；查不到就是「未安装」，不算错误
pub fn query(task: &str) -> TaskInfo {
    let task = if task.trim().is_empty() {
        DEFAULT_TASK
    } else {
        task.trim()
    };
    let mut info = TaskInfo {
        task: task.to_string(),
        ..Default::default()
    };
    let out = match Command::new("schtasks")
        .args(["/Query", "/TN", task, "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return info,
    };
    let text = decode(&out.stdout);
    if !text.contains("<Task") {
        return info;
    }
    info.installed = true;
    info.exe = tag_text(&text, "Command").unwrap_or_default();
    if let Some(a) = tag_text(&text, "Arguments") {
        info.at_sec = extract_at(&a).unwrap_or(0);
        info.also_sec = extract_also(&a);
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 从任务参数里抠出开机秒数() {
        let a = r#"collect --out "C:\ProgramData\MemBaseline" --at 60 --self-destruct --task X --json"#;
        assert_eq!(extract_at(a), Some(60));
        assert_eq!(extract_at("collect --at 45"), Some(45));
        assert_eq!(extract_at("collect --json"), None);
        assert_eq!(extract_also(a), None);
        let d = r#"collect --out "C:\x" --at 30 --also 300 --json"#;
        assert_eq!(extract_at(d), Some(30));
        assert_eq!(extract_also(d), Some(300));
    }

    #[test]
    fn xml_转义可以往返() {
        let raw = r#"a&b<c>"d""#;
        assert_eq!(xml_unescape(&xml_escape(raw)), raw);
    }
}

// ---------------------------------------------------------------- 开机自动恢复桌面布局

/// 开机自动恢复布局的任务名（和采集任务分开，互不影响）
pub const LAYOUT_TASK: &str = "MemBaseLayout";

fn layout_task_xml(exe: &Path, layout: &str, delay_sec: u64) -> String {
    let exe_s = xml_escape(&exe.to_string_lossy());
    let args_s = xml_escape(&format!(
        "layout restore \"{}\" --execute --yes --wait 180 --settle 45 --retries 3",
        layout
    ));
    let desc = format!(
        "登录后第 {delay} 秒把桌面图标摆回布局「{layout}」（membase）",
        delay = delay_sec,
        layout = layout
    );
    // 不写 <Principals>：schtasks 会按当前用户、交互式令牌注册，
    // 布局恢复必须跑在用户会话里才看得到桌面，用 SYSTEM 是无效的。
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{desc}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <Delay>PT{delay}S</Delay>
    </LogonTrigger>
  </Triggers>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT10M</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command><Arguments>{args}</Arguments></Exec>
  </Actions>
</Task>
"#,
        desc = xml_escape(&desc),
        exe = exe_s,
        args = args_s,
        delay = delay_sec
    )
}

/// 注册「登录后自动恢复布局」任务；exe 会复制到数据目录，路径不会因为挪动原文件而失效。
pub fn install_layout(
    src_exe: &Path,
    out: &Path,
    layout: &str,
    delay_sec: u64,
    task: &str,
) -> Result<PathBuf, String> {
    let task = if task.trim().is_empty() { LAYOUT_TASK } else { task.trim() };
    if !src_exe.is_file() {
        return Err(format!("找不到可执行文件：{}", src_exe.display()));
    }
    if layout.trim().is_empty() {
        return Err("布局名字不能为空".into());
    }
    fs::create_dir_all(out).map_err(|e| format!("建目录失败: {e}"))?;
    let target = out.join("membase.exe");
    let same = match (src_exe.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::copy(src_exe, &target).map_err(|e| format!("复制 exe 失败: {e}"))?;
    }

    let xml = layout_task_xml(&target, layout, delay_sec);
    let tmp = std::env::temp_dir().join(format!("membase_layout_task_{}.xml", std::process::id()));
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    fs::write(&tmp, &bytes).map_err(|e| format!("写 XML 失败: {e}"))?;

    let st = Command::new("schtasks")
        .args(["/Create", "/TN", task, "/XML", &tmp.to_string_lossy(), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_file(&tmp);

    match st {
        Ok(s) if s.success() => Ok(target),
        Ok(_) => Err("注册失败：需要管理员权限".into()),
        Err(e) => Err(format!("调用 schtasks 失败: {e}")),
    }
}

/// 查「登录自动恢复」任务里记的布局名；没装任务返回 None
pub fn query_layout(task: &str) -> Option<String> {
    let task = if task.trim().is_empty() { LAYOUT_TASK } else { task.trim() };
    let out = Command::new("schtasks")
        .args(["/Query", "/TN", task, "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let xml = decode(&out.stdout);
    let args = tag_text(&xml, "Arguments")?;
    let lowered = args.to_lowercase();
    let pos = lowered.find("layout restore")?;
    let rest = &args[pos + "layout restore".len()..];
    let rest = rest.trim_start();
    let name = if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next()?.to_string()
    } else {
        rest.split_whitespace().next()?.to_string()
    };
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
