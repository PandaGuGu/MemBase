//! 桌面布局子命令：`MemBase.exe layout <动词>`
//!
//! 这里的每个动词都对应界面上的一个可点操作，反过来界面上的每个按钮
//! 也都能在 [`ACTIONS`] 表里找到自己的命令行写法 —— 这是本项目的硬约束：
//! **人能点的，脚本一定能发**。
//!
//! 退出码沿用 mem 侧的约定：
//!   0 成功 / 1 参数 / 2 没有布局或没有桌面 / 3 读写失败
//!   4 动作执行失败 / 5 被拒绝（缺 --yes 之类）

use membase_core::layout::Layout;
use membase_core::layout_store as store;
use membase_win::{actions, desktop::Desktop, repair, shellview::ShellView};

use crate::{
    has_flag, want_json, EXIT_ACTION_FAILED, EXIT_IO, EXIT_NO_BASELINE, EXIT_REFUSED,
    EXIT_USAGE,
};

/// 界面动作 → 命令行命令。GUI 的「命令行」页与按钮提示都读这张表，
/// 所以加一个按钮时忘了配 CLI，会在界面上直接暴露出来。
pub struct Action {
    /// 内部键（GUI 查表用）
    pub key: &'static str,
    /// 出现在哪个页面
    pub page: &'static str,
    /// 界面上的说法
    pub label: &'static str,
    /// 等价的命令行
    pub cli: &'static str,
    /// 一句话说明
    pub note: &'static str,
}

