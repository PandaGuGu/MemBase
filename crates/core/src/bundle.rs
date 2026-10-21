//! 基准线打包：导出/导入成单个文件（store 方式的 zip，零第三方依赖）。
//!
//! 设计：
//!   * 纯逻辑 + 文件 IO，不碰 Win32 → Linux 也能测
//!   * 包内固定包含 manifest.json，记录采集时间、文件数、整体 crc32，方便「检测」

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const REQUIRED_FILES: &[&str] = &[
    "system.json",
    "processes.csv",
    "processes_by_name.csv",
    "services.csv",
];
pub const OPTIONAL_FILES: &[&str] = &["summary.md"];

pub fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    o.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ---------------- crc32 ----------------

fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

pub fn crc32(data: &[u8]) -> u32 {
    let table = crc_table();
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc = table[((crc ^ (*b as u32)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------- DOS 时间 ----------------

fn dos_time_date(unix_secs: u64) -> (u16, u16) {
    // 1970-01-01 起的秒数 → 公历年月日时分秒
    let days = (unix_secs / 86400) as i64;
    let secs_of_day = unix_secs % 86400;
    let hour = (secs_of_day / 3600) as u16;
    let minute = ((secs_of_day % 3600) / 60) as u16;
    let second = (secs_of_day % 60) as u16;

    // Howard Hinnant 的 civil_from_days
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    let year16 = (year as u16).saturating_sub(1980);
    let date = (year16 << 9) | ((m as u16) << 5) | (d as u16);
    let time = (hour << 11) | (minute << 5) | (second / 2);
    (time, date)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------- zip 写入（store） ----------------

pub fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    let mut out: Vec<u8> = Vec::new();
    let (dtime, ddate) = dos_time_date(now_unix());
    let mut central: Vec<u8> = Vec::new();

    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;

        // local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 name
        out.extend_from_slice(&0u16.to_le_bytes()); // store
        out.extend_from_slice(&dtime.to_le_bytes());
        out.extend_from_slice(&ddate.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // central directory entry
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // needed
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&dtime.to_le_bytes());
        central.extend_from_slice(&ddate.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);

    // end of central directory
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(&out)?;
    Ok(())
}

// ---------------- zip 读取（store） ----------------

pub fn read_zip(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut buf = Vec::new();
    fs::File::open(path)
        .map_err(|e| format!("打不开包: {e}"))?
        .read_to_end(&mut buf)
        .map_err(|e| format!("读包失败: {e}"))?;

    if buf.len() < 22 {
        return Err("不是有效的 zip（太短）".into());
    }
    // 从尾部找 EOCD
    let mut eocd = None;
    let min = buf.len().saturating_sub(65557);
    let mut i = buf.len() - 22;
    loop {
        if u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) == 0x0605_4b50 {
            eocd = Some(i);
            break;
        }
        if i == min {
            break;
        }
        i -= 1;
    }
    let eocd = eocd.ok_or_else(|| "没找到 zip 目录尾（EOCD）".to_string())?;
    let count = u16::from_le_bytes([buf[eocd + 10], buf[eocd + 11]]) as usize;
    let cd_off = u32::from_le_bytes([
        buf[eocd + 16],
        buf[eocd + 17],
        buf[eocd + 18],
        buf[eocd + 19],
    ]) as usize;

    let mut out = Vec::new();
    let mut p = cd_off;
    for _ in 0..count {
        if p + 46 > buf.len() {
            return Err("zip 目录损坏".into());
        }
        if u32::from_le_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]]) != 0x0201_4b50 {
            return Err("zip 目录项签名不对".into());
        }
        let method = u16::from_le_bytes([buf[p + 10], buf[p + 11]]);
        let crc_expect = u32::from_le_bytes([buf[p + 16], buf[p + 17], buf[p + 18], buf[p + 19]]);
        let csize = u32::from_le_bytes([buf[p + 20], buf[p + 21], buf[p + 22], buf[p + 23]]) as usize;
        let name_len = u16::from_le_bytes([buf[p + 28], buf[p + 29]]) as usize;
        let extra_len = u16::from_le_bytes([buf[p + 30], buf[p + 31]]) as usize;
        let comment_len = u16::from_le_bytes([buf[p + 32], buf[p + 33]]) as usize;
        let local_off = u32::from_le_bytes([
            buf[p + 42],
            buf[p + 43],
            buf[p + 44],
            buf[p + 45],
        ]) as usize;
        let name = String::from_utf8_lossy(&buf[p + 46..p + 46 + name_len]).to_string();
        if method != 0 {
            return Err(format!(
                "包内 {} 用了压缩方式 {}（只支持 store，请用 membase baseline export 生成的包）",
                name, method
            ));
        }
        // local header
        if local_off + 30 > buf.len() {
            return Err("zip 局部头越界".into());
        }
        let l_name_len = u16::from_le_bytes([buf[local_off + 26], buf[local_off + 27]]) as usize;
        let l_extra_len = u16::from_le_bytes([buf[local_off + 28], buf[local_off + 29]]) as usize;
        let data_start = local_off + 30 + l_name_len + l_extra_len;
        if data_start + csize > buf.len() {
            return Err("zip 数据越界".into());
        }
        let data = buf[data_start..data_start + csize].to_vec();
        let actual = crc32(&data);
        if actual != crc_expect {
            return Err(format!("包内 {} 校验失败（crc32 不符）", name));
        }
        out.push((name, data));
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

// ---------------- 基准线导出 / 导入 ----------------

#[derive(Debug)]
pub struct SnapshotInfo {
    pub dir: PathBuf,
    pub files: Vec<(String, u64)>,
    pub total_bytes: u64,
    pub crc: u32,
}

/// 读一个快照目录，算出文件清单和整体 crc32（「检测」用）
pub fn inspect_snapshot(dir: &Path) -> Result<SnapshotInfo, String> {
    if !dir.is_dir() {
        return Err(format!("不是目录: {}", dir.display()));
    }
    for need in REQUIRED_FILES {
        if !dir.join(need).exists() {
            return Err(format!("缺少必需文件: {}", need));
        }
    }
    let mut files: Vec<(String, u64)> = Vec::new();
    let mut all: Vec<u8> = Vec::new();
    let mut names: Vec<String> = REQUIRED_FILES.iter().map(|s| s.to_string()).collect();
    for o in OPTIONAL_FILES {
        if dir.join(o).exists() {
            names.push(o.to_string());
        }
    }
    names.sort();
    for n in names {
        let p = dir.join(&n);
        let data = fs::read(&p).map_err(|e| format!("读 {} 失败: {e}", n))?;
        files.push((n.clone(), data.len() as u64));
        all.extend_from_slice(&data);
    }
    let total = files.iter().map(|f| f.1).sum();
    Ok(SnapshotInfo {
        dir: dir.to_path_buf(),
        files,
        total_bytes: total,
        crc: crc32(&all),
    })
}

pub struct ExportResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: usize,
    pub crc: u32,
}

pub fn export_baseline(snap_dir: &Path, out_file: &Path) -> Result<ExportResult, String> {
    let info = inspect_snapshot(snap_dir)?;
    let captured = read_meta(snap_dir, "CapturedAtLocal").unwrap_or_default();
    let uptime = read_meta(snap_dir, "UptimeSeconds").unwrap_or_else(|| "0".into());

    let manifest = format!(
        "{{\"generator\":\"membase {}\",\"build\":{},\"format\":1,\"captured_at\":{},\"uptime_sec\":{},\"files\":{},\"bytes\":{},\"crc32\":\"{:08x}\"}}\n",
        env!("CARGO_PKG_VERSION"),
        jstr(&crate::build_id()),
        jstr(&captured),
        uptime.trim(),
        info.files.len(),
        info.total_bytes,
        info.crc
    );

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("manifest.json".to_string(), manifest.into_bytes()));
    for (name, _) in &info.files {
        let data = fs::read(snap_dir.join(name)).map_err(|e| format!("读 {name} 失败: {e}"))?;
        entries.push((name.clone(), data));
    }

    write_zip(out_file, &entries).map_err(|e| format!("写包失败: {e}"))?;
    let bytes = fs::metadata(out_file).map(|m| m.len()).unwrap_or(0);
    Ok(ExportResult {
        path: out_file.to_path_buf(),
        bytes,
        files: info.files.len(),
        crc: info.crc,
    })
}

