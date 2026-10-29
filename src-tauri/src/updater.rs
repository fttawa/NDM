//! Signed updates. Mirrors only transport public GitHub release assets; the
//! embedded public key and signed version are required for every installation.
use crate::{manager::Manager, model::now_ms};
use parking_lot::RwLock;
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

const MANIFEST: &str = "https://github.com/fttawa/NDM/releases/latest/download/latest.json";
const MIRRORS: [&str; 2] = ["https://ghfast.top/", "https://gh-proxy.com/"];

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub version: String,
    pub notes: String,
    pub source: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub checked_at: Option<u64>,
    pub error: String,
}

pub struct Updates {
    state: RwLock<Status>,
    gate: tokio::sync::Mutex<()>,
    wake: tokio::sync::Notify,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            state: RwLock::new(Status {
                phase: "idle".into(),
                ..Default::default()
            }),
            gate: Default::default(),
            wake: Default::default(),
        }
    }
}

fn routes(url: &str, mirrors: bool) -> Vec<(String, String)> {
    let mut result = vec![("GitHub".into(), url.into())];
    if mirrors {
        result.extend(MIRRORS.iter().map(|prefix| {
            (
                prefix
                    .trim_start_matches("https://")
                    .trim_end_matches('/')
                    .into(),
                format!("{prefix}{url}"),
            )
        }));
    }
    result
}

fn validate_asset(update: &Update) -> Result<(), String> {
    validate_asset_url(update.download_url.as_str(), &update.version)
}

fn validate_asset_url(url: &str, version: &str) -> Result<(), String> {
    let expected = format!(
        "https://github.com/fttawa/NDM/releases/download/v{}/NDM_{}_x64-setup.exe",
        version, version
    );
    if url != expected {
        return Err("更新包地址与版本不匹配，已拒绝安装".into());
    }
    Ok(())
}

impl Updates {
    pub fn settings_changed(&self) {
        self.wake.notify_one();
    }
    pub fn status(&self) -> Status {
        self.state.read().clone()
    }

    fn change(&self, app: &AppHandle, edit: impl FnOnce(&mut Status)) {
        let snapshot = {
            let mut state = self.state.write();
            edit(&mut state);
            state.clone()
        };
        let _ = app.emit("ndm://update", snapshot);
    }