pub const ACTIONS: &[Action] = &[
    // ---------------- 进程页 ----------------
    Action {
        key: "proc_refresh",
        page: "进程",
        label: "刷新对比",
        cli: "MemBase.exe compare",
        note: "重新拍快照并与基准 diff",
    },
    Action {
        key: "proc_pick_all",
        page: "进程",
        label: "全选 N 项 / 清空勾选",
        cli: "MemBase.exe list --include-caution",
        note: "命令行用 --only / --exclude 表达勾选范围",
    },
    Action {
        key: "proc_include_caution",
        page: "进程",
        label: "含系统组件",
        cli: "MemBase.exe trim --include-caution",
        note: "放开「谨慎」档，命令行对应同一个开关",
    },
    Action {
        key: "proc_trim",
        page: "进程",
        label: "恢复选中 N 项",
        cli: "MemBase.exe trim --execute --yes --only <名字,名字>",
        note: "界面勾选的行 = --only 列表",
    },
    Action {
        key: "proc_protect",
        page: "进程",
        label: "行首「保护」勾选框",
        cli: "MemBase.exe protect add <名字>",
        note: "取消保护用 protect remove",
    },
    Action {
        key: "proc_protect_all",
        page: "进程",
        label: "全部保护 / 取消全部保护",
        cli: "MemBase.exe protect add <名字...> | MemBase.exe protect clear",
        note: "整份名单一次写入或清空",
    },
    Action {
        key: "proc_restart",
        page: "进程·可回收",
        label: "实测重启选中 N 项",
        cli: "MemBase.exe restart <名字> --execute --yes",
        note: "只处理基准内跑久了的增长项；关掉再按原路径拉起，等 3 秒实测回收量（可能为负）",
    },
    // ---------------- 瘦身页 ----------------
    Action {
        key: "slim_execute",
        page: "瘦身",
        label: "处理选中 N 项",
        cli: "MemBase.exe slim --execute --yes --only <名字,名字>",
        note: "禁用服务 / 卸载 Store 应用 / 关功能开关；下次开机才反映到基准",
    },
    Action {
        key: "slim_refresh",
        page: "瘦身",
        label: "重新扫描",
        cli: "MemBase.exe slim",
        note: "读基准服务清单 + 当前功能开关，已禁用/已卸载的自动跳过",
    },
    Action {
        key: "slim_caution",
        page: "瘦身",
        label: "含系统组件",
        cli: "MemBase.exe slim --include-caution",
        note: "放开 Windows 目录下的组件，禁了可能断网，必须逐条判断",
    },
    // ---------------- 启动项页 ----------------
    Action {
        key: "startup_refresh",
        page: "启动项",
        label: "重新扫描",
        cli: "MemBase.exe startup list",
        note: "Run / 启动文件夹 / 非 Microsoft 计划任务",
    },
    Action {
        key: "startup_toggle",
        page: "启动项",
        label: "行尾「禁用 / 启用」",
        cli: "MemBase.exe startup disable|enable <名称> --execute --yes",
        note: "只改批准记录或任务状态，不删原始命令行",
    },
    // ---------------- 开机采集页 ----------------
    Action {
        key: "boot_dual",
        page: "开机采集",
        label: "双采样",
        cli: "MemBase.exe install --at <秒> --also <秒>",
        note: "一次开机采两点，用来分辨短命项；配合 MemBase.exe transient",
    },
    // ---------------- 基准页 ----------------
    Action {
        key: "baseline_capture",
        page: "基准",
        label: "扫描当前状态设为基准",
        cli: "MemBase.exe baseline capture",
        note: "不用重启，立刻把当前态立成基准",
    },
    Action {
        key: "baseline_export",
        page: "基准",
        label: "导出基准",
        cli: "MemBase.exe baseline export --to <文件.mbz>",
        note: "打包成单文件，可拷到别的机器",
    },
    Action {
        key: "baseline_import",
        page: "基准",
        label: "导入基准",
        cli: "MemBase.exe baseline import <文件.mbz>",
        note: "先校验 crc 再落地，坏包不动",
    },
    // ---------------- 保护名单页 ----------------
    Action {
        key: "protect_remove",
        page: "保护名单",
        label: "行尾「移除」",
        cli: "MemBase.exe protect remove <名字>",
        note: "只删这一条，通配规则原样保留",
    },
    Action {
        key: "protect_clear",
        page: "保护名单",
        label: "清空全部",
        cli: "MemBase.exe protect clear",
        note: "删掉 protect.txt 里所有规则",
    },
    // ---------------- 桌面布局页 ----------------
    Action {
        key: "layout_save",
        page: "桌面布局",
        label: "保存当前桌面",
        cli: "MemBase.exe layout save <名字>",
        note: "记下每个图标的名字与像素坐标",
    },
    Action {
        key: "layout_restore",
        page: "桌面布局",
        label: "恢复选中布局",
        cli: "MemBase.exe layout restore <名字> --execute --yes",
        note: "不加 --execute 就是演练，只报会动哪些",
    },
    Action {
        key: "layout_diff",
        page: "桌面布局",
        label: "差异预览",
        cli: "MemBase.exe layout diff <名字>",
        note: "只读：几个会动 / 几个已消失 / 几个新增",
    },
    Action {
        key: "layout_delete",
        page: "桌面布局",
        label: "删除布局",
        cli: "MemBase.exe layout delete <名字> --yes",
        note: "删布局文件，不动桌面",
    },
    Action {
        key: "layout_ghost",
        page: "桌面布局",
        label: "叠加当前位置",
        cli: "MemBase.exe layout diff <名字> --positions",
        note: "界面画蓝色虚线；命令行打印当前坐标",
    },
    Action {
        key: "layout_probe",
        page: "桌面布局",
        label: "接口诊断",
        cli: "MemBase.exe layout probe",
        note: "检查 shell 官方接口能不能用（决定能否持久化）",
    },
    Action {
        key: "layout_repair",
        page: "桌面布局",
        label: "修复布局缓存",
        cli: "MemBase.exe layout repair --execute --yes",
        note: "备份并删除 IconLayouts，重启后系统重建",
    },
    Action {
        key: "layout_open_dir",
        page: "桌面布局",
        label: "打开布局目录",
        cli: "MemBase.exe layout dir --open",
        note: "不加 --open 只打印路径",
    },
    Action {
        key: "layout_refresh",
        page: "桌面布局",
        label: "刷新",
        cli: "MemBase.exe layout list",
        note: "重新读布局目录 + 桌面状态",
    },
    // ---------------- 开机采集页 ----------------
    Action {
        key: "boot_install",
        page: "开机采集",
        label: "安排下次开机采集",
        cli: "MemBase.exe install --at <秒>",
        note: "SYSTEM 权限、无窗口，采完自动销毁",
    },
    Action {
        key: "boot_uninstall",
        page: "开机采集",
        label: "取消安排",
        cli: "MemBase.exe uninstall",
        note: "删掉开机计划任务",
    },
    // ---------------- 工具页 ----------------
    Action {
        key: "screen_off",
        page: "工具",
        label: "息屏",
        cli: "MemBase.exe screen off --seconds <秒> --exit-key esc",
        note: "全屏黑；按热键退出，到点自动亮（0 秒 = 不定时）",
    },
    Action {
        key: "screen_duration",
        page: "工具",
        label: "息屏时长（时:分:秒）",
        cli: "MemBase.exe screen duration <1:05:09>",
        note: "界面三个输入框改一下就落盘，两边同一个文件",
    },
    Action {
        key: "screen_key",
        page: "工具",
        label: "退出热键",
        cli: "MemBase.exe screen key <esc|space|enter|any>",
        note: "默认 ESC；界面下拉框与这条命令同一个文件",
    },
    Action {
        key: "screen_power",
        page: "工具",
        label: "同时关闭显示器",
        cli: "MemBase.exe screen power <on|off>",
        note: "默认关：只盖全屏黑窗，不动显示器电源（关了可能叫不醒）",
    },
    // ---------------- 设置页 ----------------
    Action {
        key: "settings_open_dir",
        page: "设置",
        label: "打开数据目录",
        cli: "MemBase.exe config get --open",
        note: "先告诉你目录在哪，再打开它",
    },
    Action {
        key: "settings_choose_dir",
        page: "设置",
        label: "更改数据目录…",
        cli: "MemBase.exe config set <目录>",
        note: "写入 %APPDATA%\\MemBase\\config.json",
    },
    Action {
        key: "theme_light",
        page: "设置",
        label: "外观 · 浅色",
        cli: "MemBase.exe theme set light",
        note: "落在 theme.txt，重启后保持",
    },
    Action {
        key: "theme_dark",
        page: "设置",
        label: "外观 · 深色",
        cli: "MemBase.exe theme set dark",
        note: "落在 theme.txt，重启后保持",
    },
];