pub struct ImportResult {
    pub dir: PathBuf,
    pub files: usize,
    pub crc: u32,
    pub captured: String,
}

/// 把包导入成一份新的基准快照目录（名字以 run_ 开头，才能被基准定位逻辑选中）
pub fn import_baseline(root: &Path, package: &Path, label: &str) -> Result<ImportResult, String> {
    let entries = read_zip(package)?;
    if entries.is_empty() {
        return Err("包是空的".into());
    }
    let has = |n: &str| entries.iter().any(|(k, _)| k == n);
    for need in REQUIRED_FILES {
        if !has(need) {
            return Err(format!("包里缺少必需文件: {}", need));
        }
    }

    // 目录名用包里记录的采集时间，比「导入时刻」更有意义；重名时加序号
    let captured_in_pkg = entries
        .iter()
        .find(|(k, _)| k == "system.json")
        .and_then(|(_, d)| meta_from_json(&String::from_utf8_lossy(d), "CapturedAtLocal"))
        .map(|s| stamp_from_local(&s))
        .unwrap_or_default();
    let stamp = if captured_in_pkg.is_empty() {
        chrono_like_stamp()
    } else {
        captured_in_pkg
    };

    let safe_label: String = label
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let label = if safe_label.is_empty() {
        "imported".to_string()
    } else {
        safe_label
    };

    let mut run_dir = root.join(format!("run_{}_import", stamp));
    let mut n = 2;
    while run_dir.exists() {
        run_dir = root.join(format!("run_{}_import{}", stamp, n));
        n += 1;
    }
    let dir = run_dir.join(&label);
    fs::create_dir_all(&dir).map_err(|e| format!("建目录失败: {e}"))?;

    let mut names: Vec<String> = Vec::new();
    let mut all: Vec<u8> = Vec::new();
    for (name, data) in &entries {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(format!("包内文件名不合法: {}", name));
        }
        fs::write(dir.join(name), data).map_err(|e| format!("写 {} 失败: {e}", name))?;
        if name != "manifest.json" {
            names.push(name.clone());
            all.extend_from_slice(data);
        }
    }
    let crc = crc32(&all);
    let captured = read_meta(&dir, "CapturedAtLocal").unwrap_or_else(|| "未知".into());
    Ok(ImportResult {
        dir,
        files: names.len(),
        crc,
        captured,
    })
}

