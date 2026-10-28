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
    pub resumable: bool,
    /// 服务器不支持分段时强制单连接
    #[serde(default)]
    pub force_single: bool,
    #[serde(default)]
    pub hls: bool,
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub connections: u32,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub error: Option<String>,
    pub mime: Option<String>,
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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            max_concurrent: 3,
            connections: 8,
            speed_limit: 0,
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36".into(),
            notify: true,
            clipboard_watch: true,
            close_to_tray: true,
            resume_on_start: true,
            glass: "acrylic".into(),
            browser_integration: true,
            capture_confirm: true,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            overclock: true,
        }
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