/// 按 key 取命令行写法（GUI 的 hover 提示用）
pub fn cli_for(key: &str) -> Option<&'static str> {
    ACTIONS.iter().find(|a| a.key == key).map(|a| a.cli)
}

/// `layout` 的子命令清单，供 `commands --json` 与 usage 使用
pub const LAYOUT_COMMANDS: &[(&str, &str, &str)] = &[
    ("list", "列出已保存的布局", "--json"),
    ("save", "保存当前桌面布局", "<名字> --json"),
    ("restore", "恢复布局（默认演练）", "[名字] --execute --yes --json"),
    ("diff", "只看差异，不动桌面", "[名字] --positions --json"),
    ("show", "查看布局内容", "[名字] --json"),
    ("delete", "删除布局", "<名字> --yes"),
    ("status", "桌面状态：图标数/自动排列/分辨率/缓存", "--json"),
    ("probe", "诊断 shell 官方接口是否可用", "--json"),
    ("repair", "备份并清理系统布局缓存（默认演练）", "--execute --yes --json"),
    ("dir", "布局目录", "--open"),
];

fn name_of(args: &[String]) -> Option<String> {
    args.iter()
        .filter(|a| !a.starts_with("--"))
        .find(|a| !a.trim().is_empty())
        .map(|a| a.trim().to_string())
}

fn resolve(args: &[String]) -> Result<String, i32> {
    if let Some(n) = name_of(args) {
        return Ok(n);
    }
    match store::latest() {
        Some(n) => Ok(n),
        None => {
            eprintln!("[x] 还没有任何布局，先执行：MemBase.exe layout save <名字>");
            Err(EXIT_NO_BASELINE)
        }
    }
}

fn suffix(list: &[String], max: usize) -> String {
    if list.is_empty() {
        return String::new();
    }
    let head: Vec<String> = list.iter().take(max).cloned().collect();
    let mut s = head.join(", ");
    if list.len() > max {
        s.push_str("...");
    }
    s
}

