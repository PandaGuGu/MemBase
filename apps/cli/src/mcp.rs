//! MCP（Model Context Protocol）stdio 服务端：让 AI 直接调用 MemBase。
//!
//! 跑法：`MemBase.exe mcp`（走 stdio，JSON-RPC 2.0，一行一条消息）。
//!
//! ## 为什么自己手写而不是引 SDK
//!
//! 官方 Rust SDK 会带进 tokio + async-trait + 一堆依赖，直接把 CLI 的编译量
//! 从「15 个第三方小库」抬到「几十个」。这里的做法是：
//!   * 只用 `serde_json`（core 已经在编，不新增依赖）；
//!   * 手写 JSON-RPC 的行分隔收发；
//!   * 工具实现**直接调用已有的 CLI 动词**，不重写业务逻辑。
//!
//! ## 安全模型（与命令行完全一致）
//!
//! 工具的参数不是「原始 argv」，而是受限的白名单参数；每个会动手的工具
//! 都带 `execute` / `yes` 两个布尔位，且必须**同时**为真才真的动手 ——
//! 与命令行 `--execute --yes` 的门禁一一对应。默认一律 dry-run。
//!
//! 换句话说：AI 能调用 MemBase，但**不能绕过**命令行已有的那些护栏
//! （关键服务拒绝、动作需二次确认、默认只列不动手）。

use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, Write};

/// 客户端在 `initialize` 里报自己的版本，我们回一个能兼容的：
/// 老版本（2024-11-05 / 2025-03-26 / 2025-06-18 / 2025-11-25）走握手流程，
/// 新版本（2026-07-28）去掉了 initialize，但老客户端仍占多数，所以握手必须留着；
/// 这里同时接受新版的 `server/discover`。
const PROTOCOL_VERSIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
    "2026-07-28",
];

/// 协商好的协议版本。新版协议里版本号随每个请求走 `_meta`，
/// 老版只在 initialize 时给一次 —— 所以要一直记着。
/// 用调用方持有的可变状态传递，避免 `static mut`（工作区禁止 unsafe）。
struct Session {
    negotiated: Option<String>,
}

impl Session {
    fn new() -> Self {
        Session { negotiated: None }
    }

    /// 当前生效的版本（未协商时回最新支持的）。
    fn version(&self) -> &'static str {
        match &self.negotiated {
            Some(v) if PROTOCOL_VERSIONS.contains(&v.as_str()) => {
                PROTOCOL_VERSIONS.iter().find(|x| **x == *v).unwrap()
            }
            _ => PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1],
        }
    }

    fn set(&mut self, v: &str) {
        if PROTOCOL_VERSIONS.contains(&v) {
            self.negotiated = Some(v.to_string());
        }
    }
}

/// 工具描述表：一个工具 = 一个 CLI 动词 + 一组受限参数。
struct Tool {
    name: &'static str,
    desc: &'static str,
    /// 是否会修改系统状态。true 时 schema 里带 execute / yes 两个门禁位。
    mutating: bool,
    /// 该工具在 execute/yes 之外的额外参数（进 JSON Schema 的 properties）。
    ///
    /// 单独存一份而不是把整个 schema 塞进来：静态表里不能调用非 const 函数
    /// （`json!` 不是 const），所以 schema 必须在运行时拼。
    extra: fn() -> Value,
    /// 构造 argv：第一个元素是子命令名。参数非法时返回错误（会作为工具错误回给 AI）。
    build: fn(&Map<String, Value>) -> Result<Vec<String>, String>,
}

// ---------------------------------------------------------------- 参数辅助

fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        other => other.to_string(),
    }
}

fn b(v: &Value) -> bool {
    match v {
        Value::Bool(x) => *x,
        Value::String(x) => x == "true" || x == "1",
        _ => false,
    }
}

fn opt_str(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key).and_then(|v| match v {
        Value::Null => None,
        Value::String(x) if x.is_empty() => None,
        other => Some(s(other)),
    })
}

fn opt_bool(args: &Map<String, Value>, key: &str) -> bool {
    args.get(key).map(b).unwrap_or(false)
}

fn opt_num(args: &Map<String, Value>, key: &str) -> Option<f64> {
    args.get(key).and_then(|v| v.as_f64())
}

