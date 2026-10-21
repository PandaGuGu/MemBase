//! 对比：当前实时快照 vs 最近一次基准快照

use crate::snapshot::{self, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub fn read_csv(path: &Path) -> Vec<Vec<String>> {
    let s = fs::read_to_string(path).unwrap_or_default();
    let s = s.trim_start_matches('\u{feff}');
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut row: Vec<String> = Vec::new();
    let mut in_q = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if in_q {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    it.next();
                    field.push('"');
                } else {
                    in_q = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => in_q = true,
                ',' => {
                    row.push(field.clone());
                    field.clear();
                }
                '\r' => {}
                '\n' => {
                    row.push(field.clone());
                    field.clear();
                    if !(row.len() == 1 && row[0].is_empty()) {
                        rows.push(row.clone());
                    }
                    row.clear();
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn col(header: &[String], name: &str) -> Option<usize> {
    header.iter().position(|h| h.eq_ignore_ascii_case(name))
}

pub fn find_latest_baseline(root: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let rd = fs::read_dir(root).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("run_") {
            continue;
        }
        if let Ok(sub) = fs::read_dir(&p) {
            for s in sub.flatten() {
                let sp = s.path();
                if !sp.is_dir() {
                    continue;
                }
                let sj = sp.join("system.json");
                if sj.exists() {
                    let mt = fs::metadata(&sj)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH);
                    if best.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
                        best = Some((mt, sp));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

pub struct ProcDiff {
    pub name: String,
    pub status: &'static str,
    pub base_inst: u32,
    pub live_inst: u32,
    pub base_ws: f64,
    pub live_ws: f64,
    pub delta: f64,
    pub pct: Option<f64>,
}

pub fn compare(root: &Path, min_mb: f64, excludes: &[String], exclude_self: bool) -> std::io::Result<()> {
    let base_dir = match find_latest_baseline(root) {
        Some(p) => p,
        None => {
            eprintln!("[x] 在 {} 下找不到基准快照（run_*\\system.json）", root.display());
            std::process::exit(2);
        }
    };
    println!("[..] 基准：{}", base_dir.display());

    let base_rows = read_csv(&base_dir.join("processes_by_name.csv"));
    if base_rows.is_empty() {
        eprintln!("[x] 基准里没有 processes_by_name.csv");
        std::process::exit(2);
    }
    let bh = &base_rows[0];
    let (i_name, i_inst, i_ws, i_priv) = (
        col(bh, "ProcessName"),
        col(bh, "Instances"),
        col(bh, "WorkingSetMB"),
        col(bh, "PrivateMB"),
    );
    let mut base: BTreeMap<String, (u32, f64, f64)> = BTreeMap::new();
    // 内存没读到的项（未提权采集 / 受保护进程）：这类项的涨跌无从判断
    let mut base_blind: BTreeSet<String> = BTreeSet::new();
    for r in base_rows.iter().skip(1) {
        if let (Some(a), Some(b), Some(c), Some(d)) = (i_name, i_inst, i_ws, i_priv) {
            let name = r.get(a).cloned().unwrap_or_default();
            let ws_cell = r.get(c).map(|s| s.trim().to_string()).unwrap_or_default();
            if ws_cell.is_empty() {
                base_blind.insert(name.clone());
            }
            base.insert(
                name,
                (
                    r.get(b).and_then(|v| v.parse().ok()).unwrap_or(0),
                    ws_cell.parse().unwrap_or(0.0),
                    r.get(d).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                ),
            );
        }
    }

    let mut base_svc: BTreeMap<String, String> = BTreeMap::new();
    let svc_rows = read_csv(&base_dir.join("services.csv"));
    if !svc_rows.is_empty() {
        let sh = &svc_rows[0];
        let (sn, ss) = (col(sh, "ServiceName"), col(sh, "State"));
        for r in svc_rows.iter().skip(1) {
            if let (Some(a), Some(b)) = (sn, ss) {
                base_svc.insert(
                    r.get(a).cloned().unwrap_or_default(),
                    r.get(b).cloned().unwrap_or_default(),
                );
            }
        }
    }

    // 老快照的坑：未提权采集时内存被写成了 0.00，而不是留空。
    // 判据是「工作集 0 但线程数非 0」——真占 0 内存的进程不会有活着的线程。
    // 命中就按「内存未知」处理，别让它冒充成「暴跌几百 MB」。
    let probe = read_csv(&base_dir.join("processes.csv"));
    if !probe.is_empty() {
        let h = &probe[0];
        if let (Some(a), Some(w), Some(t)) = (
            col(h, "ProcessName"),
            col(h, "WorkingSetMB"),
            col(h, "Threads"),
        ) {
            for r in probe.iter().skip(1) {
                let zero_ws = r
                    .get(w)
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .map(|v| v == 0.0)
                    .unwrap_or(false);
                let threads = r.get(t).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                if zero_ws && threads > 0 {
                    if let Some(n) = r.get(a) {
                        base_blind.insert(n.clone());
                    }
                }
            }
        }
    }

    println!("[..] 采集当前实时快照");
    let live: Snapshot = snapshot::snapshot_now(exclude_self);
    let mut live_agg: BTreeMap<String, (u32, f64, f64)> = BTreeMap::new();
    let mut live_blind: BTreeSet<String> = BTreeSet::new();
    for p in &live.procs {
        let e = live_agg
            .entry(p.name.clone())
            .or_insert((0u32, 0.0f64, 0.0f64));
        e.0 += 1;
        match p.ws_mb {
            Some(v) => e.1 += v,
            None => {
                live_blind.insert(p.name.clone());
            }
        }
        if let Some(v) = p.priv_mb {
            e.2 += v;
        }
    }

    let mut diffs: Vec<ProcDiff> = Vec::new();
    let mut blind: Vec<String> = Vec::new();
    let mut names: Vec<String> = base.keys().cloned().collect();
    for k in live_agg.keys() {
        if !base.contains_key(k) {
            names.push(k.clone());
        }
    }
    for n in names {
        let b = base.get(&n).copied().unwrap_or((0, 0.0, 0.0));
        let l = live_agg.get(&n).copied().unwrap_or((0, 0.0, 0.0));
        let delta = l.1 - b.1;
        // 两边都在、但有一边的内存没读到：涨跌算不出来，单独记下来，别冒充「减少」
        if b.0 > 0 && l.0 > 0 && (base_blind.contains(&n) || live_blind.contains(&n)) {
            blind.push(n);
            continue;
        }
        let pct = if b.1 > 0.0 {
            Some((delta / b.1 * 1000.0).round() / 10.0)
        } else {
            None
        };
        let status = if b.0 == 0 {
            "新增"
        } else if l.0 == 0 {
            "已退出"
        } else if delta > 0.0 {
            "增长"
        } else if delta < 0.0 {
            "减少"
        } else {
            "持平"
        };
        if excludes.iter().any(|p| wildcard_match(&n, p)) {
            continue;
        }
        if delta.abs() < min_mb && status != "已退出" {
            continue;
        }
        diffs.push(ProcDiff {
            name: n,
            status,
            base_inst: b.0,
            live_inst: l.0,
            base_ws: b.1,
            live_ws: l.1,
            delta: (delta * 100.0).round() / 100.0,
            pct,
        });
    }
    diffs.sort_by(|a, b| b.delta.partial_cmp(&a.delta).unwrap_or(std::cmp::Ordering::Equal));

    let live_priv: f64 = live.procs.iter().filter_map(|p| p.priv_mb).sum();
    let live_threads: u32 = live.procs.iter().map(|p| p.threads).sum();
    let live_handles: u32 = live.procs.iter().map(|p| p.handles).sum();

    // 基准的整机数值：进程表算总数，system.json 取内存
    let proc_rows = read_csv(&base_dir.join("processes.csv"));
    let mut b_count = 0usize;
    let mut b_threads = 0u64;
    let mut b_handles = 0u64;
    let mut b_priv = 0.0f64;
    if !proc_rows.is_empty() {
        let ph = &proc_rows[0];
        let (pt, pn, pp) = (col(ph, "Threads"), col(ph, "Handles"), col(ph, "PrivateMB"));
        for r in proc_rows.iter().skip(1) {
            b_count += 1;
            if let Some(a) = pt {
                b_threads += r.get(a).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            }
            if let Some(a) = pn {
                b_handles += r.get(a).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            }
            if let Some(a) = pp {
                b_priv += r.get(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            }
        }
    }
    if b_priv == 0.0 {
        b_priv = base.values().map(|v| v.2).sum();
    }
    let sysj = fs::read_to_string(base_dir.join("system.json")).unwrap_or_default();
    let b_used = json_num(&sysj, "UsedPhysicalMemoryMB").unwrap_or(0.0);
    let b_load = json_num(&sysj, "MemoryLoadPercent").unwrap_or(0.0);
    let b_commit = json_num(&sysj, "CommittedMB").unwrap_or(0.0);

    println!();
    println!("================ 整机对比 ================");
    println!(
        "已用物理内存: {:.0} → {} MB  ({:+})",
        b_used,
        live.used_phys_mb,
        live.used_phys_mb as i64 - b_used as i64
    );
    println!(
        "内存占用率  : {:.1} → {} %  ({:+})",
        b_load,
        live.load_pct,
        live.load_pct as i64 - b_load as i64
    );
    println!(
        "已提交      : {:.0} → {} MB  ({:+})",
        b_commit,
        live.committed_mb,
        live.committed_mb as i64 - b_commit as i64
    );
    println!(
        "进程数      : {} → {}  ({:+})",
        b_count,
        live.procs.len(),
        live.procs.len() as i64 - b_count as i64
    );
    println!(
        "线程数      : {} → {}  ({:+})",
        b_threads,
        live_threads,
        live_threads as i64 - b_threads as i64
    );
    println!(
        "句柄数      : {} → {}  ({:+})",
        b_handles,
        live_handles,
        live_handles as i64 - b_handles as i64
    );
    println!(
        "专用内存合计: {:.0} → {:.0} MB  ({:+.0}, {:+.1}%)",
        b_priv,
        live_priv,
        live_priv - b_priv,
        if b_priv > 0.0 {
            (live_priv - b_priv) / b_priv * 100.0
        } else {
            0.0
        }
    );
    println!();

    let growth: Vec<&ProcDiff> = diffs.iter().filter(|d| d.delta > 0.0).take(12).collect();
    if !growth.is_empty() {
        println!("---- 增长最多（进程名聚合）----");
        for d in &growth {
            let pct = match d.pct {
                Some(v) => format!(" ({:.1}%)", v),
                None => String::new(),
            };
            println!(
                "  {:<28} {:>8.1} -> {:>8.1} MB   {:+.1}{}",
                d.name, d.base_ws, d.live_ws, d.delta, pct
            );
        }
        println!();
    }

    let newbies: Vec<&ProcDiff> = diffs.iter().filter(|d| d.status == "新增").collect();
    if !newbies.is_empty() {
        println!("---- 基准里没有、现在在跑的进程：{} 个 ----", newbies.len());
        for d in newbies.iter().take(8) {
            println!("  {:<28} {:>8.1} MB  {} 实例", d.name, d.live_ws, d.live_inst);
        }
        println!();
    }

    let mut svc_changes: Vec<(String, String, String)> = Vec::new();
    for s in &live.svcs {
        let b = base_svc.get(&s.name);
        let bstate = b.map(|x| x.as_str()).unwrap_or("");
        if bstate == s.state {
            continue;
        }
        let change = if bstate.is_empty() {
            "新增服务".to_string()
        } else {
            format!("{} -> {}", bstate, s.state)
        };
        svc_changes.push((s.name.clone(), change, s.start_mode.clone()));
    }
    if !svc_changes.is_empty() {
        println!("---- 服务状态变化：{} 项 ----", svc_changes.len());
        for (n, c, m) in svc_changes.iter().take(12) {
            println!("  {:<32} {:<22} 启动方式={}", n, c, m);
        }
        println!();
    }

    if !blind.is_empty() {
        println!(
            "---- 内存不可比：{} 项（有一侧没读到内存，多为未提权采集或受保护进程）----",
            blind.len()
        );
        for n in blind.iter().take(8) {
            println!("  {}", n);
        }
        if blind.len() > 8 {
            println!("  …… 另有 {} 项", blind.len() - 8);
        }
        println!();
    }

    println!("基准目录：{}", base_dir.display());
    Ok(())
}

fn json_num(s: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{}\"", key);
    let i = s.find(&pat)?;
    let rest = &s[i + pat.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| c == ',' || c == '\n' || c == '}')
        .unwrap_or(rest.len());
    rest[..end].trim().trim_matches('"').parse::<f64>().ok()
}

fn wildcard_match(s: &str, pat: &str) -> bool {    let s = s.to_ascii_lowercase();
    let p = pat.to_ascii_lowercase();
    if !p.contains('*') && !p.contains('?') {
        return s == p;
    }
    let sb: Vec<char> = s.chars().collect();
    let pb: Vec<char> = p.chars().collect();
    fn go(s: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        match p[0] {
            '*' => {
                if go(s, &p[1..]) {
                    return true;
                }
                if s.is_empty() {
                    return false;
                }
                go(&s[1..], p)
            }
            '?' => !s.is_empty() && go(&s[1..], &p[1..]),
            c => !s.is_empty() && s[0] == c && go(&s[1..], &p[1..]),
        }
    }
    go(&sb, &pb)
}