fn jstr_arr(list: &[String]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|s| membase_core::bundle::jstr(s))
        .collect();
    format!("[{}]", items.join(","))
}

/// 入口：`MemBase.exe layout <动词> [参数]`
pub fn cmd_layout(args: &[String]) -> i32 {
    // 读桌面要 DPI 感知，走 shell 接口要 COM 套间；和 GUI 用同一份初始化
    membase_win::init();
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let r = match sub.as_str() {
        "list" | "ls" => layout_list(&rest),
        "save" | "capture" => layout_save(&rest),
        "restore" | "apply" => layout_restore(&rest),
        "diff" | "plan" => layout_diff(&rest),
        "show" | "cat" => layout_show(&rest),
        "delete" | "rm" => layout_delete(&rest),
        "status" => layout_status(&rest),
        "probe" => layout_probe(&rest),
        "repair" | "fix" => layout_repair(&rest),
        "dir" | "folder" => layout_dir(&rest),
        "" => {
            println!("{}", layout_usage());
            0
        }
        "help" | "-h" | "--help" => {
            println!("{}", layout_usage());
            0
        }
        other => {
            eprintln!("未知子命令: layout {}", other);
            println!("{}", layout_usage());
            EXIT_USAGE
        }
    };
    r
}

pub fn layout_usage() -> String {
    let mut s = String::from(
        "MemBase.exe layout —— 桌面图标布局（保存 / 差异 / 恢复 / 修复）\n\n用法: MemBase.exe layout <动词> [名字] [选项]\n\n",
    );
    for (n, d, a) in LAYOUT_COMMANDS {
        s.push_str(&format!("  {:<8} {:<34} {}\n", n, d, a));
    }
    s.push_str("\n选项:\n");
    s.push_str("  --execute            restore / repair 真正动手（不加就是演练）\n");
    s.push_str("  --yes                跳过确认提示\n");
    s.push_str("  --positions          diff 时逐个打印当前坐标\n");
    s.push_str("  --open               dir 时顺便打开资源管理器\n");
    s.push_str("  --json               结构化输出\n");
    s.push_str("\n注意: 恢复依赖「自动排列图标」处于关闭状态，检测到会自动帮你关掉；\n");
    s.push_str("      想看 shell 官方接口能不能用，跑 MemBase.exe layout probe。");
    s
}

fn layout_list(args: &[String]) -> i32 {
    let items = store::list();
    if want_json(args) {
        let arr: Vec<String> = items
            .iter()
            .map(|e| {
                format!(
                    "{{\"name\":{},\"file\":{},\"created\":{},\"icons\":{},\"screen\":{{\"w\":{},\"h\":{}}}}}",
                    membase_core::bundle::jstr(e.display_name()),
                    membase_core::bundle::jstr(&e.path.to_string_lossy()),
                    membase_core::bundle::jstr(&e.stamp),
                    e.count(),
                    e.data.screen.w,
                    e.data.screen.h
                )
            })
            .collect();
        println!(
            "{{\"dir\":{},\"count\":{},\"layouts\":[{}]}}",
            membase_core::bundle::jstr(&store::dir().to_string_lossy()),
            items.len(),
            arr.join(",")
        );
        return 0;
    }
    if items.is_empty() {
        println!("还没有任何布局（目录: {}）", store::dir().display());
        println!("存一份看看：MemBase.exe layout save 工作");
        return 0;
    }
    println!("布局目录: {}", store::dir().display());
    for e in &items {
        println!(
            "  {:<24} {:>3} 个图标   {}   {}",
            e.display_name(),
            e.count(),
            e.stamp,
            e.data.screen_text()
        );
    }
    0
}