    pub fn start(self: &Arc<Self>, app: AppHandle, mgr: Arc<Manager>) {
        let me = self.clone();
        // Development and isolated QA instances never install updates by themselves.
        if cfg!(debug_assertions) || std::env::var_os("NDM_DATA_DIR").is_some() {
            return;
        }
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            loop {
                if mgr.settings().auto_update {
                    let _ = me.run(&app, &mgr, false).await;
                }
                let delay = if me.status().phase == "error" {
                    15 * 60
                } else {
                    6 * 60 * 60
                };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                    _ = me.wake.notified() => {},
                }
            }
        });
    }

    pub async fn run(
        &self,
        app: &AppHandle,
        mgr: &Arc<Manager>,
        manual: bool,
    ) -> Result<Status, String> {
        let Ok(_guard) = self.gate.try_lock() else {
            return Ok(self.status());
        };
        self.change(app, |s| {
            s.phase = "checking".into();
            s.error.clear();
            s.downloaded = 0;
            s.total = None;
        });
        let result = self.perform(app, mgr, manual).await;
        if let Err(error) = &result {
            crate::diagnostics::record("update_failed");
            self.change(app, |s| {
                s.phase = "error".into();
                s.error = error.clone();
            });
        }
        result.map(|()| self.status())
    }

    async fn perform(
        &self,
        app: &AppHandle,
        mgr: &Arc<Manager>,
        manual: bool,
    ) -> Result<(), String> {
        let settings = mgr.settings();
        let mut found = None;
        for (source, endpoint) in routes(MANIFEST, settings.update_mirrors) {
            self.change(app, |s| s.source = source.clone());
            let mut builder = app
                .updater_builder()
                .endpoints(vec![endpoint.parse().map_err(|_| "更新地址无效")?])
                .map_err(|_| "更新配置无效")?
                .timeout(Duration::from_secs(12))
                .configure_client(|client| {
                    client
                        .connect_timeout(Duration::from_secs(8))
                        .read_timeout(Duration::from_secs(15))
                })
                .restart_after_install(true);
            match settings.proxy_mode.as_str() {
                "none" => builder = builder.no_proxy(),
                "custom" => {
                    let proxy = if settings.proxy_url.contains("://") {
                        settings.proxy_url.clone()
                    } else {
                        format!("http://{}", settings.proxy_url)
                    };
                    builder = builder.proxy(proxy.parse().map_err(|_| "代理地址无效")?);
                }
                _ => {}
            }
            let updater = builder.build().map_err(|_| "无法初始化签名更新服务")?;
            match updater.check().await {
                Ok(Some(update)) => {
                    found = Some(update);
                    break;
                }
                Ok(None) => {
                    self.change(app, |s| {
                        s.phase = "current".into();
                        s.checked_at = Some(now_ms());
                        s.version = env!("CARGO_PKG_VERSION").into();
                        s.notes.clear();
                    });
                    return Ok(());
                }
                Err(_) => continue,
            }
        }
        let mut update = found.ok_or("更新服务暂不可达，已尝试可用线路，稍后将自动重试")?;
        validate_asset(&update)?;
        self.change(app, |s| {
            s.version = update.version.clone();
            s.notes = update.body.clone().unwrap_or_default();
            s.checked_at = Some(now_ms());
        });
        update.timeout = Some(Duration::from_secs(180));
        let mut package = None;
        for (source, url) in routes(update.download_url.as_str(), settings.update_mirrors) {
            if !manual && !mgr.settings().auto_update {
                self.change(app, |s| s.phase = "available".into());
                return Ok(());
            }
            self.change(app, |s| {
                s.phase = "downloading".into();
                s.source = source;
                s.downloaded = 0;
                s.total = None;
            });
            update.download_url = url.parse().map_err(|_| "下载地址无效")?;
            let mut downloaded = 0;
            let mut emitted = Instant::now();
            match update
                .download(
                    |chunk, total| {
                        downloaded += chunk as u64;
                        if emitted.elapsed() >= Duration::from_millis(250) {
                            self.change(app, |s| {
                                s.downloaded = downloaded;
                                s.total = total;
                            });
                            emitted = Instant::now();
                        }
                    },
                    || self.change(app, |s| s.phase = "verifying".into()),
                )
                .await
            {
                Ok(bytes) => {
                    package = Some(bytes);
                    break;
                }
                Err(_) => continue,
            }
        }
        let package = package.ok_or("更新包下载或签名校验失败，已保留当前版本，稍后自动重试")?;
        if !manual && !mgr.settings().auto_update {
            self.change(app, |s| s.phase = "available".into());
            return Ok(());
        }
        self.change(app, |s| {
            s.phase = "saving".into();
            s.downloaded = package.len() as u64;
            s.total = Some(package.len() as u64);
        });
        mgr.prepare_update().await?;
        self.change(app, |s| s.phase = "installing".into());
        // The official Windows updater starts the silent NSIS installer with /R,
        // exits this process and restarts the installed app automatically.
        if let Err(_error) = update.install(package) {
            mgr.abort_update();
            return Err("无法启动更新安装程序，下载任务已恢复，请稍后重试".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mirrors_preserve_the_exact_public_url_and_can_be_disabled() {
        let url = "https://github.com/fttawa/NDM/releases/download/v1.2.3/NDM_1.2.3_x64-setup.exe";
        let result = routes(url, true);
        assert_eq!(result[0].1, url);
        assert_eq!(result[1].1, format!("https://ghfast.top/{url}"));
        assert_eq!(result.len(), 3);
        assert_eq!(routes(url, false).len(), 1);
        assert!(validate_asset_url(url, "1.2.3").is_ok());
        for bad in [
            "https://evil.invalid/app.exe",
            "https://github.com/other/NDM/releases/download/v1.2.3/NDM_1.2.3_x64-setup.exe",
            "http://github.com/fttawa/NDM/releases/download/v1.2.3/NDM_1.2.3_x64-setup.exe",
        ] {
            assert!(validate_asset_url(bad, "1.2.3").is_err());
        }
        assert!(validate_asset_url(url, "1.2.4").is_err());
    }
}