fn opt_list(args: &Map<String, Value>, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::String(x) if x.is_empty() => None,
                other => Some(s(other)),
            })
            .collect(),
        Some(Value::String(x)) => x
            .split([',', ';'])
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// 把列表拼成 `--only a,b` 这种形式（CLI 的 list_arg 按逗号/分号切）。
fn push_list(out: &mut Vec<String>, args: &Map<String, Value>, key: &str, flag: &str) {
    let items = opt_list(args, key);
    if !items.is_empty() {
        out.push(flag.to_string());
        out.push(items.join(","));
    }
}

/// 动手类工具统一的门禁：execute 与 yes 必须同时为真。
///
/// 与 CLI 里 `if execute && !yes { 拒绝 }` 的判断完全一致 ——
/// 这里只是把它提前到参数层，好让 AI 在 schema 里就看见这两位。
fn mutating_gate(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let execute = opt_bool(args, "execute");
    let yes = opt_bool(args, "yes");
    if execute && !yes {
        return Err(
            "execute 与 yes 必须同时为 true 才会真的动手；两者都为 false 时只输出预演结果。"
                .to_string(),
        );
    }
    let mut out = Vec::new();
    if execute {
        out.push("--execute".to_string());
        out.push("--yes".to_string());
    }
    Ok(out)
}

// ---------------------------------------------------------------- 工具定义

fn t_status(_: &Map<String, Value>) -> Result<Vec<String>, String> {
    Ok(vec!["status".into(), "--json".into()])
}
fn extra_none() -> Value {
    json!({})
}

fn t_list(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["list".to_string(), "--json".to_string()];
    if opt_bool(args, "all") {
        out.push("--all".into());
    }
    if opt_bool(args, "include_caution") {
        out.push("--include-caution".into());
    }
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    push_list(&mut out, args, "only", "--only");
    push_list(&mut out, args, "exclude", "--exclude");
    Ok(out)
}
fn extra_list() -> Value {
    json!({
        "all": {"type": "boolean", "description": "连「系统关键」项也列出来"},
        "include_caution": {"type": "boolean", "description": "放开「谨慎」档（Windows 目录下的组件）"},
        "min_mb": {"type": "number", "description": "只看占用超过 N MB 的项"},
        "only": {"type": "array", "items": {"type": "string"}, "description": "只处理这些名字"},
        "exclude": {"type": "array", "items": {"type": "string"}, "description": "排除这些名字"}
    })
}

fn t_plan(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["plan".to_string(), "--json".to_string()];
    if opt_bool(args, "include_caution") {
        out.push("--include-caution".into());
    }
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    push_list(&mut out, args, "only", "--only");
    push_list(&mut out, args, "exclude", "--exclude");
    Ok(out)
}
fn extra_plan() -> Value {
    json!({
        "include_caution": {"type": "boolean", "description": "放开「谨慎」档"},
        "min_mb": {"type": "number", "description": "只看占用超过 N MB 的项"},
        "only": {"type": "array", "items": {"type": "string"}, "description": "只看这些名字"},
        "exclude": {"type": "array", "items": {"type": "string"}, "description": "排除这些名字"}
    })
}

fn t_trim(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["trim".to_string(), "--json".to_string()];
    out.extend(mutating_gate(args)?);
    if opt_bool(args, "include_caution") {
        out.push("--include-caution".into());
    }
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    push_list(&mut out, args, "only", "--only");
    push_list(&mut out, args, "exclude", "--exclude");
    Ok(out)
}
fn extra_restart() -> Value {
    json!({
        "target": {"type": "string", "description": "进程名 / PID / 服务名"},
        "wait": {"type": "number", "description": "拉起后等几秒再量（默认 3）"}
    })
}

fn extra_kill() -> Value {
    json!({
        "target": {"type": "string", "description": "进程名或 PID"}
    })
}

fn extra_svc() -> Value {
    json!({
        "action": {"type": "string", "enum": ["stop", "start", "disable", "enable"], "description": "要做的动作"},
        "service": {"type": "string", "description": "服务名"}
    })
}

