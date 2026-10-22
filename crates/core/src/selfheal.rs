//! 自愈型进程的记录（信息层，**不做探测**）。
//!
//! 有些进程杀掉会被外壳秒拉起（`SearchHost` / `explorer` 一类），对它们做「重启回收」
//! 是打地鼠：净收益 0，还可能丢掉用户正在用的界面。
//!
//! `restart` 已经能识别一次（返回 `revived`），但那个结论只活在当次输出里，
//! 下次 `list` 又看不出来。这里把它落成一条只读信息，`status` / `list` 直接标出。
//!
//! 故意**不**做自动探测：探测本身就要杀进程，那正是要避免的打地鼠。
//! 只有用户真的跑过一次 `restart` 并观察到自愈，才会有这条记录。

use std::collections::BTreeMap;

/// 缓存文件里最多留多少条：这只是「别指望重启能回收」的提示，不需要历史。
pub const MAX_ENTRIES: usize = 64;

/// 解析：一行一条 `进程名<TAB>时间`，`#` 开头是注释，同名后者覆盖前者。
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (name, at) = match line.split_once('\t') {
            Some((n, a)) => (n.trim(), a.trim()),
            None => (line.trim(), ""),
        };
        if name.is_empty() {
            continue;
        }
        out.insert(name.to_string(), at.to_string());
    }
    out
}

/// 序列化：按名字排序，附固定注释头（和 `protect.txt` 一个风格）
pub fn serialize(map: &BTreeMap<String, String>) -> String {
    let mut s = String::from(
        "# 自愈型进程：杀掉会被系统/外壳重新拉起，别指望用 restart 回收\n\
         # 由 `MemBase.exe restart` 实测后自动记录，一行一条 `进程名<TAB>时间`\n",
    );
    for (name, at) in map {
        s.push_str(name);
        s.push('\t');
        s.push_str(at);
        s.push('\n');
    }
    s
}

/// 记一条结论；`keep` 是保留上限，超出时丢最早的（按时间字符串排序，空时间最旧）。
pub fn note(text: &str, name: &str, at: &str, keep: usize) -> String {
    let mut map = parse(text);
    map.insert(name.to_string(), at.to_string());
    if map.len() > keep.max(1) {
        let mut entries: Vec<(String, String)> = map.into_iter().collect();
        // 时间升序，空字符串排最前 —— 没时间的先丢
        entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        let drop_n = entries.len() - keep.max(1);
        let kept: BTreeMap<String, String> = entries.into_iter().skip(drop_n).collect();
        return serialize(&kept);
    }
    serialize(&map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析与序列化往返() {
        let text = "# 注释\nSearchHost\t2026-10-21 22:00\ninvalid-line\n";
        let m = parse(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("SearchHost").map(|s| s.as_str()), Some("2026-10-21 22:00"));
        assert_eq!(m.get("invalid-line").map(|s| s.as_str()), Some(""));

        let out = serialize(&m);
        assert!(out.starts_with('#'));
        assert!(parse(&out).contains_key("SearchHost"));
    }

    #[test]
    fn 记录会覆盖同名并限制条数() {
        let mut text = String::new();
        text = note(&text, "SearchHost", "2026-10-21 22:00", 3);
        text = note(&text, "SearchHost", "2026-10-22 07:00", 3);
        let m = parse(&text);
        assert_eq!(m.len(), 1);
        assert_eq!(m["SearchHost"], "2026-10-22 07:00");

        // 超过上限时，时间最早的先被丢掉
        text = note(&text, "A", "2026-10-20 01:00", 2);
        text = note(&text, "B", "2026-10-22 09:00", 2);
        let m = parse(&text);
        assert_eq!(m.len(), 2);
        assert!(!m.contains_key("A"), "最早的那条应该被丢掉：{:?}", m);
        assert!(m.contains_key("SearchHost") && m.contains_key("B"));
    }
}
