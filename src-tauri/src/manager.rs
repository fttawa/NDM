//! 任务管理：队列调度、状态维护、持久化、进度广播

use crate::engine::{self, ActiveRt, DlError};
use crate::limiter::Limiter;
use crate::model::{now_ms, Progress, Settings, Status, Task};
use crate::storage::{ExportBundle, Storage};
use crate::util;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_notification::NotificationExt;

/// 速度统计的滑动窗口：越长越平稳，越短越灵敏
const SPEED_WINDOW: Duration = Duration::from_secs(3);

pub struct Manager {
    pub app: AppHandle,
    tasks: Mutex<Vec<Task>>,
    active: Mutex<HashMap<String, Arc<ActiveRt>>>,
    persist_lock: Mutex<()>,
    settings: RwLock<Settings>,
    client: RwLock<reqwest::Client>,
    pub limiter: Limiter,
    storage: Storage,
    pending_import: Mutex<Option<(String, ExportBundle)>>,
    pub torrent_engine: crate::torrent::Engine,
    updating: AtomicBool,
}

pub struct AddOptions {
    pub save_dir: Option<String>,
    pub file_name: Option<String>,
    /// 与 urls 一一对应的文件名（批量添加时使用，如 OpenList 列表中的真实文件名）
    pub file_names: Option<Vec<String>>,
    pub connections: Option<u32>,
    pub start: bool,
    pub headers: Vec<(String, String)>,
    pub hls: bool,
    pub torrent: Option<crate::torrent::Source>,
}