fn extra_feature() -> Value {
    json!({
        "action": {"type": "string", "enum": ["list", "off", "on"], "description": "要做的动作"},
        "key": {"type": "string", "description": "功能开关名（off/on 时必填）"},
        "no_restart": {"type": "boolean", "description": "只改注册表，不重启宿主让它当场生效"}
    })
}

fn extra_startup() -> Value {
    json!({
        "action": {"type": "string", "enum": ["list", "disable", "enable"], "description": "要做的动作"},
        "name": {"type": "string", "description": "启动项名称（disable/enable 时必填）"},
        "all": {"type": "boolean", "description": "list 时连系统计划任务一起列"}
    })
}

fn extra_protect() -> Value {
    json!({
        "action": {"type": "string", "enum": ["list", "add", "remove", "clear"], "description": "要做的动作"},
        "names": {"type": "array", "items": {"type": "string"}, "description": "名字列表（add/remove 时必填）"}
    })
}

fn extra_collect() -> Value {
    json!({
        "at": {"type": "number", "description": "开机第几秒采（默认 60）"},
        "out": {"type": "string", "description": "数据目录"}
    })
}

fn extra_compare() -> Value {
    json!({
        "min_mb": {"type": "number", "description": "只看变化超过 N MB 的项"},
        "exclude": {"type": "array", "items": {"type": "string"}, "description": "排除这些名字"}
    })
}

fn extra_transient() -> Value {
    json!({
        "min_mb": {"type": "number", "description": "只看占用超过 N MB 的项"}
    })
}

fn extra_baseline() -> Value {
    json!({
        "action": {"type": "string", "enum": ["info", "verify", "capture", "export", "import"], "description": "要做的动作"},
        "to": {"type": "string", "description": "export 的输出路径"},
        "package": {"type": "string", "description": "import 的 .mbz 文件路径"},
        "label": {"type": "string", "description": "capture 的采样点标签"}
    })
}

/// 会动手的工具统一在 schema 里加 execute / yes 两位。
fn schema_mutating(extra: Value) -> Value {
    let mut props = Map::new();
    props.insert("execute".into(), json!({
        "type": "boolean",
        "description": "是否真的动手。false（默认）只输出预演结果，不改系统"
    }));
    props.insert("yes".into(), json!({
        "type": "boolean",
        "description": "二次确认。必须与 execute 同时为 true，否则拒绝执行"
    }));
    if let Value::Object(p) = extra {
        for (k, v) in p {
            props.insert(k, v);
        }
    }
    json!({"type": "object", "properties": props, "additionalProperties": false})
}

/// 只读工具的 schema：只有 extra 里那些参数。
fn schema_readonly(extra: Value) -> Value {
    let mut props = Map::new();
    if let Value::Object(p) = extra {
        for (k, v) in p {
            props.insert(k, v);
        }
    }
    json!({"type": "object", "properties": props, "additionalProperties": false})
}

fn t_slim(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["slim".to_string(), "--json".to_string()];
    out.extend(mutating_gate(args)?);
    if opt_bool(args, "include_caution") {
        out.push("--include-caution".into());
    }
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    push_list(&mut out, args, "only", "--only");
    push_list(&mut out, args, "exclude", "--exclude");
    Ok(out)
}

fn t_svc(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let action = opt_str(args, "action").ok_or("缺少 action（stop / start / disable / enable）")?;
    if !matches!(action.as_str(), "stop" | "start" | "disable" | "enable") {
        return Err(format!("action 只支持 stop / start / disable / enable，收到 {action}"));
    }
    let name = opt_str(args, "service").ok_or("缺少 service（服务名）")?;
    let mut out = vec!["svc".to_string(), action, name, "--json".to_string()];
    // svc 的门禁只看 --execute（CLI 里 svc 不要求 --yes）
    if opt_bool(args, "execute") {
        out.push("--execute".into());
    }
    Ok(out)
}

fn t_kill(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let target = opt_str(args, "target").ok_or("缺少 target（进程名或 PID）")?;
    let mut out = vec!["kill".to_string(), target, "--json".to_string()];
    if opt_bool(args, "execute") {
        out.push("--execute".into());
    }
    Ok(out)
}

fn t_restart(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let target = opt_str(args, "target").ok_or("缺少 target（进程名 / PID / 服务名）")?;
    let mut out = vec!["restart".to_string(), target, "--json".to_string()];
    out.extend(mutating_gate(args)?);
    if let Some(n) = opt_num(args, "wait") {
        out.push("--wait".into());
        out.push(format!("{}", n));
    }
    Ok(out)
}

