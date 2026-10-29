use parking_lot::RwLock;
use std::sync::OnceLock;

fn browser() -> &'static RwLock<String> {
    static UA: OnceLock<RwLock<String>> = OnceLock::new();
    UA.get_or_init(|| RwLock::new(format!("NDM/{}", env!("CARGO_PKG_VERSION"))))
}

pub fn effective(custom: &str) -> String {
    if custom.trim().is_empty() {
        browser().read().clone()
    } else {
        custom.to_string()
    }
}

pub fn set_browser(value: String) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 2048
        || reqwest::header::HeaderValue::from_str(&value).is_err()
    {
        return Err("浏览器 User-Agent 无效".into());
    }
    *browser().write() = value;
    Ok(())
}

pub fn migrate_legacy(value: &mut String) {
    if value == "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36" {
        value.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn follows_browser_without_overwriting_custom_values() {
        set_browser("NDM-UA-test/1".into()).unwrap();
        assert_eq!(effective(""), "NDM-UA-test/1");
        assert_eq!(effective("custom/9"), "custom/9");
        assert!(set_browser("bad\r\nheader".into()).is_err());
        let mut custom = "MyBrowser/123".into();
        migrate_legacy(&mut custom);
        assert_eq!(custom, "MyBrowser/123");
    }
}
