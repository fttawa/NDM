use crate::bridge::{Bridge, BridgeStatus};
use crate::manager::{AddOptions, Manager};
use crate::model::{DiskInfo, Settings, Task};
use crate::util;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager as _, State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

type M<'a> = State<'a, Arc<Manager>>;

#[tauri::command]
pub fn get_tasks(m: M) -> Vec<Task> {
    m.snapshot()
}

#[tauri::command]
pub fn get_settings(m: M) -> Settings {
    m.settings()
}

#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    m: M,
    bridge: State<'_, Arc<Bridge>>,
    settings: Settings,
) -> Result<Settings, String> {
    let old = m.settings();
    let old_glass = old.glass;
    let s = m.set_settings(settings)?;
    if s.browser_integration != old.browser_integration {
        if s.browser_integration {
            bridge.start(m.inner().clone());
        } else {
            bridge.stop();
        }
    }
    if s.glass != old_glass {
        refresh_glass(&app);
    }
    Ok(s)
}

/// 当前实际生效的窗口材质
#[derive(Default)]
pub struct GlassState(parking_lot::Mutex<String>);

#[tauri::command]
pub fn effective_glass(state: State<'_, GlassState>) -> String {
    state.0.lock().clone()
}

/// 按设置与窗口状态计算并应用窗口材质：
/// 最大化时亚克力临时换成云母（亚克力在大窗口下重绘开销大），还原后恢复。
/// 实际材质变化时通知前端，前端据此调整背景透明度。
pub fn refresh_glass(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let want = app.state::<Arc<Manager>>().settings().glass;
    let maximized = w.is_maximized().unwrap_or(false);
    let effective = if want == "acrylic" && maximized {
        "mica".to_string()
    } else {
        want
    };
    let state = app.state::<GlassState>();
    let mut cur = state.0.lock();
    if *cur != effective {
        apply_glass(&w, &effective);
        *cur = effective.clone();
        let _ = app.emit("ndm://glass", effective);
    }
}

#[tauri::command]
pub fn bridge_status(bridge: State<'_, Arc<Bridge>>) -> BridgeStatus {
    bridge.status()
}

#[tauri::command]
pub fn export_data(m: M, path: String) -> Result<(), String> {
    m.export_data(Path::new(&path))
}

#[tauri::command]
pub fn preview_import(m: M, path: String) -> Result<serde_json::Value, String> {
    m.preview_import(Path::new(&path))
}

#[tauri::command]
pub fn import_data(
    app: AppHandle,
    m: M,
    bridge: State<'_, Arc<Bridge>>,
    ticket: String,
) -> Result<String, String> {
    let backup = m.import_data(ticket)?;
    if m.settings().browser_integration {
        bridge.start(m.inner().clone());
    } else {
        bridge.stop();
    }
    refresh_glass(&app);
    Ok(backup)
}

/// 随安装包附带的浏览器扩展目录（开发模式下为项目内的 extension 目录）
#[tauri::command]
pub fn extension_dir(app: AppHandle) -> Result<String, String> {
    let bundled = app
        .path()
        .resource_dir()
        .map(|d| d.join("extension"))
        .ok()
        .filter(|d| d.join("manifest.json").exists());
    let dir = bundled
        .or_else(|| {
            let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../extension");
            dev.join("manifest.json").exists().then_some(dev)
        })
        .ok_or("未找到扩展目录")?;
    let dir = dir.canonicalize().unwrap_or(dir);
    // 去掉 Windows 的 \\?\ 前缀，便于用户复制路径
    Ok(dir
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string())
}