fn layout_save(args: &[String]) -> i32 {
    let name = name_of(args).unwrap_or_else(|| {
        chrono::Local::now()
            .format("auto-%Y%m%d-%H%M%S")
            .to_string()
    });
    let layout = match actions::snapshot(&name) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[x] 读不到桌面（explorer 可能没在跑）: {:#}", e);
            return EXIT_IO;
        }
    };
    if layout.icons.is_empty() {
        eprintln!("[x] 桌面上没有图标（或图标被隐藏），未保存");
        return EXIT_ACTION_FAILED;
    }
    let path = match store::save(&layout) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[x] {}: {:#}", "保存失败", e);
            return EXIT_IO;
        }
    };
    if want_json(args) {
        println!(
            "{{\"ok\":true,\"name\":{},\"file\":{},\"icons\":{},\"auto_arrange\":{},\"screen\":{{\"w\":{},\"h\":{}}}}}",
            membase_core::bundle::jstr(&layout.name),
            membase_core::bundle::jstr(&path.to_string_lossy()),
            layout.icons.len(),
            layout.auto_arrange,
            layout.screen.w,
            layout.screen.h
        );
    } else {
        println!(
            "已保存 {} 个图标的位置 -> {}",
            layout.icons.len(),
            path.display()
        );
        if layout.auto_arrange {
            println!("  ⚠ 当前开着「自动排列图标」：建议先关掉，否则恢复会被立刻重排");
        }
    }
    0
}

fn layout_show(args: &[String]) -> i32 {
    let name = match resolve(args) {
        Ok(n) => n,
        Err(c) => return c,
    };
    let data = match store::load(&name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[x] {}", e);
            return EXIT_NO_BASELINE;
        }
    };
    if want_json(args) {
        let arr: Vec<String> = data
            .icons
            .iter()
            .map(|i| {
                format!(
                    "{{\"name\":{},\"x\":{},\"y\":{}}}",
                    membase_core::bundle::jstr(&i.name),
                    i.x,
                    i.y
                )
            })
            .collect();
        println!(
            "{{\"name\":{},\"created\":{},\"screen\":{{\"w\":{},\"h\":{}}},\"auto_arrange\":{},\"icons\":[{}]}}",
            membase_core::bundle::jstr(&data.name),
            membase_core::bundle::jstr(&data.created),
            data.screen.w,
            data.screen.h,
            data.auto_arrange,
            arr.join(",")
        );
        return 0;
    }
    println!(
        "[{}]  {}  {}",
        data.name,
        data.created,
        data.screen_text()
    );
    for it in &data.icons {
        println!("  ({:>6}, {:>6})  {}", it.x, it.y, it.name);
    }
    0
}

fn layout_diff(args: &[String]) -> i32 {
    let name = match resolve(args) {
        Ok(n) => n,
        Err(c) => return c,
    };
    let data = match store::load(&name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[x] {}", e);
            return EXIT_NO_BASELINE;
        }
    };
    let plan = match actions::plan(&data) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[x] 读不到桌面: {:#}", e);
            return EXIT_IO;
        }
    };
    let positions = has_flag(args, "--positions");

    if want_json(args) {
        let mut rows: Vec<String> = Vec::new();
        if positions {
            // 只输出「会动的」那些，和界面的蓝色虚线一一对应
            let cur = actions::current_positions().unwrap_or_default();
            let now: std::collections::HashMap<String, (i32, i32)> = cur
                .iter()
                .map(|i| (i.name.to_lowercase(), (i.x, i.y)))
                .collect();
            for it in &data.icons {
                if let Some(&(cx, cy)) = now.get(&it.name.to_lowercase()) {
                    if cx != it.x || cy != it.y {
                        rows.push(format!(
                            "{{\"name\":{},\"from\":[{},{}],\"to\":[{},{}]}}",
                            membase_core::bundle::jstr(&it.name),
                            cx,
                            cy,
                            it.x,
                            it.y
                        ));
                    }
                }
            }
        }
        println!(
            "{{\"layout\":{},\"total\":{},\"to_move\":{},\"missing\":{},\"extra\":{},\"moves\":[{}]}}",
            membase_core::bundle::jstr(&name),
            plan.total,
            plan.to_move,
            jstr_arr(&plan.missing),
            jstr_arr(&plan.extra),
            rows.join(",")
        );
        return 0;
    }

    println!("布局 [{}]：{} 个图标", name, plan.total);
    println!("  需要移动  : {}", plan.to_move);
    println!("  已消失    : {}{}", plan.missing.len(), fmt_suffix(&plan.missing, 6));
    println!("  新增未记录: {}{}", plan.extra.len(), fmt_suffix(&plan.extra, 6));
    if positions {
        let cur = actions::current_positions().unwrap_or_default();
        let now: std::collections::HashMap<String, (i32, i32)> = cur
            .iter()
            .map(|i| (i.name.to_lowercase(), (i.x, i.y)))
            .collect();
        println!();
        for it in &data.icons {
            let Some(&(cx, cy)) = now.get(&it.name.to_lowercase()) else {
                continue;
            };
            if cx == it.x && cy == it.y {
                continue;
            }
            println!(
                "  [移动] {}: ({},{}) -> ({},{})",
                it.name, cx, cy, it.x, it.y
            );
        }
    }
    if plan.to_move > 0 {
        println!(
            "\n正式恢复：MemBase.exe layout restore {} --execute --yes",
            name
        );
    }
    0
}