fn t_feature(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let action = opt_str(args, "action").unwrap_or_else(|| "list".to_string());
    let mut out = vec!["feature".to_string()];
    match action.as_str() {
        "list" | "ls" => {
            out.push("list".into());
            out.push("--json".into());
        }
        "off" | "on" => {
            let key = opt_str(args, "key").ok_or("缺少 key（功能开关名）")?;
            out.push(action);
            out.push(key);
            out.push("--json".into());
            out.extend(mutating_gate(args)?);
            if opt_bool(args, "no_restart") {
                out.push("--no-restart".into());
            }
        }
        other => {
            return Err(format!("action 只支持 list / off / on，收到 {other}"));
        }
    }
    Ok(out)
}

fn t_startup(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let action = opt_str(args, "action").unwrap_or_else(|| "list".to_string());
    let mut out = vec!["startup".to_string()];
    match action.as_str() {
        "list" | "ls" => {
            out.push("list".into());
            out.push("--json".into());
            if opt_bool(args, "all") {
                out.push("--all".into());
            }
        }
        "disable" | "enable" => {
            let name = opt_str(args, "name").ok_or("缺少 name（启动项名称）")?;
            out.push(action);
            out.push(name);
            out.push("--json".into());
            out.extend(mutating_gate(args)?);
        }
        other => {
            return Err(format!("action 只支持 list / disable / enable，收到 {other}"));
        }
    }
    Ok(out)
}

fn t_protect(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let action = opt_str(args, "action").unwrap_or_else(|| "list".to_string());
    let mut out = vec!["protect".to_string()];
    match action.as_str() {
        "list" | "ls" => {
            out.push("list".into());
            out.push("--json".into());
        }
        "add" | "remove" | "rm" | "clear" => {
            out.push(action.clone());
            out.push("--json".into());
            let names = opt_list(args, "names");
            if action != "clear" && names.is_empty() {
                return Err("add / remove 需要 names".to_string());
            }
            for n in names {
                out.push(n);
            }
        }
        other => {
            return Err(format!(
                "action 只支持 list / add / remove / clear，收到 {other}"
            ));
        }
    }
    Ok(out)
}

fn t_collect(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["collect".to_string(), "--json".to_string()];
    if let Some(n) = opt_num(args, "at") {
        out.push("--at".into());
        out.push(format!("{}", n));
    }
    if let Some(p) = opt_str(args, "out") {
        out.push("--out".into());
        out.push(p);
    }
    Ok(out)
}

fn t_compare(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["compare".to_string(), "--json".to_string()];
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    push_list(&mut out, args, "exclude", "--exclude");
    Ok(out)
}

fn t_transient(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = vec!["transient".to_string(), "--json".to_string()];
    if let Some(n) = opt_num(args, "min_mb") {
        out.push("--min-mb".into());
        out.push(format!("{}", n));
    }
    Ok(out)
}

fn t_commands(_: &Map<String, Value>) -> Result<Vec<String>, String> {
    Ok(vec!["commands".to_string(), "--json".to_string()])
}

fn t_baseline(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let action = opt_str(args, "action").unwrap_or_else(|| "info".to_string());
    let mut out = vec!["baseline".to_string(), action.clone(), "--json".to_string()];
    match action.as_str() {
        "export" => {
            if let Some(p) = opt_str(args, "to") {
                out.push("--to".into());
                out.push(p);
            }
        }
        "import" => {
            let pkg = opt_str(args, "package").ok_or("import 需要 package（.mbz 文件路径）")?;
            out.push(pkg);
        }
        "capture" => {
            if let Some(l) = opt_str(args, "label") {
                out.push("--label".into());
                out.push(l);
            }
        }
        _ => {}
    }
    Ok(out)
}