/// 用给定的代理设置访问测试地址，返回耗时（毫秒）
#[tauri::command]
pub async fn test_proxy(settings: Settings) -> Result<u64, String> {
    let client = crate::manager::build_client(&settings)?;
    let start = std::time::Instant::now();
    let resp = client
        .get("https://cp.cloudflare.com/generate_204")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "连接超时".to_string()
            } else if e.is_connect() {
                "无法连接（请检查代理地址和端口）".to_string()
            } else {
                format!("请求失败：{e}")
            }
        })?;
    if resp.status().is_success() {
        Ok(start.elapsed().as_millis() as u64)
    } else {
        Err(format!("服务器返回 HTTP {}", resp.status().as_u16()))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddArgs {
    save_dir: Option<String>,
    file_name: Option<String>,
    file_names: Option<Vec<String>>,
    connections: Option<u32>,
    start: Option<bool>,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

#[tauri::command]
pub fn add_tasks(m: M, urls: Vec<String>, options: AddArgs) -> Result<Vec<Task>, String> {
    m.add_tasks(
        urls,
        AddOptions {
            save_dir: options.save_dir,
            file_name: options.file_name,
            file_names: options.file_names,
            connections: options.connections,
            start: options.start.unwrap_or(true),
            headers: options.headers,
        },
    )
}

#[tauri::command]
pub fn pause_tasks(m: M, ids: Vec<String>) {
    m.pause(&ids);
}

#[tauri::command]
pub fn resume_tasks(m: M, ids: Vec<String>) {
    m.resume(&ids);
}

#[tauri::command]
pub fn remove_tasks(m: M, ids: Vec<String>, delete_file: bool) {
    m.remove(&ids, delete_file);
}

#[tauri::command]
pub fn pause_all(m: M) {
    m.pause_all();
}

#[tauri::command]
pub fn resume_all(m: M) {
    m.resume_all();
}

#[tauri::command]
pub fn clear_finished(m: M) {
    m.clear_finished();
}

#[tauri::command]
pub fn redownload(m: M, id: String) {
    m.redownload(&id);
}

#[tauri::command]
pub fn open_task_file(app: AppHandle, m: M, id: String) -> Result<(), String> {
    let path = m.task_path(&id).ok_or("任务不存在")?;
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_task(app: AppHandle, m: M, id: String) -> Result<(), String> {
    let path = m.task_path(&id).ok_or("任务不存在")?;
    if path.is_dir() {
        app.opener()
            .open_path(path.to_string_lossy().to_string(), None::<&str>)
            .map_err(|e| e.to_string())
    } else {
        app.opener()
            .reveal_item_in_dir(path)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn open_dir(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// 用系统浏览器打开网址（只允许 https，防止被用来打开本地程序）
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "网址无效".to_string())?;
    if parsed.scheme() != "https" {
        return Err("只允许打开 https 网址".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn disk_info(path: String) -> Option<DiskInfo> {
    let p = util::existing_ancestor(Path::new(&path))?;
    Some(DiskInfo {
        free: fs2::available_space(p).ok()?,
        total: fs2::total_space(p).ok()?,
    })
}

/// 设置窗口毛玻璃效果：acrylic | mica | none
pub fn apply_glass(window: &WebviewWindow, mode: &str) {
    let result = match mode {
        "acrylic" => window.set_effects(EffectsBuilder::new().effect(Effect::Acrylic).build()),
        "mica" => window.set_effects(EffectsBuilder::new().effect(Effect::Mica).build()),
        _ => window.set_effects(None),
    };
    if let Err(e) = result {
        eprintln!("设置窗口效果失败: {e}");
    }
}

#[tauri::command]
pub fn export_diagnostics(m: M, path: String) -> Result<(), String> {
    let tasks: Vec<_> = m.snapshot().iter().enumerate().map(|(index,t)| serde_json::json!({
        "index":index,"status":t.status,"downloaded":t.downloaded,"total":t.total,
        "connections":t.connections,"segmented":!t.segments.is_empty(),"hasError":t.error.is_some()
    })).collect();
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,
        "arch":std::env::consts::ARCH,"tasks":tasks,"events":crate::diagnostics::events()});
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_update(m: M<'_>) -> Result<serde_json::Value, String> {
    let response = m
        .client()
        .get("https://api.github.com/repos/fttawa/NDM/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "NDM-update-check")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "无法连接更新服务，请检查网络或代理".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "更新服务返回 HTTP {}，请稍后重试",
            response.status().as_u16()
        ));
    }
    let release: serde_json::Value =
        serde_json::from_slice(&response.bytes().await.map_err(|_| "读取更新信息失败")?)
            .map_err(|_| "更新信息格式无效")?;
    let tag = release["tag_name"].as_str().ok_or("缺少版本号")?;
    let newer =
        version_tuple(tag).ok_or("版本号无效")? > version_tuple(env!("CARGO_PKG_VERSION")).unwrap();
    Ok(
        serde_json::json!({"version":tag,"available":newer,"notes":release["body"].as_str().unwrap_or(""),
        "url":format!("https://github.com/fttawa/NDM/releases/tag/{tag}")}),
    )
}

fn version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim_start_matches('v').split('.');
    let value = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    #[test]
    fn compares_numeric_versions() {
        assert!(version_tuple("v0.2.10") > version_tuple("0.2.9"));
        assert_eq!(version_tuple("v0.2.2"), version_tuple("0.2.2"));
        assert!(version_tuple("1.2.3.4").is_none());
        assert!(version_tuple("1.2.3-beta").is_none());
        assert!(version_tuple("bad-version").is_none());
    }
}
