mod bridge;
mod capture;
mod commands;
mod diagnostics;
mod engine;
mod hls;
mod libtorrent;
mod limiter;
mod manager;
mod model;
mod storage;
mod torrent;
mod util;

use manager::Manager;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager as _, RunEvent, WindowEvent};

pub(crate) fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        // Windows 会阻止后台程序抢焦点，短暂置顶确保窗口出现在浏览器之上
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
        let _ = w.set_always_on_top(false);
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", "全部开始", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", "全部暂停", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &resume, &pause, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("NDM 下载管理器")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let mgr = app.state::<Arc<Manager>>();
            match event.id.as_ref() {
                "show" => show_main(app),
                "resume_all" => mgr.resume_all(),
                "pause_all" => mgr.pause_all(),
                "quit" => {
                    mgr.persist();
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // 指定独立数据目录（测试实例）时不做单实例限制
    if std::env::var_os("NDM_DATA_DIR").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }));
    }
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let mgr = Manager::new(app.handle().clone())?;
            app.manage(mgr.clone());
            let bridge = Arc::new(bridge::Bridge::default());
            if mgr.settings().browser_integration {
                bridge.start(mgr.clone());
            }
            app.manage(bridge);
            app.manage(commands::GlassState::default());
            app.manage(capture::Captures::default());
            commands::refresh_glass(app.handle());
            mgr.start();
            build_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Destroyed if capture::is_capture_window(window.label()) => {
                capture::cleanup_window(window)
            }
            // 以下行为只针对主窗口；下载确认卡片正常关闭
            _ if window.label() != "main" => {}
            WindowEvent::CloseRequested { api, .. } => {
                let mgr = window.state::<Arc<Manager>>();
                if mgr.settings().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // 最大化/还原会触发 Resized，据此切换材质
            WindowEvent::Resized(_) => commands::refresh_glass(window.app_handle()),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_tasks,
            commands::get_settings,
            commands::set_settings,
            commands::add_tasks,
            commands::inspect_hls,
            commands::add_torrent_file,
            commands::pause_tasks,
            commands::resume_tasks,
            commands::remove_tasks,
            commands::pause_all,
            commands::resume_all,
            commands::clear_finished,
            commands::redownload,
            commands::open_task_file,
            commands::reveal_task,
            commands::open_dir,
            commands::open_url,
            commands::disk_info,
            commands::test_proxy,
            commands::bridge_status,
            commands::export_data,
            commands::import_data,
            commands::preview_import,
            commands::export_diagnostics,
            commands::check_update,
            commands::effective_glass,
            capture::get_capture,
            capture::get_batch,
            capture::capture_ready,
            capture::close_capture,
            capture::animate_height,
            commands::extension_dir,
        ])
        .build(tauri::generate_context!())
        .expect("启动 NDM 失败");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(mgr) = app.try_state::<Arc<Manager>>() {
                mgr.persist();
            }
        }
    });
}
