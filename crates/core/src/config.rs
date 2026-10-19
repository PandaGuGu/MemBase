//! 数据目录配置：让用户自己决定基准线存哪儿，而不是永远塞在 ProgramData。
//!
//! 解析优先级（高 → 低）：
//!   1. 命令行 --out
//!   2. 环境变量 MEMBASE_ROOT
//!   3. 用户配置文件 %APPDATA%\MemBase\config.json
//!   4. %ProgramData%\MemBaseline（系统默认）
//!
//! 配置文件放 APPDATA 而不是数据目录里：数据目录本身可能被改，配置得先找得到。

use std::fs;
use std::path::{Path, PathBuf};

/// 用户配置（目前只有数据目录，以后可扩展）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub data_root: Option<String>,
}

pub fn serialize(cfg: &Config) -> String {
    let v = match &cfg.data_root {
        Some(p) => crate::bundle::jstr(p),
        None => "null".to_string(),
    };
    format!("{{\n  \"data_root\": {}\n}}\n", v)
}

pub fn parse(text: &str) -> Config {
    let mut out = Config::default();
    let key = "\"data_root\"";
    let i = match text.find(key) {
        Some(i) => i + key.len(),
        None => return out,
    };
    let rest = text[i..].trim_start();
    let rest = match rest.strip_prefix(':') {
        Some(r) => r.trim_start(),
        None => return out,
    };
    let Some(r2) = rest.strip_prefix('"') else {
        return out;
    };
    let mut s = String::new();
    let mut chars = r2.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('r') => s.push('\r'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(n) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(n) {
                            s.push(ch);
                        }
                    }
                }
                Some(other) => s.push(other),
                None => break,
            },
            c => s.push(c),
        }
    }
    if !s.trim().is_empty() {
        out.data_root = Some(s);
    }
    out
}

/// %APPDATA%\MemBase\config.json；MEMBASE_CONFIG 可整体重定向（测试/便携模式用）
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("MEMBASE_CONFIG") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata = appdata.trim();
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join("MemBase").join("config.json");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("MemBase")
            .join("config.json");
    }
    PathBuf::from("membase-config.json")
}

pub fn read_from(path: &Path) -> Config {
    fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn write_to(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serialize(cfg))
}

pub fn load() -> Config {
    read_from(&config_path())
}

pub fn save(cfg: &Config) -> std::io::Result<PathBuf> {
    let p = config_path();
    write_to(&p, cfg)?;
    Ok(p)
}

/// 最终生效的数据目录
pub fn data_root(cli: Option<&str>) -> String {
    if let Some(v) = cli {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    if let Ok(v) = std::env::var("MEMBASE_ROOT") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    if let Some(v) = load().data_root {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    crate::default_root()
}

/// 当前目录的来源说明，UI 和 CLI 都拿它显示
pub fn root_source(cli: Option<&str>) -> &'static str {
    if let Some(v) = cli {
        if !v.trim().is_empty() {
            return "--out";
        }
    }
    if let Ok(v) = std::env::var("MEMBASE_ROOT") {
        if !v.trim().is_empty() {
            return "MEMBASE_ROOT";
        }
    }
    let from_cfg = load().data_root.map(|s| !s.trim().is_empty()).unwrap_or(false);
    if from_cfg {
        return "配置文件";
    }
    "系统默认"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_unicode_path() {
        let cfg = Config {
            data_root: Some(r"D:\内存 基准\数据".to_string()),
        };
        let text = serialize(&cfg);
        assert!(text.is_ascii(), "配置里的中文应该被转义成 \\u");
        assert_eq!(parse(&text), cfg);
    }

    #[test]
    fn parse_null_or_missing_is_none() {
        assert_eq!(parse("{\"data_root\": null}").data_root, None);
        assert_eq!(parse("{}").data_root, None);
        assert_eq!(parse("").data_root, None);
    }

    #[test]
    fn parse_tolerates_spacing() {
        let c = parse("{ \"data_root\" :  \"C:\\\\tmp\\\\x\" }");
        assert_eq!(c.data_root.as_deref(), Some(r"C:\tmp\x"));
    }

    #[test]
    fn file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mbcfg_{}", std::process::id()));
        let p = dir.join("config.json");
        let cfg = Config {
            data_root: Some(r"E:\Base".into()),
        };
        write_to(&p, &cfg).unwrap();
        assert_eq!(read_from(&p), cfg);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_arg_wins_over_everything() {
        // 只要显式传了 --out，就一定用它
        assert_eq!(data_root(Some(r"Z:\explicit")), r"Z:\explicit");
        assert_eq!(root_source(Some(r"Z:\explicit")), "--out");
    }
}