/// 只做校验：包能不能读、必需文件齐不齐、crc 对不对
pub fn verify_package(package: &Path) -> Result<(usize, u32, String), String> {
    let entries = read_zip(package)?;
    let has = |n: &str| entries.iter().any(|(k, _)| k == n);
    for need in REQUIRED_FILES {
        if !has(need) {
            return Err(format!("包里缺少必需文件: {}", need));
        }
    }
    let mut all: Vec<u8> = Vec::new();
    let mut n = 0;
    for (name, data) in &entries {
        if name == "manifest.json" {
            continue;
        }
        n += 1;
        all.extend_from_slice(data);
    }
    let crc = crc32(&all);
    let captured = entries
        .iter()
        .find(|(k, _)| k == "system.json")
        .and_then(|(_, d)| meta_from_json(&String::from_utf8_lossy(d), "CapturedAtLocal"))
        .unwrap_or_else(|| "未知".into());
    Ok((n, crc, captured))
}

/// 找「当前生效」的基准快照：root 下 run_*\*\system.json 里 mtime 最新的那个。
/// 纯文件判断，所以 Linux 上也能用（便于测试）。
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

pub fn snapshot_meta(dir: &Path, key: &str) -> Option<String> {
    read_meta(dir, key)
}

/// 形如 20261019_215600
pub fn timestamp() -> String {
    chrono_like_stamp()
}

/// 把 system.json 里的 "2026-10-18 14:47:14" 变成 "20261018_144714"。
/// 导出包用它命名：文件名反映「基准是什么时候采的」，而不是「什么时候导出的」。
pub fn stamp_from_local(s: &str) -> String {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 14 {
        return String::new();
    }
    format!("{}_{}", &digits[0..8], &digits[8..14])
}