/// 按设置构建 HTTP 客户端；自定义代理地址无效时返回错误
pub fn build_client(s: &Settings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    match s.proxy_mode.as_str() {
        "none" => builder = builder.no_proxy(),
        "custom" => {
            let url = s.proxy_url.trim();
            if url.is_empty() {
                return Err("请填写代理地址".into());
            }
            // 未写协议时默认按 HTTP 代理处理
            let url = if url.contains("://") {
                url.to_string()
            } else {
                format!("http://{url}")
            };
            let proxy = reqwest::Proxy::all(&url).map_err(|e| format!("代理地址无效：{e}"))?;
            builder = builder.proxy(proxy);
        }
        // system：reqwest 默认读取环境变量与 Windows 系统代理
        _ => {}
    }
    builder
        .user_agent(crate::user_agent::effective(&s.user_agent))
        // 强制 HTTP/1.1：每个分段独占一条 TCP 连接，HTTP/2 多路复用会让多线程加速失效
        .http1_only()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

impl Manager {
    pub fn new(app: AppHandle) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        // NDM_DATA_DIR：使用独立的数据目录（开发测试用，不影响正在使用的实例）
        let data_dir = match std::env::var_os("NDM_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => app.path().app_data_dir()?,
        };
        std::fs::create_dir_all(&data_dir)?;

        crate::diagnostics::init(&data_dir);
        let storage = Storage::open(data_dir.join("state.db"))?;
        let mut settings: Settings = match storage.load_settings()? {
            Some(settings) => settings,
            None => {
                let settings = read_json(&data_dir.join("settings.json")).unwrap_or_default();
                storage.save_settings(&settings)?;
                settings
            }
        };
        if settings.download_dir.is_empty() {
            settings.download_dir = app
                .path()
                .download_dir()
                .or_else(|_| app.path().home_dir())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        settings.max_concurrent = settings.max_concurrent.clamp(1, 16);
        settings.connections = settings.connections.clamp(1, 32);
        settings.bt.validate()?;
        crate::user_agent::migrate_legacy(&mut settings.user_agent);

        let mut tasks: Vec<Task> = match storage.load_tasks()? {
            Some(tasks) => tasks,
            None => {
                let tasks =
                    read_json::<Vec<Task>>(&data_dir.join("tasks.json")).unwrap_or_default();
                storage.save_tasks(&tasks)?;
                tasks
            }
        };
        for t in tasks.iter_mut() {
            let update_resume = std::mem::take(&mut t.resume_after_update);
            t.speed = 0;
            t.active_conns = 0;
            t.boost = 0;
            if matches!(t.status, Status::Active | Status::Waiting)
                || (update_resume && t.status == Status::Paused)
            {
                t.status = if settings.resume_on_start || update_resume {
                    Status::Waiting
                } else {
                    Status::Paused
                };
            }
        }
        storage.save_settings(&settings)?;
        storage.save_tasks(&tasks)?;

        let client = build_client(&settings).unwrap_or_else(|e| {
            eprintln!("{e}，回退为跟随系统代理");
            build_client(&Settings {
                proxy_mode: "system".into(),
                ..settings.clone()
            })
            .unwrap_or_default()
        });

        Ok(Arc::new(Self {
            client: RwLock::new(client),
            limiter: Limiter::new(settings.speed_limit * 1024),
            settings: RwLock::new(settings),
            tasks: Mutex::new(tasks),
            active: Mutex::new(HashMap::new()),
            persist_lock: Mutex::new(()),
            app,
            storage,
            pending_import: Mutex::new(None),
            torrent_engine: Default::default(),
            updating: AtomicBool::new(false),
        }))
    }

    pub fn start(self: &Arc<Self>) {
        self.schedule();
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.ticker().await });
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    pub fn set_browser_user_agent(&self, value: String) -> Result<(), String> {
        let _guard = self.persist_lock.lock();
        crate::user_agent::set_browser(value)?;
        *self.client.write() = build_client(&self.settings())?;
        Ok(())
    }

    pub async fn prepare_update(self: &Arc<Self>) -> Result<(), String> {
        self.updating.store(true, Ordering::Release);
        let ids: Vec<_> = {
            let mut tasks = self.tasks.lock();
            tasks
                .iter_mut()
                .filter(|t| matches!(t.status, Status::Active | Status::Waiting))
                .map(|t| {
                    t.resume_after_update = true;
                    t.id.clone()
                })
                .collect()
        };
        self.pause(&ids);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while !self.active.lock().is_empty() {
            if tokio::time::Instant::now() >= deadline {
                self.abort_update();
                return Err("下载任务尚未安全停止，已取消本次安装，稍后自动重试".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let result = {
            let _guard = self.persist_lock.lock();
            self.storage
                .save_tasks(&self.tasks.lock())
                .map_err(|_| "保存下载进度失败，已取消安装".to_string())
        };
        if result.is_err() {
            self.abort_update();
        }
        result
    }

    pub fn abort_update(self: &Arc<Self>) {
        let ids: Vec<_> = self
            .tasks
            .lock()
            .iter_mut()
            .filter_map(|t| std::mem::take(&mut t.resume_after_update).then(|| t.id.clone()))
            .collect();
        self.updating.store(false, Ordering::Release);
        self.resume(&ids);
        self.persist();
    }

    pub fn bt_details(&self, id: &str) -> Result<Option<crate::bt_config::Runtime>, String> {
        {
            let tasks = self.tasks.lock();
            let task = tasks
                .iter()
                .find(|task| task.id == id)
                .ok_or("任务不存在")?;
            if task.torrent.is_none() {
                return Err("该任务不是 BT 下载".into());
            }
        }
        let runtime = self.active.lock().get(id).cloned();
        Ok(runtime.and_then(|rt| rt.bt_details.lock().clone()))
    }

    pub fn refresh_bt_metadata(&self, id: &str) -> Result<(), String> {
        let task = self.task(id).ok_or("任务不存在")?;
        let source = task.torrent.ok_or("该任务不是 BT 下载")?;
        if source.metainfo.is_empty() || source.file_sizes.len() == source.files.len() {
            return Ok(());
        }
        let (new_source, _, _) = crate::libtorrent::inspect_for(&source.metainfo, source.version)?;
        if new_source.info_hash != source.info_hash {
            return Err("BT 元数据哈希不匹配".into());
        }
        self.update_task(id, |task| {
            if task.torrent.as_ref().is_some_and(|current| {
                current.info_hash == new_source.info_hash
                    && current.file_sizes.len() != current.files.len()
            }) {
                task.torrent = Some(new_source);
            }
        });
        self.persist();
        self.emit_tasks();
        Ok(())
    }

    pub fn export_data(&self, path: &Path) -> Result<(), String> {
        self.persist();
        let _guard = self.persist_lock.lock();
        self.storage
            .export_bundle(path)
            .map_err(|e| format!("导出失败：{e}"))
    }

    pub fn preview_import(&self, path: &Path) -> Result<serde_json::Value, String> {
        let bundle = Storage::read_bundle(path).map_err(|e| format!("备份无效：{e}"))?;
        build_client(&bundle.settings)?;
        let old = serde_json::to_value(self.settings()).map_err(|e| e.to_string())?;
        let new = serde_json::to_value(&bundle.settings).map_err(|e| e.to_string())?;
        let changes: Vec<String> = new
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, value)| old.get(*key) != Some(*value))
            .map(|(key, _)| key.clone())
            .collect();
        let ticket = uuid::Uuid::new_v4().to_string();
        let preview = serde_json::json!({"ticket":ticket,"taskCount":bundle.tasks.len(),"currentCount":self.tasks.lock().len(),"settingsChanges":changes});
        *self.pending_import.lock() = Some((ticket, bundle));
        Ok(preview)
    }

    pub fn import_data(self: &Arc<Self>, ticket: String) -> Result<String, String> {
        let _persist = self.persist_lock.lock();
        let active = self.active.lock();
        if !active.is_empty() {
            return Err("请先暂停所有下载，等待连接退出后再导入".into());
        }
        let mut pending = self.pending_import.lock();
        let (expected, bundle) = pending.as_mut().ok_or("请先预览备份")?;
        if *expected != ticket {
            return Err("备份预览已失效，请重新选择".into());
        }
        bundle.settings.max_concurrent = bundle.settings.max_concurrent.clamp(1, 16);
        bundle.settings.connections = bundle.settings.connections.clamp(1, 32);
        bundle.settings.bt.validate()?;
        let client = build_client(&bundle.settings)?;
        let mut tasks = self.tasks.lock();
        for task in &mut bundle.tasks {
            task.resume_after_update = false;
            task.speed = 0;
            task.active_conns = 0;
            task.boost = 0;
            task.connections = if task.torrent.is_some() {
                80
            } else {
                task.connections.clamp(1, 32)
            };
            if matches!(task.status, Status::Active | Status::Waiting) {
                task.status = Status::Paused;
            }
        }
        let backup = self
            .storage
            .replace_bundle(
                bundle,
                &ExportBundle {
                    version: 2,
                    settings: self.settings(),
                    tasks: tasks.clone(),
                },
            )
            .map_err(|e| format!("导入失败，数据未替换：{e}"))?;
        *tasks = bundle.tasks.clone();
        *self.settings.write() = bundle.settings.clone();
        *self.client.write() = client;
        self.limiter
            .set_rate(bundle.settings.speed_limit.saturating_mul(1024));
        *pending = None;
        drop(tasks);
        drop(active);
        drop(_persist);
        self.emit_tasks();
        Ok(backup.to_string_lossy().into_owned())
    }

    pub fn client(&self) -> reqwest::Client {
        self.client.read().clone()
    }

    pub fn snapshot(&self) -> Vec<Task> {
        let mut tasks = self.tasks.lock().clone();
        for task in &mut tasks {
            if let Some(source) = &mut task.torrent {
                source.metainfo.clear();
            }
        }
        tasks
    }

    pub fn task(&self, id: &str) -> Option<Task> {
        self.tasks.lock().iter().find(|t| t.id == id).cloned()
    }

    pub fn update_task(&self, id: &str, f: impl FnOnce(&mut Task)) {
        if let Some(t) = self.tasks.lock().iter_mut().find(|t| t.id == id) {
            f(t);
        }
    }

    /// Reserve a task's filename while holding the task list lock. This keeps
    /// concurrent additions from selecting the same name.
    pub fn reserve_task_name(&self, id: &str, dir: &str, candidate: &str) -> String {
        let mut tasks = self.tasks.lock();
        let dir_path = Path::new(dir);
        let name = util::unique_name(dir_path, candidate, |name| {
            tasks
                .iter()
                .any(|t| t.id != id && t.save_dir == dir && t.name_locked && t.file_name == name)
        });
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.file_name = name.clone();
            task.name_locked = true;
        }
        name
    }

    pub fn emit_tasks(&self) {
        let _ = self.app.emit("ndm://tasks", self.snapshot());
    }

    /// 把运行中任务的分段进度同步回任务列表并写盘
    pub fn persist(&self) {
        let _persist = self.persist_lock.lock();
        let actives: Vec<(String, Arc<ActiveRt>)> = self
            .active
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let tasks = {
            let mut tasks = self.tasks.lock();
            for (id, rt) in &actives {
                if let Some(t) = tasks.iter_mut().find(|t| &t.id == id) {
                    if rt.segmented.load(std::sync::atomic::Ordering::Acquire) {
                        t.segments = rt.segments();
                    }
                    t.downloaded = rt.downloaded();
                }
            }
            tasks.clone()
        };
        if let Err(e) = self.storage.save_tasks(&tasks) {
            crate::diagnostics::record("tasks_save_failed");
            eprintln!("保存任务失败：{e}");
        }
    }

    pub fn set_settings(self: &Arc<Self>, mut s: Settings) -> Result<Settings, String> {
        let guard = self.persist_lock.lock();
        s.max_concurrent = s.max_concurrent.clamp(1, 16);
        s.connections = s.connections.clamp(1, 32);
        s.bt.validate()?;
        s.proxy_url = s.proxy_url.trim().to_string();
        let net_changed = {
            let cur = self.settings.read();
            s.user_agent != cur.user_agent
                || s.proxy_mode != cur.proxy_mode
                || s.proxy_url != cur.proxy_url
        };
        let client = if net_changed {
            Some(build_client(&s)?)
        } else {
            None
        };
        self.storage
            .save_settings(&s)
            .map_err(|e| format!("保存设置失败：{e}"))?;
        if let Some(client) = client {
            *self.client.write() = client;
        }
        *self.settings.write() = s.clone();
        self.sync_limits();
        drop(guard);
        self.schedule();
        Ok(s)
    }

    /// 按并发上限启动等待中的任务
    pub fn sync_limits(&self) {
        let rate = self.settings.read().speed_limit.saturating_mul(1024);
        let tasks = self.tasks.lock();
        let bt = tasks
            .iter()
            .filter(|t| t.status == Status::Active && t.torrent.is_some())
            .count() as u64;
        let http = tasks
            .iter()
            .filter(|t| t.status == Status::Active && t.torrent.is_none())
            .count() as u64;
        let per_task = if rate == 0 {
            0
        } else {
            (rate / (bt + http).max(1)).max(1)
        };
        self.limiter.set_rate(if http > 0 {
            per_task.saturating_mul(http)
        } else {
            rate
        });
        self.torrent_engine.set_limit(per_task);
    }
    pub fn schedule(self: &Arc<Self>) {
        if self.updating.load(Ordering::Acquire) {
            return;
        }
        let max = self.settings.read().max_concurrent as usize;
        let mut started = Vec::new();
        {
            let mut active = self.active.lock();
            if self.updating.load(Ordering::Acquire) {
                return;
            }
            let mut tasks = self.tasks.lock();
            for t in tasks.iter_mut() {
                if active.len() >= max {
                    break;
                }
                if t.status == Status::Waiting && !active.contains_key(&t.id) {
                    t.status = Status::Active;
                    t.error = None;
                    let rt = Arc::new(ActiveRt::new());
                    active.insert(t.id.clone(), rt.clone());
                    started.push((t.id.clone(), rt));
                }
            }
        }
        self.sync_limits();
        for (id, rt) in started {
            let me = self.clone();
            tauri::async_runtime::spawn(async move { me.run_task(id, rt).await });
        }
        self.emit_tasks();
    }

    async fn run_task(self: Arc<Self>, id: String, rt: Arc<ActiveRt>) {
        let res = engine::run(self.clone(), id.clone(), rt.clone()).await;
        let mut done_name = None;
        {
            // Do not report an idle engine until the final piece/segment
            // progress has been copied back; the updater waits on this lock.
            let mut active = self.active.lock();
            active.remove(&id);
            let mut tasks = self.tasks.lock();
            if let Some(t) = tasks.iter_mut().find(|t| t.id == id) {
                if rt.segmented.load(std::sync::atomic::Ordering::Acquire) {
                    t.segments = rt.segments();
                }
                t.downloaded = rt.downloaded();
                t.speed = 0;
                t.active_conns = 0;
                t.boost = 0;
                match res {
                    Ok(()) => {
                        t.status = Status::Completed;
                        if let Some(total) = t.total {
                            t.downloaded = total;
                        }
                        t.finished_at = Some(now_ms());
                        t.error = None;
                        done_name = Some(t.file_name.clone());
                    }
                    Err(DlError::Cancelled) => {
                        if t.status == Status::Active {
                            t.status = Status::Paused;
                        }
                    }
                    Err(e) => {
                        crate::diagnostics::record("download_failed");
                        t.status = Status::Error;
                        t.error = Some(e.to_string());
                    }
                }
            }
        }
        if let Some(name) = done_name {
            if self.settings.read().notify {
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title("下载完成")
                    .body(name)
                    .show();
            }
        }
        self.persist();
        self.schedule();
    }

    async fn ticker(self: Arc<Self>) {
        // 每个任务最近几秒的 (时刻, 已下载) 采样，用滑动窗口计算速度
        let mut samples: HashMap<String, std::collections::VecDeque<(Instant, u64)>> =
            HashMap::new();
        let mut tick = 0u64;
        let mut was_active = false;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            tick += 1;
            let actives: Vec<(String, Arc<ActiveRt>)> = self
                .active
                .lock()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if actives.is_empty() {
                samples.clear();
                if was_active {
                    was_active = false;
                    let _ = self.app.emit("ndm://progress", Vec::<Progress>::new());
                }
                continue;
            }
            was_active = true;
            let now = Instant::now();
            let mut progress = Vec::with_capacity(actives.len());
            {
                let mut tasks = self.tasks.lock();
                for (id, rt) in &actives {
                    let downloaded = rt.downloaded();
                    let win = samples.entry(id.clone()).or_default();
                    win.push_back((now, downloaded));
                    while win.len() > 2 && now.duration_since(win[0].0) > SPEED_WINDOW {
                        win.pop_front();
                    }
                    let (t0, b0) = win[0];
                    let dt = now.duration_since(t0).as_secs_f64();
                    let speed = if dt > 0.05 {
                        downloaded.saturating_sub(b0) as f64 / dt
                    } else {
                        0.0
                    };
                    let conns = rt.conns.load(std::sync::atomic::Ordering::Acquire);
                    let boost = rt.boost();
                    let latency_ms = rt.latency_ms.load(std::sync::atomic::Ordering::Acquire);
                    if let Some(t) = tasks.iter_mut().find(|t| &t.id == id) {
                        t.downloaded = downloaded;
                        let speed = if t.torrent.is_some() {
                            rt.torrent_speed.load(std::sync::atomic::Ordering::Acquire)
                        } else {
                            speed as u64
                        };
                        t.speed = speed;
                        t.active_conns = conns;
                        t.boost = boost;
                        t.latency_ms = latency_ms;
                        progress.push(Progress {
                            id: id.clone(),
                            downloaded,
                            total: t.total,
                            speed,
                            active_conns: conns,
                            boost,
                            latency_ms,
                            segments: rt.segments(),
                        });
                    }
                }
            }
            samples.retain(|k, _| actives.iter().any(|(id, _)| id == k));
            let _ = self.app.emit("ndm://progress", progress);
            if tick.is_multiple_of(10) {
                self.persist();
            }
        }
    }

    pub fn add_tasks(
        self: &Arc<Self>,
        urls: Vec<String>,
        opts: AddOptions,
    ) -> Result<Vec<Task>, String> {
        let settings = self.settings();
        // 先与文件名配对再过滤无效链接，避免名字错位
        let single_name = if urls.len() == 1 {
            opts.file_name.clone()
        } else {
            None
        };
        let names: Vec<Option<String>> = match opts.file_names {
            Some(v) if v.len() == urls.len() => v.into_iter().map(Some).collect(),
            _ => vec![single_name; urls.len()],
        };
        let pairs: Vec<(String, Option<String>)> = urls
            .into_iter()
            .zip(names)
            .map(|(u, n)| {
                (
                    u.trim().to_string(),
                    n.filter(|n| !n.trim().is_empty())
                        .map(|n| util::sanitize(&n)),
                )
            })
            .filter(|(u, _)| {
                reqwest::Url::parse(u)
                    .map(|p| matches!(p.scheme(), "http" | "https" | "magnet"))
                    .unwrap_or(false)
            })
            .collect();
        if pairs.is_empty() {
            return Err("没有有效的 HTTP/HTTPS 或磁力链接".into());
        }
        // Validate the whole batch before mutating the queue.
        let sources: Vec<_> = pairs
            .iter()
            .map(|(url, _)| {
                if url
                    .get(..7)
                    .is_some_and(|s| s.eq_ignore_ascii_case("magnet:"))
                {
                    crate::torrent::magnet_source(url).map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<_, String>>()?;
        let save_dir = opts
            .save_dir
            .filter(|d| !d.trim().is_empty())
            .unwrap_or(settings.download_dir.clone());
        let conns = opts
            .connections
            .unwrap_or(settings.connections)
            .clamp(1, 32);
        let mut created = Vec::new();
        // 本批次已占用的文件名（同批同名文件也要去重）
        let mut batch_names = std::collections::HashSet::new();
        let mut tasks = self.tasks.lock();
        let mut hashes = std::collections::HashSet::new();
        for (source, _) in sources.iter().flatten() {
            if !hashes.insert(&source.info_hash)
                || tasks.iter().any(|t| {
                    t.torrent
                        .as_ref()
                        .is_some_and(|s| s.info_hash == source.info_hash)
                })
            {
                return Err("该种子已在任务列表中，请继续或重新下载已有任务".into());
            }
        }
        for ((url, custom), magnet) in pairs.into_iter().zip(sources) {
            let url = if magnet.is_some() {
                reqwest::Url::parse(&url).unwrap().to_string()
            } else {
                url
            };
            let torrent = magnet
                .as_ref()
                .map(|(source, _)| opts.torrent.clone().unwrap_or_else(|| source.clone()));
            if let Some(source) = torrent.as_ref() {
                crate::torrent::validate_source(source, &url)?;
            }
            let hls = torrent.is_none() && (opts.hls || crate::hls::is_hls_url(&url));
            let total = torrent
                .as_ref()
                .filter(|source| !source.metainfo.is_empty())
                .map(|source| {
                    crate::torrent::parse_metainfo(&source.metainfo).map(|(_, _, total)| total)
                })
                .transpose()?;
            let custom = if torrent.is_some() {
                Some(custom.unwrap_or_else(|| magnet.as_ref().unwrap().1.clone()))
            } else {
                custom
            };
            let (file_name, locked) = match custom {
                Some(n) => (n, true),
                None => (
                    util::filename_from_url(&url)
                        .map(|n| util::sanitize(&n))
                        .unwrap_or_else(|| "download".into()),
                    false,
                ),
            };
            let file_name = if locked {
                let unique = util::unique_name(Path::new(&save_dir), &file_name, |n| {
                    batch_names.contains(n)
                        || tasks
                            .iter()
                            .any(|t| t.save_dir == save_dir && t.name_locked && t.file_name == n)
                });
                batch_names.insert(unique.clone());
                unique
            } else {
                file_name
            };
            created.push(Task {
                id: uuid::Uuid::new_v4().to_string(),
                hls,
                torrent,
                url,
                file_name,
                name_locked: locked,
                save_dir: save_dir.clone(),
                total,
                downloaded: 0,
                status: if opts.start {
                    Status::Waiting
                } else {
                    Status::Paused
                },
                resumable: magnet.is_some(),
                force_single: false,
                segments: Vec::new(),
                connections: if magnet.is_some() {
                    80
                } else if hls {
                    1
                } else {
                    conns
                },
                created_at: now_ms(),
                finished_at: None,
                error: None,
                mime: None,
                headers: if magnet.is_some() {
                    vec![]
                } else {
                    opts.headers.clone()
                },
                speed: 0,
                active_conns: 0,
                boost: 0,
                latency_ms: 0,
                resume_after_update: false,
            });
        }
        // 新任务放在最前面；文件名检查与插入在同一把锁内完成。
        for t in created.iter().rev() {
            tasks.insert(0, t.clone());
        }
        drop(tasks);
        self.persist();
        self.schedule();
        for task in &mut created {
            if let Some(source) = &mut task.torrent {
                source.metainfo.clear();
            }
        }
        Ok(created)
    }

    pub fn pause(self: &Arc<Self>, ids: &[String]) {
        {
            let mut tasks = self.tasks.lock();
            for t in tasks.iter_mut().filter(|t| ids.contains(&t.id)) {
                if matches!(t.status, Status::Active | Status::Waiting) {
                    t.status = Status::Paused;
                    t.speed = 0;
                }
            }
        }
        let to_cancel: Vec<_> = {
            let active = self.active.lock();
            ids.iter()
                .filter_map(|id| active.get(id).cloned())
                .collect()
        };
        for rt in to_cancel {
            rt.cancel.cancel();
        }
        self.persist();
        self.schedule();
    }

    pub fn resume(self: &Arc<Self>, ids: &[String]) {
        {
            let mut tasks = self.tasks.lock();
            for t in tasks.iter_mut().filter(|t| ids.contains(&t.id)) {
                if matches!(t.status, Status::Paused | Status::Error) {
                    t.status = Status::Waiting;
                    t.error = None;
                }
            }
        }
        self.schedule();
    }

    pub fn pause_all(self: &Arc<Self>) {
        let ids: Vec<String> = self.tasks.lock().iter().map(|t| t.id.clone()).collect();
        self.pause(&ids);
    }

    pub fn resume_all(self: &Arc<Self>) {
        let ids: Vec<String> = self.tasks.lock().iter().map(|t| t.id.clone()).collect();
        self.resume(&ids);
    }

    pub fn remove(self: &Arc<Self>, ids: &[String], delete_file: bool) {
        let removed: Vec<Task> = {
            let mut tasks = self.tasks.lock();
            let (gone, keep): (Vec<Task>, Vec<Task>) =
                tasks.drain(..).partition(|t| ids.contains(&t.id));
            *tasks = keep;
            gone
        };
        for t in &removed {
            if let Some(rt) = self.active.lock().get(&t.id) {
                rt.cancel.cancel();
            }
        }
        if delete_file {
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                for t in removed {
                    // 等待下载线程释放文件句柄
                    for _ in 0..100 {
                        if !me.active.lock().contains_key(&t.id) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    if me.active.lock().contains_key(&t.id) {
                        continue;
                    }
                    if let Some(source) = &t.torrent {
                        if let Err(e) =
                            crate::torrent::delete_files(&t.save_dir, &t.file_name, source)
                        {
                            eprintln!("删除 BT 文件失败：{e}");
                        }
                        continue;
                    }
                    let dir = Path::new(&t.save_dir);
                    let _ = std::fs::remove_file(dir.join(format!("{}.part", t.file_name)));
                    if t.status == Status::Completed {
                        let _ = std::fs::remove_file(dir.join(&t.file_name));
                    }
                }
            });
        }
        self.persist();
        self.emit_tasks();
    }

    pub fn clear_finished(self: &Arc<Self>) {
        self.tasks.lock().retain(|t| t.status != Status::Completed);
        self.persist();
        self.emit_tasks();
    }

    /// 重置任务进度并重新下载
    pub fn redownload(self: &Arc<Self>, id: &str) {
        if self.active.lock().contains_key(id) {
            return;
        }
        {
            let mut tasks = self.tasks.lock();
            if let Some(t) = tasks.iter_mut().find(|t| t.id == id) {
                let dir = Path::new(&t.save_dir);
                if t.torrent.is_some() {
                    // Keep previous content and use a fresh folder for an explicit restart.
                    t.file_name = util::unique_name(dir, &t.file_name, |_| false);
                    t.name_locked = true;
                }
                let _ = std::fs::remove_file(dir.join(format!("{}.part", t.file_name)));
                if t.torrent.is_none()
                    && (t.status == Status::Completed || dir.join(&t.file_name).exists())
                {
                    // 已存在同名文件时重新探测命名，避免覆盖
                    t.name_locked = false;
                }
                t.segments.clear();
                t.downloaded = 0;
                t.total = None;
                t.resumable = false;
                t.force_single = false;
                t.finished_at = None;
                t.error = None;
                t.status = Status::Waiting;
            }
        }
        self.schedule();
    }

    /// 任务对应的磁盘路径：完成后为正式文件，否则为 .part
    pub fn task_path(&self, id: &str) -> Option<PathBuf> {
        let t = self.task(id)?;
        let dir = PathBuf::from(&t.save_dir);
        let final_path = dir.join(&t.file_name);
        if final_path.exists() {
            return Some(final_path);
        }
        let part = dir.join(format!("{}.part", t.file_name));
        if part.exists() {
            return Some(part);
        }
        Some(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_proxy(mode: &str, url: &str) -> Settings {
        Settings {
            proxy_mode: mode.into(),
            proxy_url: url.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn proxy_settings() {
        assert!(build_client(&with_proxy("system", "")).is_ok());
        assert!(build_client(&with_proxy("none", "")).is_ok());
        assert!(build_client(&with_proxy("custom", "http://127.0.0.1:7890")).is_ok());
        assert!(build_client(&with_proxy("custom", "socks5://user:pass@127.0.0.1:1080")).is_ok());
        // 省略协议时按 HTTP 代理处理
        assert!(build_client(&with_proxy("custom", "127.0.0.1:7890")).is_ok());
        assert!(build_client(&with_proxy("custom", "")).is_err());
        assert!(build_client(&with_proxy("custom", "http://")).is_err());
    }

    /// 需要联网，手动运行：cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn proxy_live() {
        for s in [
            with_proxy("none", ""),
            with_proxy("custom", "http://127.0.0.1:7897"),
            with_proxy("custom", "socks5://127.0.0.1:7897"),
            with_proxy("custom", "http://127.0.0.1:1"),
        ] {
            let r = crate::commands::test_proxy(s.clone()).await;
            println!("{:>8} {:<26} => {:?}", s.proxy_mode, s.proxy_url, r);
        }
    }
}