static TOOLS: &[Tool] = &[
    Tool {
        name: "mem_status",
        mutating: false,
        desc: "MemBase 概况：基准线信息、当前内存占用、线外项计数。只读。",
        extra: extra_none,
        build: t_status,
    },
    Tool {
        name: "mem_list",
        mutating: false,
        desc: "列出「基准线里没有、现在却在跑」的进程与服务。只读。",
        extra: extra_list,
        build: t_list,
    },
    Tool {
        name: "mem_plan",
        mutating: false,
        desc: "只生成线外项的处理计划，不执行。动手前的预演。",
        extra: extra_plan,
        build: t_plan,
    },
    Tool {
        name: "mem_trim",
        mutating: true,
        desc: "恢复基准：先礼后兵地关掉线外进程与服务，当场生效。会改系统状态。",
        extra: extra_plan,
        build: t_trim,
    },
    Tool {
        name: "mem_slim",
        mutating: true,
        desc: "基线瘦身：禁用开机不需要的服务 / 卸载 Store 应用 / 关功能开关，下次开机才生效。会改系统状态。",
        extra: extra_plan,
        build: t_slim,
    },
    Tool {
        name: "mem_restart",
        mutating: true,
        desc: "局部重启：关掉再按原路径拉起，并报告实测回收量。会改系统状态。",
        extra: extra_restart,
        build: t_restart,
    },
    Tool {
        name: "mem_kill",
        mutating: true,
        desc: "终止指定进程（先礼后兵）。会改系统状态。",
        extra: extra_kill,
        build: t_kill,
    },
    Tool {
        name: "mem_svc",
        mutating: true,
        desc: "单个服务操作：stop / start / disable / enable。disable = 下次开机不再起来。会改系统状态。",
        extra: extra_svc,
        build: t_svc,
    },
    Tool {
        name: "mem_feature",
        mutating: true,
        desc: "已知功能开关（只写 HKCU）：列出 / 关掉 / 撤销。off 会让它当场退出，但开机仍可能自启。",
        extra: extra_feature,
        build: t_feature,
    },
    Tool {
        name: "mem_startup",
        mutating: true,
        desc: "启动项扫描与开关：Run / 启动文件夹 / 非 Microsoft 计划任务。",
        extra: extra_startup,
        build: t_startup,
    },
    Tool {
        name: "mem_protect",
        mutating: true,
        desc: "保护名单：勾上保护 = 永不处理。",
        extra: extra_protect,
        build: t_protect,
    },
    Tool {
        name: "mem_collect",
        mutating: false,
        desc: "采集一次当前状态快照。",
        extra: extra_collect,
        build: t_collect,
    },
    Tool {
        name: "mem_compare",
        mutating: false,
        desc: "和最近一次基准做 diff，列出涨了 / 掉了 / 新出现的项。只读。",
        extra: extra_compare,
        build: t_compare,
    },
    Tool {
        name: "mem_transient",
        mutating: false,
        desc: "双采样对比：列出只在前一次采样出现的短命项。只读。",
        extra: extra_transient,
        build: t_transient,
    },
    Tool {
        name: "mem_baseline",
        mutating: true,
        desc: "基准线：info / verify / capture / export / import。",
        extra: extra_baseline,
        build: t_baseline,
    },
    Tool {
        name: "mem_commands",
        mutating: false,
        desc: "列出 MemBase 全部命令与参数（自描述）。只读。",
        extra: extra_none,
        build: t_commands,
    },
];

fn find_tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

// ---------------------------------------------------------------- 调用 CLI

/// 真正执行一条 CLI 命令并取回 JSON 输出。
///
/// 走的是「起一个子进程跑自己」的笨办法，理由：
///   * CLI 的每个动词都自己管退出码、往 stdout 打 JSON，直接复用最省事；
///   * 不会因为某个动词调了 `std::process::exit` 把整个 MCP 服务端带走。
#[cfg(windows)]
fn run_cli(argv: &[String]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| format!("取不到自身路径: {e}"))?;
    let out = std::process::Command::new(exe)
        .args(argv)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("启动子进程失败: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let code = out.status.code().unwrap_or(-1);

    if code != 0 {
        // 错误信息优先用 stderr；CLI 的错误都走 stderr
        let msg = if stderr.is_empty() {
            stdout.trim().to_string()
        } else {
            stderr
        };
        return Err(format!("退出码 {code}：{msg}"));
    }
    Ok(stdout.trim().to_string())
}

#[cfg(not(windows))]
fn run_cli(_argv: &[String]) -> Result<String, String> {
    Err("MCP 模式目前只在 Windows 上可用（平台层依赖 Win32）".to_string())
}

