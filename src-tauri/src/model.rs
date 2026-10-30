use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Waiting,
    Active,
    Paused,
    Completed,
    Error,
}

/// 一个下载分段，`end` 为包含边界，`pos` 为下一个待写入的字节
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub url: String,
    pub file_name: String,
    /// 文件名已确定（用户指定或已探测），后续不再改名
    #[serde(default)]
    pub name_locked: bool,
    pub save_dir: String,
    pub total: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    #[serde(default)]
    pub scheduled_at: Option<u64>,
    #[serde(default)]
    pub resumable: bool,
    /// 服务器不支持分段时强制单连接
    #[serde(default)]
    pub force_single: bool,
    #[serde(default)]
    pub hls: bool,
    #[serde(default)]
    pub torrent: Option<crate::torrent::Source>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub connections: u32,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub error: Option<String>,
    pub mime: Option<String>,
    #[serde(default)]
    pub http_identity: HttpIdentity,
    /// 附加请求头（浏览器扩展传入的 Cookie、Referer、User-Agent），续传时继续使用
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    // 以下为运行时字段
    #[serde(default)]
    pub speed: u64,
    #[serde(default)]
    pub active_conns: u32,
    /// 当前超频连接数
    #[serde(default)]
    pub boost: u32,
    /// 请求延迟（毫秒），0 表示未测得
    #[serde(default)]
    pub latency_ms: u32,
    #[serde(default)]
    pub resume_after_update: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct HttpIdentity {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl HttpIdentity {
    fn strong_etag(&self) -> Option<&str> {
        self.etag.as_deref().filter(|v| {
            v.len() >= 2
                && v.starts_with('"')
                && v.ends_with('"')
                && v.as_bytes()[1..v.len() - 1]
                    .iter()
                    .all(|b| *b == 0x21 || (0x23..=0x7e).contains(b))
        })
    }

    pub fn validator(&self) -> Option<&str> {
        self.strong_etag().or(self.last_modified.as_deref())
    }

    pub fn matches(&self, other: &Self) -> bool {
        if let Some(etag) = self.strong_etag() {
            other.etag.as_deref() == Some(etag)
        } else if let Some(modified) = &self.last_modified {
            other.last_modified.as_ref() == Some(modified)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::HttpIdentity;

    #[test]
    fn resume_requires_strong_etag_or_matching_last_modified() {
        let identity = |etag: &str, modified: Option<&str>| HttpIdentity {
            etag: Some(etag.into()),
            last_modified: modified.map(str::to_owned),
        };
        let date = Some("Fri, 30 Oct 2026 10:00:00 GMT");
        let strong = identity("\"v1\"", date);
        assert_eq!(strong.validator(), Some("\"v1\""));
        assert!(!strong.matches(&identity("\"v2\"", date)));
        assert!(strong.matches(&identity("\"v1\"", None)));
        for invalid in ["", "v1", "W/\"v1\"", "\"bad\"etag\"", "\"bad\netag\""] {
            assert!(identity(invalid, None).validator().is_none());
            assert!(!identity(invalid, None).matches(&identity(invalid, None)));
            assert_eq!(identity(invalid, date).validator(), date);
            assert!(identity(invalid, date).matches(&identity("W/\"v2\"", date)));
        }
        assert!(!HttpIdentity::default().matches(&HttpIdentity::default()));
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub capture_on_top: bool,
    pub task_view: TaskView,
    pub download_window: crate::schedule::DownloadWindow,
    pub download_dir: String,
    pub max_concurrent: u32,
    pub connections: u32,
    /// KB/s，0 表示不限速
    pub speed_limit: u64,
    pub user_agent: String,
    pub notify: bool,
    pub clipboard_watch: bool,
    pub close_to_tray: bool,
    pub resume_on_start: bool,
    /// acrylic | mica | none
    pub glass: String,
    /// 界面风格：default（深色中性灰）| md3（Material Design 3）
    pub theme: String,
    /// 启用浏览器扩展接入（本地 HTTP 服务）
    pub browser_integration: bool,
    /// 拦截到下载时先弹出确认框；关闭则直接开始
    pub capture_confirm: bool,
    /// system | none | custom
    pub proxy_mode: String,
    /// 自定义代理地址，如 http://127.0.0.1:7890、socks5://user:pass@host:1080
    pub proxy_url: String,
    /// 连接超频：按测得的请求延迟提前补连接，最多加开每任务连接数的一半
    pub overclock: bool,
    pub bt: crate::bt_config::BtSettings,
    pub auto_update: bool,
    pub update_mirrors: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_on_top: false,
            task_view: Default::default(),
            download_window: Default::default(),
            download_dir: String::new(),
            max_concurrent: 3,
            connections: 8,
            speed_limit: 0,
            user_agent: String::new(),
            notify: true,
            clipboard_watch: true,
            close_to_tray: true,
            resume_on_start: true,
            glass: "acrylic".into(),
            theme: "default".into(),
            browser_integration: true,
            capture_confirm: true,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            overclock: true,
            bt: Default::default(),
            auto_update: true,
            update_mirrors: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskView {
    pub layout: String,
    pub sort: String,
    pub descending: bool,
    pub group: String,
    pub show_percentage: bool,
}
impl Default for TaskView {
    fn default() -> Self {
        Self {
            layout: "cards".into(),
            sort: "created".into(),
            descending: true,
            group: "none".into(),
            show_percentage: true,
        }
    }
}
impl TaskView {
    pub fn validate(&self) -> Result<(), String> {
        if !["cards", "list"].contains(&self.layout.as_str())
            || !["created", "name", "type", "size"].contains(&self.sort.as_str())
            || !["none", "date", "type", "status"].contains(&self.group.as_str())
        {
            return Err("下载视图配置无效".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: u64,
    pub active_conns: u32,
    pub boost: u32,
    pub latency_ms: u32,
    pub segments: Vec<Segment>,
}

#[derive(Serialize)]
pub struct DiskInfo {
    pub free: u64,
    pub total: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn old_settings_gain_safe_schedule_and_view_defaults() {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "downloadDir": "C:/Downloads", "captureConfirm": true
        }))
        .unwrap();
        assert!(!settings.capture_on_top);
        assert!(!settings.download_window.enabled);
        assert_eq!(settings.task_view.layout, "cards");
        assert!(settings.task_view.show_percentage);
        assert!(settings.task_view.validate().is_ok());
        assert!(settings.download_window.validate().is_ok());
        let mut view = settings.task_view;
        view.group = "unknown".into();
        assert!(view.validate().is_err());
    }
}