fn fmt_suffix(list: &[String], max: usize) -> String {
    if list.is_empty() {
        String::new()
    } else {
        format!("  {}", suffix(list, max))
    }
}

fn layout_restore(args: &[String]) -> i32 {
    let name = match resolve(args) {
        Ok(n) => n,
        Err(c) => return c,
    };
    let data: Layout = match store::load(&name) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[x] {}", e);
            return EXIT_NO_BASELINE;
        }
    };
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");

    if !execute {
        // 演练：逐条列出会动的位置，和界面的差异视图同源
        let plan = match actions::plan(&data) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[x] 读不到桌面: {:#}", e);
                return EXIT_IO;
            }
        };
        let cur = actions::current_positions().unwrap_or_default();
        let now: std::collections::HashMap<String, (i32, i32)> = cur
            .iter()
            .map(|i| (i.name.to_lowercase(), (i.x, i.y)))
            .collect();
        let mut moves: Vec<String> = Vec::new();
        for it in &data.icons {
            if let Some(&(cx, cy)) = now.get(&it.name.to_lowercase()) {
                if cx != it.x || cy != it.y {
                    moves.push(format!(
                        "{{\"name\":{},\"from\":[{},{}],\"to\":[{},{}]}}",
                        membase_core::bundle::jstr(&it.name),
                        cx,
                        cy,
                        it.x,
                        it.y
                    ));
                }
            }
        }
        if want_json(args) {
            println!(
                "{{\"ok\":true,\"dry_run\":true,\"layout\":{},\"total\":{},\"to_move\":{},\"missing\":{},\"moves\":[{}]}}",
                membase_core::bundle::jstr(&name),
                plan.total,
                plan.to_move,
                jstr_arr(&plan.missing),
                moves.join(",")
            );
            return 0;
        }
        println!("[演练] 布局 [{}]，还没动桌面", name);
        for it in &data.icons {
            if let Some(&(cx, cy)) = now.get(&it.name.to_lowercase()) {
                let flag = if (cx, cy) != (it.x, it.y) { "移动" } else { "不动" };
                if flag == "移动" {
                    println!("  [{}] {}: ({},{}) -> ({},{})", flag, it.name, cx, cy, it.x, it.y);
                }
            }
        }
        println!(
            "合计 {} 个，其中 {} 个位置不一致，{} 个已消失",
            plan.total,
            plan.to_move,
            plan.missing.len()
        );
        println!(
            "\n真正动手：MemBase.exe layout restore {} --execute --yes",
            name
        );
        return 0;
    }

    if !yes {
        eprintln!("[!] 恢复会改动桌面图标位置，请加 --yes 确认");
        return EXIT_REFUSED;
    }

    let r = match actions::restore(&data) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[x] 恢复失败: {:#}", e);
            return EXIT_ACTION_FAILED;
        }
    };

    if want_json(args) {
        println!(
            "{{\"ok\":{},\"layout\":{},\"restored\":{},\"moved\":{},\"rounds\":{},\"shell_api\":{},\"auto_arrange_disabled\":{},\"failed\":{},\"missing\":{},\"extra\":{}}}",
            r.failed.is_empty(),
            membase_core::bundle::jstr(&name),
            r.restored,
            r.moved,
            r.rounds,
            r.used_shell_api,
            r.auto_arrange_disabled,
            jstr_arr(&r.failed),
            jstr_arr(&r.missing),
            jstr_arr(&r.extra)
        );
    } else {
        println!(
            "已恢复 {} 个图标到布局 [{}]（移动 {} 个，{} 轮写入）",
            r.restored, name, r.moved, r.rounds
        );
        if r.auto_arrange_disabled {
            println!("已关闭「自动排列图标」");
        }
        if !r.failed.is_empty() {
            println!("  {} 个没摆到位:{}", r.failed.len(), fmt_suffix(&r.failed, 6));
        }
        if !r.missing.is_empty() {
            println!(
                "  未找到 {} 个（可能已删除或改名）:{}",
                r.missing.len(),
                fmt_suffix(&r.missing, 8)
            );
        }
        if !r.extra.is_empty() {
            println!("  新增 {} 个图标未在布局中，保持原位", r.extra.len());
        }
        println!(
            "  {}",
            if r.used_shell_api {
                "位置已由 shell 写入，重启后保持"
            } else {
                "shell 接口不可用，只改了内存，重启可能被系统覆盖"
            }
        );
        if !r.failed.is_empty() || !r.used_shell_api {
            println!("  提示：先跑一次 MemBase.exe layout repair --execute --yes 清理系统布局缓存");
        }
    }
    if r.failed.is_empty() {
        0
    } else {
        EXIT_ACTION_FAILED
    }
}

