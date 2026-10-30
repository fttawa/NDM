//! 浏览器扩展桥接：本地 HTTP 服务（127.0.0.1:16800），接收扩展转交的下载
//!
//! 安全：只接受来自浏览器扩展（Origin 为扩展协议）的请求，
//! 写入接口还需要启动期随机 token，普通网页无法向 NDM 添加任务。

use crate::manager::{AddOptions, Manager};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager as _;
use tiny_http::{Header, Method, Request, Response, Server};

pub const PORT: u16 = 16800;

/// 监听端口；NDM_BRIDGE_PORT 可覆盖（开发测试实例使用，避免与正在运行的 NDM 冲突）
fn port() -> u16 {
    std::env::var("NDM_BRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(PORT)
}
const MAX_BODY: u64 = 1024 * 1024;

/// 端口被占用时的重试间隔（如升级安装时旧进程尚未退出）
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

pub struct Bridge {
    server: Mutex<Option<Arc<Server>>>,
    error: Mutex<Option<String>>,
    /// 是否应当运行（关闭浏览器集成时为 false，重试线程据此退出）
    wanted: AtomicBool,
    /// 是否有监听/重试线程在运行，避免重复启动
    running: AtomicBool,
    token: String,
}

impl Default for Bridge {
    fn default() -> Self {
        Self {
            server: Mutex::new(None),
            error: Mutex::new(None),
            wanted: AtomicBool::new(false),
            running: AtomicBool::new(false),
            token: uuid::Uuid::new_v4().to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
}

/// 扩展发来的下载请求
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capture {
    url: String,
    filename: Option<String>,
    referrer: Option<String>,
    cookies: Option<String>,
    user_agent: Option<String>,
    file_size: Option<i64>,
    #[serde(default)]
    hls: bool,
}

/// 扩展发来的批量下载（如 OpenList 当前文件夹的文件清单）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    /// 来源，如 openlist
    source: Option<String>,
    /// 显示用标题（文件夹名）
    title: Option<String>,
    referrer: Option<String>,
    items: Vec<BatchItem>,
    /// 文件过多被截断
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BatchItem {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    /// 相对当前文件夹的子目录（以 / 分隔，空表示当前目录）
    #[serde(default)]
    pub dir: String,
}

/// 推送给批量下载卡片的数据
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BatchPayload {
    pub source: String,
    pub title: String,
    pub referrer: Option<String>,
    pub headers: Vec<(String, String)>,
    pub items: Vec<BatchItem>,
    pub truncated: bool,
}

const MAX_BATCH: usize = 5000;

/// 推送给前端确认框的数据
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CapturePayload {
    pub url: String,
    pub file_name: Option<String>,
    pub headers: Vec<(String, String)>,
    pub file_size: Option<u64>,
    pub referrer: Option<String>,
    pub hls: bool,
}

impl Bridge {
    /// 启动监听；端口被占用时在后台每隔几秒重试，直到成功或被关闭
    pub fn start(self: &Arc<Self>, mgr: Arc<Manager>) {
        self.wanted.store(true, Ordering::Release);
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("ndm-bridge".into())
            .spawn(move || {
                while me.wanted.load(Ordering::Acquire) {
                    match Server::http(("127.0.0.1", port())) {
                        Ok(server) => {
                            let server = Arc::new(server);
                            *me.server.lock() = Some(server.clone());
                            *me.error.lock() = None;
                            // stop() 调用 unblock() 后迭代结束，释放端口；
                            // 若期间又被重新开启（关闭后立刻打开），回到循环重新监听
                            for req in server.incoming_requests() {
                                handle(&mgr, req);
                            }
                        }
                        Err(e) => {
                            *me.error.lock() =
                                Some(format!("端口 {} 被占用，正在重试…（{e}）", port()));
                            std::thread::sleep(RETRY_INTERVAL);
                        }
                    }
                }
                me.running.store(false, Ordering::Release);
            })
            .ok();
    }

    pub fn stop(&self) {
        self.wanted.store(false, Ordering::Release);
        *self.error.lock() = None;
        if let Some(server) = self.server.lock().take() {
            server.unblock();
        }
    }

    pub fn status(&self) -> BridgeStatus {
        BridgeStatus {
            running: self.server.lock().is_some(),
            port: port(),
            error: self.error.lock().clone(),
        }
    }
}

fn is_allowed_origin(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => [
            "chrome-extension://",
            "moz-extension://",
            "extension://",
            "safari-web-extension://",
        ]
        .iter()
        .any(|p| o.starts_with(p)),
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("合法的响应头")
}

fn respond(req: Request, status: u16, body: serde_json::Value, origin: Option<&str>) {
    let mut resp = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"));
    if let Some(o) = origin {
        resp = resp
            .with_header(header("Access-Control-Allow-Origin", o))
            .with_header(header("Vary", "Origin"));
    }
    let _ = req.respond(resp);
}

fn handle(mgr: &Arc<Manager>, mut req: Request) {
    let origin = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    let origin = origin.as_deref();

    if !is_allowed_origin(origin) {
        return respond(req, 403, serde_json::json!({ "error": "forbidden" }), None);
    }
    let path = req.url().split('?').next().unwrap_or("").to_string();

    match (req.method(), path.as_str()) {
        (Method::Options, _) => {
            let mut resp = Response::empty(204)
                .with_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
                .with_header(header(
                    "Access-Control-Allow-Headers",
                    "Content-Type, X-NDM-Token",
                ))
                .with_header(header("Access-Control-Allow-Private-Network", "true"))
                .with_header(header("Access-Control-Max-Age", "600"));
            if let Some(o) = origin {
                resp = resp.with_header(header("Access-Control-Allow-Origin", o));
            }
            let _ = req.respond(resp);
        }
        (Method::Get, "/ping") => {
            let s = mgr.settings();
            respond(
                req,
                200,
                serde_json::json!({ "name": "NDM", "version": env!("CARGO_PKG_VERSION"), "confirm": s.capture_confirm, "token": mgr.app.state::<Arc<crate::bridge::Bridge>>().token }),
                origin,
            );
        }
        (Method::Post, "/batch") => {
            if !authorized(&req, mgr) {
                return respond(req, 401, serde_json::json!({ "error": "未授权" }), origin);
            }
            let mut body = String::new();
            if req
                .as_reader()
                .take(MAX_BODY * 16)
                .read_to_string(&mut body)
                .is_err()
            {
                return respond(
                    req,
                    400,
                    serde_json::json!({ "error": "请求体无效" }),
                    origin,
                );
            }
            match serde_json::from_str::<Batch>(&body) {
                Ok(batch) => match accept_batch(mgr, batch) {
                    Ok(n) => respond(
                        req,
                        200,
                        serde_json::json!({ "ok": true, "count": n }),
                        origin,
                    ),
                    Err(e) => respond(req, 400, serde_json::json!({ "error": e }), origin),
                },
                Err(e) => respond(
                    req,
                    400,
                    serde_json::json!({ "error": format!("JSON 无效：{e}") }),
                    origin,
                ),
            }
        }
        (Method::Post, "/add") => {
            if !authorized(&req, mgr) {
                return respond(req, 401, serde_json::json!({ "error": "未授权" }), origin);
            }
            let mut body = String::new();
            if req
                .as_reader()
                .take(MAX_BODY)
                .read_to_string(&mut body)
                .is_err()
            {
                return respond(
                    req,
                    400,
                    serde_json::json!({ "error": "请求体无效" }),
                    origin,
                );
            }
            match serde_json::from_str::<Capture>(&body) {
                Ok(cap) => match accept(mgr, cap) {
                    Ok(()) => respond(req, 200, serde_json::json!({ "ok": true }), origin),
                    Err(e) => respond(req, 400, serde_json::json!({ "error": e }), origin),
                },
                Err(e) => respond(
                    req,
                    400,
                    serde_json::json!({ "error": format!("JSON 无效：{e}") }),
                    origin,
                ),
            }
        }
        _ => respond(
            req,
            404,
            serde_json::json!({ "error": "not found" }),
            origin,
        ),
    }
}

fn authorized(req: &Request, mgr: &Arc<Manager>) -> bool {
    req.headers()
        .iter()
        .find(|h| h.field.equiv("X-NDM-Token"))
        .map(|h| h.value.as_str() == mgr.app.state::<Arc<Bridge>>().token)
        .unwrap_or(false)
}

fn accept(mgr: &Arc<Manager>, cap: Capture) -> Result<(), String> {
    let parsed = reqwest::Url::parse(cap.url.trim()).map_err(|_| "链接无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只支持 HTTP/HTTPS 链接".into());
    }
    let nonempty = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let referrer = nonempty(cap.referrer);
    let mut headers = Vec::new();
    if let Some(r) = &referrer {
        headers.push(("Referer".to_string(), r.clone()));
    }
    if let Some(c) = nonempty(cap.cookies) {
        headers.push(("Cookie".to_string(), c));
    }
    if let Some(ua) = nonempty(cap.user_agent) {
        headers.push(("User-Agent".to_string(), ua));
    }
    let file_name = nonempty(cap.filename);

    if mgr.settings().capture_confirm {
        // 弹出独立确认小卡片，由用户确认文件名与保存位置。
        let payload = CapturePayload {
            url: parsed.to_string(),
            file_name,
            headers,
            file_size: cap.file_size.filter(|s| *s > 0).map(|s| s as u64),
            referrer,
            hls: cap.hls,
        };
        crate::capture::open(&mgr.app, payload)?;
    } else {
        mgr.add_tasks(
            vec![parsed.to_string()],
            AddOptions {
                save_dir: None,
                file_name,
                file_names: None,
                connections: None,
                start: true,
                headers,
                hls: cap.hls,
                torrent: None,
                scheduled_at: None,
            },
        )?;
    }
    Ok(())
}

fn accept_batch(mgr: &Arc<Manager>, batch: Batch) -> Result<usize, String> {
    let is_http = |u: &str| {
        reqwest::Url::parse(u)
            .map(|p| matches!(p.scheme(), "http" | "https"))
            .unwrap_or(false)
    };
    let items: Vec<BatchItem> = batch
        .items
        .into_iter()
        .filter(|i| is_http(&i.url) && !i.name.trim().is_empty())
        .take(MAX_BATCH)
        .collect();
    if items.is_empty() {
        return Err("没有可下载的文件".into());
    }
    let referrer = batch.referrer.filter(|r| is_http(r));
    let headers = referrer
        .iter()
        .map(|r| ("Referer".to_string(), r.clone()))
        .collect();
    let n = items.len();
    crate::capture::open_batch(
        &mgr.app,
        BatchPayload {
            source: batch.source.unwrap_or_default(),
            title: batch
                .title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "批量下载".into()),
            referrer,
            headers,
            items,
            truncated: batch.truncated,
        },
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_rules() {
        assert!(is_allowed_origin(None));
        assert!(is_allowed_origin(Some("chrome-extension://abcdefg")));
        assert!(is_allowed_origin(Some("moz-extension://1234")));
        assert!(!is_allowed_origin(Some("https://evil.example.com")));
        assert!(!is_allowed_origin(Some("null")));
        assert!(!is_allowed_origin(Some("http://127.0.0.1:8080")));
    }
}
