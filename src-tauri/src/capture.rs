//! 下载确认小卡片（类似 IDM 的“下载文件信息”对话框）
//!
//! 浏览器扩展转交下载时，单独弹出一个置顶的小窗口，不打扰主窗口。
//! 每个下载一个窗口，标签为 `capture-<id>`，前端按 id 取回下载信息。

use crate::bridge::{BatchPayload, CapturePayload};
use crate::manager::Manager;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, LogicalSize, Manager as _, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL_PREFIX: &str = "capture-";
const WIDTH: f64 = 460.0;
const HEIGHT: f64 = 268.0;

/// 批量下载卡片尺寸（含文件列表）
const BATCH_WIDTH: f64 = 560.0;
const BATCH_HEIGHT: f64 = 540.0;

/// 等待确认的下载（单个 / 批量）
#[derive(Default)]
pub struct Captures {
    single: Mutex<HashMap<String, CapturePayload>>,
    batch: Mutex<HashMap<String, BatchPayload>>,
}

pub fn is_capture_window(label: &str) -> bool {
    label.starts_with(LABEL_PREFIX)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

/// 单个下载的确认卡片
pub fn open(app: &AppHandle, payload: CapturePayload) -> Result<(), String> {
    let id = new_id();
    app.state::<Captures>().single.lock().insert(id.clone(), payload);
    open_window(app, &id, &format!("capture/{id}"), "NDM - 新的下载", WIDTH, HEIGHT)
}

/// 批量下载卡片（如 OpenList 当前文件夹）
pub fn open_batch(app: &AppHandle, payload: BatchPayload) -> Result<(), String> {
    let id = new_id();
    app.state::<Captures>().batch.lock().insert(id.clone(), payload);
    open_window(app, &id, &format!("batch/{id}"), "NDM - 批量下载", BATCH_WIDTH, BATCH_HEIGHT)
}

fn open_window(app: &AppHandle, id: &str, route: &str, title: &str, width: f64, height: f64) -> Result<(), String> {

    // 已打开的卡片数，用于错开叠放
    let stacked = app.webview_windows().keys().filter(|l| is_capture_window(l)).count();

    let window = WebviewWindowBuilder::new(
        app,
        format!("{LABEL_PREFIX}{id}"),
        WebviewUrl::App(format!("index.html#{route}").into()),
    )
    .title(title)
    .inner_size(width, height)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(true)
    .always_on_top(true)
    .center()
    // 前端渲染完成后再显示，避免白屏闪烁
    .visible(false)
    .build()
    .map_err(|e| format!("无法打开下载确认窗口：{e}"))?;

    if stacked > 0 {
        if let Ok(pos) = window.outer_position() {
            let offset = (stacked as i32 % 8) * 28;
            let _ = window.set_position(PhysicalPosition::new(pos.x + offset, pos.y + offset));
        }
    }

    let glass = app.state::<Arc<Manager>>().settings().glass;
    crate::commands::apply_glass(&window, &glass);

    // 兜底：前端未能主动显示时，稍后强制显示
    let w = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        if !w.is_visible().unwrap_or(true) {
            show(&w);
        }
    });
    Ok(())
}

/// 显示并置前（Windows 会阻止后台程序抢焦点，置顶窗口可以保证可见）
pub fn show(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
pub fn get_capture(state: tauri::State<'_, Captures>, id: String) -> Option<CapturePayload> {
    state.single.lock().get(&id).cloned()
}

#[tauri::command]
pub fn get_batch(state: tauri::State<'_, Captures>, id: String) -> Option<BatchPayload> {
    state.batch.lock().get(&id).cloned()
}

/// 前端渲染完成后调用：显示卡片窗口
#[tauri::command]
pub fn capture_ready(window: WebviewWindow) {
    show(&window);
}

/// 以缓动动画调整窗口高度（向下展开，顶部不动）。
/// 在后端线程中逐帧设置尺寸，避免前端每帧一次 IPC 往返导致卡顿。
#[tauri::command]
pub fn animate_height(window: WebviewWindow, height: f64, duration: u64) {
    std::thread::spawn(move || {
        let scale = window.scale_factor().unwrap_or(1.0);
        let Ok(size) = window.inner_size() else { return };
        let width = size.width as f64 / scale;
        let from = size.height as f64 / scale;
        let total = Duration::from_millis(duration.max(1)).as_secs_f64();
        let start = Instant::now();
        loop {
            let t = (start.elapsed().as_secs_f64() / total).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            let _ = window.set_size(LogicalSize::new(width, from + (height - from) * eased));
            if t >= 1.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(7));
        }
    });
}

/// 确认或取消后关闭卡片，并清理待确认数据
#[tauri::command]
pub fn close_capture(window: WebviewWindow, state: tauri::State<'_, Captures>, id: String) {
    state.single.lock().remove(&id);
    state.batch.lock().remove(&id);
    let _ = window.close();
}