fn layout_delete(args: &[String]) -> i32 {
    let Some(name) = name_of(args) else {
        eprintln!("用法: MemBase.exe layout delete <名字> --yes");
        return EXIT_USAGE;
    };
    if !store::exists(&name) {
        eprintln!("[x] 布局不存在: {}", name);
        return EXIT_NO_BASELINE;
    }
    if !has_flag(args, "--yes") {
        eprintln!("[!] 删除不可撤销，请加 --yes 确认（布局文件: {}）", store::path_of(&name).display());
        return EXIT_REFUSED;
    }
    match store::delete(&name) {
        Ok(()) => {
            if want_json(args) {
                println!(
                    "{{\"ok\":true,\"deleted\":{}}}",
                    membase_core::bundle::jstr(&name)
                );
            } else {
                println!("已删除: {}", name);
            }
            0
        }
        Err(e) => {
            eprintln!("[x] {}", e);
            EXIT_IO
        }
    }
}

fn layout_status(args: &[String]) -> i32 {
    let d = match Desktop::open() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[x] 连不上桌面（explorer 可能没在跑）: {:#}", e);
            return EXIT_IO;
        }
    };
    let count = d.count().unwrap_or(0);
    let arrange = d.auto_arrange();
    let (sw, sh) = membase_win::sys::screen_size();
    let pid = d.pid();
    let layouts = store::list().len();
    let cache = repair::describe_cache();
    let shell_ok = ShellView::try_open().is_some();

    if want_json(args) {
        println!(
            "{{\"explorer_pid\":{},\"icons\":{},\"auto_arrange\":{},\"screen\":{{\"w\":{},\"h\":{}}},\"layout_dir\":{},\"layouts\":{},\"shell_api\":{},\"cache\":{}}}",
            pid,
            count,
            arrange,
            sw,
            sh,
            membase_core::bundle::jstr(&store::dir().to_string_lossy()),
            layouts,
            shell_ok,
            membase_core::bundle::jstr(&cache)
        );
        return 0;
    }
    println!("explorer pid      : {}", pid);
    println!("桌面图标数量      : {}", count);
    println!(
        "自动排列图标      : {}",
        if arrange {
            "开启（恢复前必须关闭，工具会自动关）"
        } else {
            "已关闭 ✓"
        }
    );
    println!("主屏分辨率        : {}x{}", sw, sh);
    println!("shell 官方接口    : {}", if shell_ok { "可用（恢复可持久化）" } else { "不可用（只能改内存）" });
    println!("布局目录          : {}", store::dir().display());
    println!("已存布局数量      : {}", layouts);
    println!("系统布局缓存      : {}", cache);
    0
}

