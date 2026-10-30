//! Bundled libtorrent helper, isolated over private stdin/stdout (no TCP control service).
use crate::{
    engine::{ActiveRt, DlError},
    manager::Manager,
    torrent::{self, Source},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tauri::Manager as _;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn executable(app: Option<&tauri::AppHandle>) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        let local =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bt-helper/native/ndm-bt-engine.exe");
        if local.is_file() {
            return Ok(local);
        }
    }
    if let Some(app) = app {
        let installed = app
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("bt-helper/ndm-bt-engine.exe");
        if installed.is_file() {
            return Ok(installed);
        }
    }
    // Installed Windows resources live alongside the application executable.
    let installed = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("无法定位程序目录")?
        .join("bt-helper/ndm-bt-engine.exe");
    if installed.is_file() {
        return Ok(installed);
    }
    Err("BT 内核文件缺失，请重新安装完整版本；开发环境请先运行 pnpm build:bt-helper".into())
}

fn command(app: Option<&tauri::AppHandle>) -> Result<std::process::Command, String> {
    let mut command = std::process::Command::new(executable(app)?);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Inspection {
    info_hash: String,
    files: Vec<String>,
    total: u64,
    name: String,
    version: u8,
    file_sizes: Vec<u64>,
    trackers: Vec<String>,
    private: bool,
    piece_length: u64,
    piece_count: u32,
}

pub fn inspect(data: &[u8]) -> Result<(Source, String, u64), String> {
    inspect_for(data, 0)
}

pub fn inspect_for(data: &[u8], version: u8) -> Result<(Source, String, u64), String> {
    if data.is_empty() || data.len() > torrent::MAX_META {
        return Err("种子文件为空或超过 16 MiB".into());
    }
    let mut child = command(None)?
        .spawn()
        .map_err(|e| format!("无法启动 BT 内核：{e}"))?;
    let input = serde_json::to_vec(
        &serde_json::json!({"type":"inspect", "metainfo": STANDARD.encode(data), "version":version}),
    )
    .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    // Drain stdout concurrently to avoid blocking when a torrent contains many files.
    let stdout = child.stdout.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(64 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let write = stdin.write_all(&input).and_then(|_| stdin.write_all(b"\n"));
    drop(stdin);
    if let Err(error) = write {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("BT 种子解析超时".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = output
        .join()
        .map_err(|_| "BT 内核输出失败")?
        .map_err(|e| e.to_string())?;
    let event: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "BT 内核返回无效数据")?;
    if !status.success() || event["type"] != "inspection" {
        return Err(event["message"].as_str().unwrap_or("BT 种子无效").into());
    }
    let info: Inspection = serde_json::from_value(event).map_err(|e| e.to_string())?;
    for file in &info.files {
        torrent::validate_relative(std::path::Path::new(file))?;
    }
    Ok((
        Source {
            info_hash: info.info_hash,
            metainfo: data.to_vec(),
            files: info.files,
            version: info.version,
            file_sizes: info.file_sizes,
            trackers: info.trackers,
            private: info.private,
            piece_length: info.piece_length,
            piece_count: info.piece_count,
        },
        crate::util::sanitize(&info.name),
        info.total,
    ))
}

async fn send(
    stdin: &mut tokio::process::ChildStdin,
    event: serde_json::Value,
) -> Result<(), DlError> {
    let mut data = serde_json::to_vec(&event).map_err(|e| DlError::Other(e.to_string()))?;
    data.push(b'\n');
    stdin.write_all(&data).await.map_err(DlError::from)
}

pub async fn run(mgr: &Arc<Manager>, id: &str, rt: &Arc<ActiveRt>) -> Result<(), DlError> {
    let task = mgr.task(id).ok_or(DlError::Cancelled)?;
    let source = task
        .torrent
        .as_ref()
        .ok_or_else(|| DlError::Other("缺少 BT 元数据".into()))?;
    torrent::validate_source(source, &task.url).map_err(DlError::Other)?;
    let directory = PathBuf::from(&task.save_dir).join(&task.file_name);
    torrent::prepare_output(&directory, &source.info_hash).map_err(DlError::Other)?;
    let mut child = tokio::process::Command::from(command(Some(&mgr.app)).map_err(DlError::Other)?)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DlError::Other(format!("启动 BT 内核失败：{e}")))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    rt.single.store(task.downloaded, Ordering::Release);
    let mut limit = mgr.torrent_engine.limit();
    let mut bt = mgr.settings().bt;
    let config = serde_json::json!({"type":"run", "url":task.url,"metainfo":STANDARD.encode(&source.metainfo),
        "directory":directory,"hash":source.info_hash,"version":source.version,"limit":limit,"bt":bt});
    send(&mut stdin, config).await?;
    let mut stopping = false;
    let mut reconnect = false;
    let mut complete = false;
    let mut stop_deadline = tokio::time::Instant::now() + Duration::from_secs(86400 * 365);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let result = loop {
        tokio::select! {
            _ = rt.cancel.cancelled(), if !stopping => {
                stopping = true;
                stop_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
                if let Err(error) = send(&mut stdin, serde_json::json!({"type":"stop"})).await { break Err(error); }
            }
            _ = tokio::time::sleep_until(stop_deadline), if stopping => { break Err(DlError::Cancelled); }
            _ = tick.tick() => {
                let next_bt = mgr.settings().bt;
                if next_bt.proxy != bt.proxy && !stopping {
                    reconnect = true;
                    stopping = true;
                    stop_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
                    if let Err(error) = send(&mut stdin, serde_json::json!({"type":"stop"})).await { break Err(error); }
                }
                if next_bt != bt && !stopping {
                    bt = next_bt;
                    if let Err(error) = send(&mut stdin, serde_json::json!({"type":"configure","bt":bt})).await { break Err(error); }
                }
                let new_limit = mgr.torrent_engine.limit();
                if new_limit != limit {
                    limit = new_limit;
                    if let Err(error) = send(&mut stdin, serde_json::json!({"type":"limit","bps":limit})).await { break Err(error); }
                }
            }
            line = lines.next_line() => {
                let line = match line { Ok(Some(line)) => line, Ok(None) => break Err(DlError::Other("BT 内核意外退出".into())), Err(e) => break Err(e.into()) };
                let event: serde_json::Value = match serde_json::from_str(&line) { Ok(event) => event, Err(_) => break Err(DlError::Other("BT 内核协议错误".into())) };
                match event["type"].as_str() {
                    Some("metadata") => {
                        let data = match STANDARD.decode(event["metainfo"].as_str().unwrap_or("")) { Ok(data) if data.len() <= torrent::MAX_META => data, _ => break Err(DlError::Other("BT 元数据无效".into())) };
                        let (new_source, _, total) = match inspect_for(&data, source.version) { Ok(info) => info, Err(e) => break Err(DlError::Other(e)) };
                        if new_source.info_hash != source.info_hash || new_source.version != source.version { break Err(DlError::Other("BT 元数据哈希不匹配".into())); }
                        if let Err(e) = torrent::validate_output(&directory, &new_source.files) { break Err(DlError::Other(e)); }
                        mgr.update_task(id, |task| { task.torrent = Some(new_source); task.total = Some(total); task.resumable = true; });
                        mgr.persist(); mgr.emit_tasks();
                        if !stopping { if let Err(e) = send(&mut stdin, serde_json::json!({"type":"start"})).await { break Err(e); } }
                    }
                    Some("progress") => {
                        if let Some(details) = event.get("details") {
                            let details = match serde_json::from_value(details.clone()) {
                                Ok(details) => details,
                                Err(_) => break Err(DlError::Other("BT 详情数据无效".into())),
                            };
                            *rt.bt_details.lock() = Some(details);
                        }
                        if event["checking"] != true { rt.single.store(event["downloaded"].as_u64().unwrap_or(0), Ordering::Release); }
                        rt.torrent_speed.store(event["speed"].as_u64().unwrap_or(0), Ordering::Release);
                        rt.conns.store(event["peers"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32, Ordering::Release);
                    }
                    Some("complete") => { complete = true; break Ok(()); }
                    Some("stopped") => break Err(DlError::Cancelled),
                    Some("error") => break Err(DlError::Other(event["message"].as_str().unwrap_or("BT 下载失败").into())),
                    _ => break Err(DlError::Other("BT 内核事件无效".into())),
                }
            }
        }
    };
    drop(stdin);
    let exit = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
        Ok(exit) => exit.map_err(DlError::from),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(DlError::Other("BT 内核退出超时".into()))
        }
    };
    if result.is_ok() && !stopping {
        let status = exit?;
        if !status.success() {
            return Err(DlError::Other("BT 内核未正常退出".into()));
        }
    }
    if complete {
        rt.single.store(
            mgr.task(id)
                .and_then(|t| t.total)
                .unwrap_or(rt.downloaded()),
            Ordering::Release,
        );
    }
    if reconnect {
        Err(DlError::Restart)
    } else if stopping {
        Err(DlError::Cancelled)
    } else {
        result
    }
}