fn chrono_like_stamp() -> String {    // 用 system.json 里已有的时间更方便，但导入时可能没有；这里用文件系统时间
    let (t, d) = dos_time_date(now_unix());
    let year = 1980 + ((d >> 9) & 0x7F);
    let month = (d >> 5) & 0x0F;
    let day = d & 0x1F;
    let hour = (t >> 11) & 0x1F;
    let minute = (t >> 5) & 0x3F;
    let second = (t & 0x1F) * 2;
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        year, month, day, hour, minute, second
    )
}

fn meta_from_json(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let i = text.find(&pat)?;
    let rest = text[i + pat.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        let q = r.find('"')?;
        return Some(r[..q].to_string());
    }
    // 数字 / 布尔 / null
    let end = rest
        .find(|c: char| c == ',' || c == '}' || c == '\n')
        .unwrap_or(rest.len());
    let v = rest[..end].trim();
    if v.is_empty() || v == "null" {
        None
    } else {
        Some(v.to_string())
    }
}

fn read_meta(dir: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join("system.json")).ok()?;
    meta_from_json(&text, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        // "123456789" 的标准 CRC32 是 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn zip_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mbzip_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("t.mbz");
        let entries = vec![
            ("a.txt".to_string(), b"hello".to_vec()),
            ("sub/b.bin".to_string(), vec![0u8, 1, 2, 255]),
        ];
        write_zip(&path, &entries).unwrap();
        let back = read_zip(&path).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].0, "a.txt");
        assert_eq!(back[0].1, b"hello");
        assert_eq!(back[1].1, vec![0u8, 1, 2, 255]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_import_roundtrip() {
        let base = std::env::temp_dir().join(format!("mbimp_{}", std::process::id()));
        let src = base.join("snap");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("system.json"),
            "{\"CapturedAtLocal\":\"2026-10-19 21:00:00\",\"UptimeSeconds\":60}",
        )
        .unwrap();
        fs::write(src.join("processes.csv"), "a,b\n1,2\n").unwrap();
        fs::write(src.join("processes_by_name.csv"), "ProcessName,Instances\nx,1\n").unwrap();
        fs::write(src.join("services.csv"), "ServiceName,State\ns,Running\n").unwrap();
        fs::write(src.join("summary.md"), "# hi\n").unwrap();

        let info = inspect_snapshot(&src).unwrap();
        assert_eq!(info.files.len(), 5);

        let pkg = base.join("pkg.mbz");
        let r = export_baseline(&src, &pkg).unwrap();
        assert!(r.bytes > 0);

        let (n, _, captured) = verify_package(&pkg).unwrap();
        assert_eq!(n, 5);
        assert_eq!(captured, "2026-10-19 21:00:00");

        let dst_root = base.join("root");
        fs::create_dir_all(&dst_root).unwrap();
        let imp = import_baseline(&dst_root, &pkg, "test").unwrap();
        assert!(imp.dir.exists());
        assert!(imp.dir.to_string_lossy().contains("run_"));
        assert_eq!(imp.captured, "2026-10-19 21:00:00");
        assert!(imp.dir.join("processes.csv").exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn inspect_rejects_missing_files() {
        let dir = std::env::temp_dir().join(format!("mbmiss_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("system.json"), "{}").unwrap();
        let err = inspect_snapshot(&dir).unwrap_err();
        assert!(err.contains("缺少必需文件"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_escape_makes_ascii() {
        let s = jstr("开机·测试\"x\"");
        assert!(s.is_ascii());
        assert!(s.contains("\\u5f00"));
    }

    #[test]
    fn meta_reads_numbers_and_strings() {
        let text = "{\"CapturedAtLocal\":\"2026-01-01 00:00:00\",\"UptimeSeconds\":60,\"Tail\":\"x\"}";
        assert_eq!(
            meta_from_json(text, "CapturedAtLocal").as_deref(),
            Some("2026-01-01 00:00:00")
        );
        assert_eq!(meta_from_json(text, "UptimeSeconds").as_deref(), Some("60"));
        assert_eq!(meta_from_json(text, "Tail").as_deref(), Some("x"));
    }
}