fn layout_probe(args: &[String]) -> i32 {
    match ShellView::try_open() {
        Some(sv) => {
            let items = sv.items();
            if want_json(args) {
                let arr: Vec<String> = items
                    .iter()
                    .take(8)
                    .map(|i| {
                        format!(
                            "{{\"name\":{},\"x\":{},\"y\":{}}}",
                            membase_core::bundle::jstr(&i.name),
                            i.x,
                            i.y
                        )
                    })
                    .collect();
                println!(
                    "{{\"shell_api\":true,\"count\":{},\"items\":[{}]}}",
                    items.len(),
                    arr.join(",")
                );
            } else {
                println!("shell 桌面视图: 可用");
                println!("条目数量      : {}", items.len());
                for it in items.iter().take(8) {
                    println!("  ({:>6}, {:>6})  {}", it.x, it.y, it.name);
                }
            }
            0
        }
        None => {
            if want_json(args) {
                println!("{{\"shell_api\":false,\"count\":0,\"items\":[]}}");
            } else {
                println!("shell 桌面视图: 拿不到（恢复会退回改内存的方式，重启可能被覆盖）");
            }
            EXIT_ACTION_FAILED
        }
    }
}

fn layout_repair(args: &[String]) -> i32 {
    let execute = has_flag(args, "--execute");
    let yes = has_flag(args, "--yes");
    let cache = match repair::read_icon_layouts() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[x] 读缓存失败: {:#}", e);
            return EXIT_IO;
        }
    };

    let Some(blob) = cache else {
        if want_json(args) {
            println!("{{\"ok\":true,\"needed\":false,\"message\":\"已经是干净状态\"}}");
        } else {
            println!("系统里没有 IconLayouts 缓存，已经是干净状态，不需要修复");
        }
        return 0;
    };

    if !execute {
        let msg = format!(
            "缓存 {} 字节，修复会先备份到 {} 再删除它",
            blob.len(),
            repair::backup_path().display()
        );
        if want_json(args) {
            println!(
                "{{\"ok\":true,\"dry_run\":true,\"needed\":true,\"bytes\":{},\"backup\":{}}}",
                blob.len(),
                membase_core::bundle::jstr(&repair::backup_path().to_string_lossy())
            );
        } else {
            println!("[演练] {}", msg);
            println!("真正动手：MemBase.exe layout repair --execute --yes");
        }
        return 0;
    }
    if !yes {
        eprintln!("[!] 修复会删除系统布局缓存（先备份），请加 --yes 确认");
        return EXIT_REFUSED;
    }

    match repair::repair() {
        Ok(None) => {
            if want_json(args) {
                println!("{{\"ok\":true,\"needed\":false}}");
            } else {
                println!("系统里没有 IconLayouts 缓存，不需要修复");
            }
            0
        }
        Ok(Some(info)) => {
            if want_json(args) {
                println!(
                    "{{\"ok\":true,\"needed\":true,\"bytes\":{},\"backup\":{}}}",
                    info.bytes,
                    membase_core::bundle::jstr(&info.backup.to_string_lossy())
                );
            } else {
                println!(
                    "已备份原缓存 -> {}（{} 字节）",
                    info.backup.display(),
                    info.bytes
                );
                println!("已删除 IconLayouts（系统下次启动会重建一份干净的）");
                println!("\n接着做两步：");
                println!("  1. 重启电脑 —— 桌面会先变成默认排列，这是正常的");
                println!("  2. 重启后执行：MemBase.exe layout restore <名字> --execute --yes");
            }
            0
        }
        Err(e) => {
            eprintln!("[x] 修复失败: {:#}", e);
            EXIT_IO
        }
    }
}

fn layout_dir(args: &[String]) -> i32 {
    let dir = store::dir();
    if has_flag(args, "--open") {
        membase_win::win::open_folder(&dir.to_string_lossy());
    }
    if want_json(args) {
        println!(
            "{{\"dir\":{},\"exists\":{}}}",
            membase_core::bundle::jstr(&dir.to_string_lossy()),
            dir.is_dir()
        );
    } else {
        println!("{}", dir.display());
    }
    0
}
