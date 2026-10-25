use percent_encoding::percent_decode_str;
use std::path::Path;

/// 从 Content-Disposition 中解析文件名，优先 RFC 5987 的 filename*
pub fn filename_from_disposition(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.split(';').map(|p| p.trim()).collect();
    for p in &parts {
        if let Some(rest) = strip_prefix_ci(p, "filename*=") {
            let rest = rest.trim_matches('"');
            let encoded = rest.splitn(3, '\'').nth(2).unwrap_or(rest);
            let name = percent_decode_str(encoded).decode_utf8_lossy().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    for p in &parts {
        if let Some(rest) = strip_prefix_ci(p, "filename=") {
            let raw = rest.trim_matches('"');
            let name = percent_decode_str(raw).decode_utf8_lossy().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

pub fn filename_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let last = parsed.path_segments()?.rfind(|s| !s.is_empty())?;
    let name = percent_decode_str(last).decode_utf8_lossy().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// 清理 Windows 文件名中的非法字符
pub fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    if out.chars().count() > 200 {
        out = out.chars().take(200).collect();
    }
    if out.is_empty() {
        "download".into()
    } else {
        out
    }
}

/// 如果文件已存在（或被其他任务占用），生成 `name (1).ext` 形式的新名字
pub fn unique_name(dir: &Path, name: &str, taken: impl Fn(&str) -> bool) -> String {
    let exists =
        |n: &str| taken(n) || dir.join(n).exists() || dir.join(format!("{n}.part")).exists();
    if !exists(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for i in 1..10000 {
        let candidate = format!("{stem} ({i}){ext}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    format!("{stem}-{}{ext}", crate::model::now_ms())
}

/// 跨平台的定位写入
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut written = 0usize;
        while written < buf.len() {
            let n = file.seek_write(&buf[written..], offset + written as u64)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "写入失败",
                ));
            }
            written += n;
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
}

/// 查找最近存在的祖先目录，用于查询磁盘空间
pub fn existing_ancestor(path: &Path) -> Option<&Path> {
    let mut p = Some(path);
    while let Some(cur) = p {
        if cur.exists() {
            return Some(cur);
        }
        p = cur.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"a b.zip\"").as_deref(),
            Some("a b.zip")
        );
        assert_eq!(
            filename_from_disposition(
                "attachment; filename=x.zip; filename*=UTF-8''%E4%B8%AD%E6%96%87.zip"
            )
            .as_deref(),
            Some("中文.zip")
        );
    }

    #[test]
    fn url_name() {
        assert_eq!(
            filename_from_url("https://a.com/dir/%E6%B5%8B%E8%AF%95.bin?x=1").as_deref(),
            Some("测试.bin")
        );
        assert_eq!(filename_from_url("https://a.com/"), None);
    }

    #[test]
    fn sanitize_name() {
        assert_eq!(sanitize("a:b?.txt. "), "a_b_.txt");
        assert_eq!(sanitize(""), "download");
    }
}