// ---------------------------------------------------------------- JSON-RPC

fn result_obj(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error_obj(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tools_list() -> Value {
    let arr: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            // mutating 的工具多两个门禁位（execute / yes）
            let schema = if t.mutating {
                schema_mutating((t.extra)())
            } else {
                schema_readonly((t.extra)())
            };
            json!({
                "name": t.name,
                "description": t.desc,
                "inputSchema": schema,
                "annotations": {
                    "readOnlyHint": !t.mutating,
                    "destructiveHint": t.mutating,
                },
            })
        })
        .collect();
    json!({ "tools": arr })
}

fn call_tool(name: &str, args: &Map<String, Value>) -> Value {
    let Some(t) = find_tool(name) else {
        return json!({
            "content": [{"type": "text", "text": format!("未知工具：{name}")}],
            "isError": true
        });
    };
    let argv = match (t.build)(args) {
        Ok(a) => a,
        Err(e) => {
            return json!({
                "content": [{"type": "text", "text": e}],
                "isError": true
            })
        }
    };
    match run_cli(&argv) {
        Ok(text) => {
            if text.is_empty() {
                json!({"content": [{"type": "text", "text": "(无输出)"}], "isError": false})
            } else {
                json!({"content": [{"type": "text", "text": text}], "isError": false})
            }
        }
        Err(e) => json!({
            "content": [{"type": "text", "text": e}],
            "isError": true
        }),
    }
}

/// 处理一条请求，返回要写回客户端的响应（None = 这是个通知，不用回）。
fn handle(req: &Value, sess: &mut Session) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned();
    let params = req
        .get("params")
        .and_then(|p| p.as_object())
        .cloned()
        .unwrap_or_default();

    // 新版协议把版本号放在每个请求的 _meta 里
    if let Some(v) = params
        .get("_meta")
        .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(|v| v.as_str())
    {
        sess.set(v);
    }

    // 通知（没有 id）不需要响应
    let Some(id) = id else {
        return None;
    };

    match method {
        "initialize" => {
            let want = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // 客户端要什么版本就尽量给什么版本；实在不认识就给最新的
            let chosen = if PROTOCOL_VERSIONS.contains(&want) {
                want.to_string()
            } else {
                PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1].to_string()
            };
            sess.set(&chosen);
            Some(result_obj(
                id,
                json!({
                    "protocolVersion": chosen,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {
                        "name": "membase",
                        "title": "MemBase 内存基准线",
                        "version": membase_core::VERSION,
                    },
                    "instructions": "先调 mem_status 看概况，再用 mem_plan 预演；要动手时 execute 与 yes 必须同时为 true。"
                }),
            ))
        }
        // 新版协议：一次性拿到能力与版本
        "server/discover" => Some(result_obj(
            id,
            json!({
                "protocolVersions": PROTOCOL_VERSIONS,
                "protocolVersion": sess.version(),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {
                    "name": "membase",
                    "title": "MemBase 内存基准线",
                    "version": membase_core::VERSION,
                },
            }),
        )),
        "ping" => Some(result_obj(id, json!({}))),
        "tools/list" => Some(result_obj(id, tools_list())),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params
                .get("arguments")
                .and_then(|a| a.as_object())
                .cloned()
                .unwrap_or_default();
            Some(result_obj(id, call_tool(name, &args)))
        }
        other => Some(error_obj(id, -32601, &format!("未实现的方法：{other}"))),
    }
}

/// MCP 服务端主循环：读 stdin 的一行行 JSON-RPC，把响应写到 stdout。
pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let reader = BufReader::new(stdin.lock());
    let mut sess = Session::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[x] 读 stdin 失败: {e}");
                return 3;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                // 解析不了也回一条，客户端才知道出了事
                let resp = error_obj(Value::Null, -32700, &format!("JSON 解析失败: {e}"));
                let _ = writeln!(out, "{resp}");
                let _ = out.flush();
                continue;
            }
        };
        if let Some(resp) = handle(&req, &mut sess) {
            if writeln!(out, "{resp}").is_err() {
                break;
            }
            if out.flush().is_err() {
                break;
            }
        }
    }
    0
}
