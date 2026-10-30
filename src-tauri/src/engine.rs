//! 下载引擎：探测、多连接分段下载、动态分段、单流回退

use crate::manager::Manager;
use crate::model::{HttpIdentity, Segment};
use crate::util;
use futures_util::StreamExt;
use parking_lot::Mutex;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    IF_RANGE, LAST_MODIFIED, RANGE,
};
use reqwest::StatusCode;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// 分段最小尺寸
const MIN_SEGMENT: u64 = 1024 * 1024;
/// 动态拆分时每一半的最小尺寸：剩余 ≥ 2 × MIN_SPLIT 的分段才会被拆分，
/// 粒度足够细，空闲连接才能接手小文件和尾段
const MIN_SPLIT: u64 = 256 * 1024;
const MAX_RETRIES: u32 = 5;
/// 超频检查周期
const OVERCLOCK_TICK: Duration = Duration::from_millis(150);

fn atomic_try_update(
    atom: &AtomicU32,
    set_order: Ordering,
    load_order: Ordering,
    mut update: impl FnMut(u32) -> Option<u32>,
) -> Result<u32, u32> {
    let mut old = atom.load(load_order);
    loop {
        let Some(new) = update(old) else {
            return Err(old);
        };
        match atom.compare_exchange_weak(old, new, set_order, load_order) {
            Ok(_) => return Ok(old),
            Err(actual) => old = actual,
        }
    }
}

#[derive(Debug)]
pub enum DlError {
    Cancelled,
    /// 服务器不支持 Range 请求
    NoRange,
    Restart,
    Other(String),
}

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DlError::Cancelled => write!(f, "已取消"),
            DlError::NoRange => write!(f, "服务器不支持分段下载"),
            DlError::Restart => write!(f, "正在应用 BT 代理并重新连接"),
            DlError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl From<reqwest::Error> for DlError {
    fn from(e: reqwest::Error) -> Self {
        let msg = if e.is_timeout() {
            "连接超时".to_string()
        } else if e.is_connect() {
            "无法连接服务器".to_string()
        } else {
            // 拼接完整的错误链，便于定位 TLS/代理等底层问题
            let mut s = format!("网络错误：{e}");
            let mut src = std::error::Error::source(&e);
            while let Some(inner) = src {
                s.push_str(&format!(" ← {inner}"));
                src = inner.source();
            }
            s
        };
        DlError::Other(msg)
    }
}

impl From<std::io::Error> for DlError {
    fn from(e: std::io::Error) -> Self {
        DlError::Other(format!("文件错误：{e}"))
    }
}

/// 运行时分段状态，`end` 可能被动态拆分缩小
pub struct SegRt {
    pub start: u64,
    pub end: AtomicU64,
    pub pos: AtomicU64,
    pub assigned: AtomicBool,
    /// 该分段当前连接的速度（字节/秒，平滑后），未在传输时为 0
    pub rate: AtomicU64,
}

impl SegRt {
    fn new(start: u64, end: u64, pos: u64, assigned: bool) -> Self {
        Self {
            start,
            end: AtomicU64::new(end),
            pos: AtomicU64::new(pos),
            assigned: AtomicBool::new(assigned),
            rate: AtomicU64::new(0),
        }
    }
    fn remaining(&self) -> u64 {
        (self.end.load(Ordering::Acquire) + 1).saturating_sub(self.pos.load(Ordering::Acquire))
    }
    fn is_done(&self) -> bool {
        self.remaining() == 0
    }
}

/// 一个正在运行的任务的共享状态
pub struct ActiveRt {
    pub cancel: CancellationToken,
    pub segs: Mutex<Vec<Arc<SegRt>>>,
    pub segmented: AtomicBool,
    pub single: AtomicU64,
    pub torrent_speed: AtomicU64,
    pub bt_details: Mutex<Option<crate::bt_config::Runtime>>,
    /// 已建立（连接中 + 传输中）的连接数
    pub conns: AtomicU32,
    /// 已收到响应、正在传输数据的连接数
    pub streaming: AtomicU32,
    /// 存活的下载协程数（含重试等待中的）
    pub workers: AtomicU32,
    /// 常规连接数（任务设置的连接数），超出部分为超频连接
    pub target: AtomicU32,
    /// 请求发出到收到响应的平均耗时（毫秒），0 表示尚未测得
    pub latency_ms: AtomicU32,
}

impl ActiveRt {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            segs: Mutex::new(Vec::new()),
            segmented: AtomicBool::new(false),
            single: AtomicU64::new(0),
            torrent_speed: AtomicU64::new(0),
            bt_details: Mutex::new(None),
            conns: AtomicU32::new(0),
            streaming: AtomicU32::new(0),
            workers: AtomicU32::new(0),
            target: AtomicU32::new(0),
            latency_ms: AtomicU32::new(0),
        }
    }

    /// 记录一次请求延迟（指数平滑）
    fn record_latency(&self, d: Duration) {
        let ms = (d.as_millis() as u32).max(1);
        let _ = atomic_try_update(
            &self.latency_ms,
            Ordering::AcqRel,
            Ordering::Acquire,
            |old| {
                Some(if old == 0 {
                    ms
                } else {
                    (old * 7 + ms * 3) / 10
                })
            },
        );
    }

    /// 当前超频连接数
    pub fn boost(&self) -> u32 {
        let target = self.target.load(Ordering::Acquire);
        if target == 0 {
            return 0;
        }
        self.workers.load(Ordering::Acquire).saturating_sub(target)
    }

    /// 还有可分给新连接的工作（未分配的分段，或足够大可拆分的分段）
    fn has_work(&self) -> bool {
        self.segs.lock().iter().any(|s| {
            !s.is_done() && (!s.assigned.load(Ordering::Acquire) || s.remaining() >= MIN_SPLIT * 2)
        })
    }

    /// 一个延迟内就会传完、随后空闲的连接数
    fn finishing(&self, latency_ms: u32) -> u32 {
        // 留出一个检查周期的余量
        let horizon = latency_ms as u64 + OVERCLOCK_TICK.as_millis() as u64;
        self.segs
            .lock()
            .iter()
            .filter(|s| {
                let rate = s.rate.load(Ordering::Acquire);
                rate > 0 && !s.is_done() && s.remaining().saturating_mul(1000) / rate <= horizon
            })
            .count() as u32
    }

    /// 连接数超过常规值时让出一个名额（超频连接就位后，多出的连接传完当前分段即退出）
    fn retire(&self) -> bool {
        let target = self.target.load(Ordering::Acquire);
        atomic_try_update(&self.workers, Ordering::AcqRel, Ordering::Acquire, |w| {
            (w > target).then(|| w - 1)
        })
        .is_ok()
    }

    pub fn downloaded(&self) -> u64 {
        if self.segmented.load(Ordering::Acquire) {
            self.segs
                .lock()
                .iter()
                .map(|s| {
                    let end = s.end.load(Ordering::Acquire) + 1;
                    s.pos
                        .load(Ordering::Acquire)
                        .min(end)
                        .saturating_sub(s.start)
                })
                .sum()
        } else {
            self.single.load(Ordering::Acquire)
        }
    }

    pub fn segments(&self) -> Vec<Segment> {
        self.segs
            .lock()
            .iter()
            .map(|s| {
                let end = s.end.load(Ordering::Acquire);
                Segment {
                    start: s.start,
                    end,
                    pos: s.pos.load(Ordering::Acquire).min(end + 1),
                }
            })
            .collect()
    }

    /// 领取一个未分配的分段；没有时把剩余最多的分段一分为二（动态分段）
    fn take_segment(&self) -> Option<Arc<SegRt>> {
        let mut segs = self.segs.lock();
        for s in segs.iter() {
            if !s.assigned.load(Ordering::Acquire) && !s.is_done() {
                s.assigned.store(true, Ordering::Release);
                return Some(s.clone());
            }
        }
        let (idx, best) = segs
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_done())
            .max_by_key(|(_, s)| s.remaining())?;
        let rem = best.remaining();
        if rem < MIN_SPLIT * 2 {
            return None;
        }
        let pos = best.pos.load(Ordering::Acquire);
        let end = best.end.load(Ordering::Acquire);
        let mid = pos + rem / 2;
        best.end.store(mid - 1, Ordering::Release);
        let new = Arc::new(SegRt::new(mid, end, mid, true));
        segs.insert(idx + 1, new.clone());
        Some(new)
    }
}

struct ConnGuard<'a>(&'a AtomicU32);
impl<'a> ConnGuard<'a> {
    fn new(c: &'a AtomicU32) -> Self {
        c.fetch_add(1, Ordering::AcqRel);
        Self(c)
    }
}
impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 计算需要新开的超频连接数
///
/// - `streaming`：正在传输的连接，其中 `finishing` 个会在一个延迟内传完而空闲
/// - `connecting`：已发出请求、等待响应的连接，很快就会开始传输
/// - 重试等待中的连接（`workers - streaming - connecting`）视为掉速缺口
///
/// 有效连接不足常规连接数时补足，总数不超过 常规 + 超频上限
fn overclock_needed(
    target: u32,
    max_extra: u32,
    workers: u32,
    streaming: u32,
    connecting: u32,
    finishing: u32,
) -> u32 {
    let effective = streaming.saturating_sub(finishing) + connecting;
    let deficit = target.saturating_sub(effective);
    let room = (target + max_extra).saturating_sub(workers);
    deficit.min(room)
}

/// 每任务超频连接上限：连接数的一半
pub fn overclock_limit(connections: u32) -> u32 {
    connections / 2
}

/// 存活的下载协程计数，`retire` 成功后不再重复扣减
struct WorkerGuard {
    rt: Arc<ActiveRt>,
    retired: bool,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.retired {
            self.rt.workers.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// 分段传输期间：计入传输中的连接，结束时清零分段速度
struct StreamGuard<'a> {
    rt: &'a ActiveRt,
    seg: &'a SegRt,
}
impl<'a> StreamGuard<'a> {
    fn new(rt: &'a ActiveRt, seg: &'a SegRt) -> Self {
        rt.streaming.fetch_add(1, Ordering::AcqRel);
        Self { rt, seg }
    }
}
impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        self.seg.rate.store(0, Ordering::Release);
        self.rt.streaming.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 下载源：URL 与附加请求头（来自浏览器的 Cookie、Referer、User-Agent 等）
#[derive(Clone)]
struct Source {
    url: String,
    headers: Arc<Vec<(String, String)>>,
    identity: HttpIdentity,
    total: Option<u64>,
}

impl Source {
    fn from_task(task: &crate::model::Task) -> Self {
        Self {
            url: task.url.clone(),
            headers: Arc::new(task.headers.clone()),
            identity: task.http_identity.clone(),
            total: task.total,
        }
    }

    fn get(&self, client: &reqwest::Client) -> reqwest::RequestBuilder {
        let mut req = client.get(&self.url);
        for (k, v) in self.headers.iter() {
            if [
                "range",
                "if-range",
                "accept-encoding",
                "host",
                "content-length",
            ]
            .contains(&k.to_ascii_lowercase().as_str())
            {
                continue;
            }
            // 非法的头名称/值直接跳过，不影响下载
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                req = req.header(name, value);
            }
        }
        req.header(ACCEPT_ENCODING, "identity")
    }
}

struct ProbeInfo {
    total: Option<u64>,
    resumable: bool,
    file_name: Option<String>,
    mime: Option<String>,
    identity: HttpIdentity,
}

fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let (unit, range) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = range.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    if end < start {
        return None;
    }
    let total = (total != "*").then(|| total.parse().ok()).flatten();
    Some((start, end, total))
}

async fn probe(
    client: &reqwest::Client,
    src: &Source,
    rt: &ActiveRt,
) -> Result<ProbeInfo, DlError> {
    let t0 = Instant::now();
    let req = src.get(client).header(RANGE, "bytes=0-").send();
    let resp = tokio::select! {
        r = req => r?,
        _ = rt.cancel.cancelled() => return Err(DlError::Cancelled),
    };
    rt.record_latency(t0.elapsed());
    let status = resp.status();
    if !status.is_success() {
        return Err(DlError::Other(format!(
            "服务器返回 HTTP {}",
            status.as_u16()
        )));
    }
    let headers = resp.headers();
    let header_str = |k| {
        headers.get(k).map(|v: &reqwest::header::HeaderValue| {
            String::from_utf8_lossy(v.as_bytes()).to_string()
        })
    };

    let (total, resumable) = if status == StatusCode::PARTIAL_CONTENT {
        // Content-Range: bytes 0-999/1000
        match header_str(CONTENT_RANGE).and_then(|v| parse_content_range(&v)) {
            Some((0, _, total)) => (total, total.is_some()),
            _ => (None, false),
        }
    } else {
        let accept = header_str(ACCEPT_RANGES)
            .map(|v| v.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);
        let total = resp.content_length();
        (total, accept && total.is_some())
    };

    let file_name = header_str(CONTENT_DISPOSITION)
        .and_then(|v| util::filename_from_disposition(&v))
        .or_else(|| util::filename_from_url(resp.url().as_str()));
    let mime =
        header_str(CONTENT_TYPE).map(|v| v.split(';').next().unwrap_or("").trim().to_string());
    Ok(ProbeInfo {
        total,
        resumable,
        file_name,
        mime,
        identity: HttpIdentity {
            etag: header_str(ETAG),
            last_modified: header_str(LAST_MODIFIED),
        },
    })
}

fn plan_segments(total: u64, conns: u32) -> Vec<Segment> {
    let n = (total / MIN_SEGMENT).clamp(1, conns.max(1) as u64);
    let size = total / n;
    (0..n)
        .map(|i| {
            let start = i * size;
            let end = if i == n - 1 {
                total - 1
            } else {
                start + size - 1
            };
            Segment {
                start,
                end,
                pos: start,
            }
        })
        .collect()
}

pub async fn refresh_link(
    client: &reqwest::Client,
    original: &crate::model::Task,
    url: &str,
    restart: bool,
) -> Result<crate::model::Task, String> {
    let target = reqwest::Url::parse(url.trim()).map_err(|_| "请输入有效的 HTTP(S) 下载链接")?;
    if !matches!(target.scheme(), "http" | "https")
        || target.host_str().is_none()
        || !target.username().is_empty()
        || target.password().is_some()
        || url.len() > 16384
    {
        return Err("请输入不含用户名密码的 HTTP(S) 下载链接".into());
    }
    let mut task = original.clone();
    task.url = target.to_string();
    if reqwest::Url::parse(&original.url)
        .ok()
        .is_none_or(|old| old.origin() != target.origin())
    {
        // Browser credentials are scoped to the old origin. Do not forward
        // cookies, authorization or referrers to a newly pasted host.
        task.headers.retain(|(name, _)| {
            ["user-agent", "accept", "accept-language"]
                .contains(&name.to_ascii_lowercase().as_str())
        });
    }
    let info = tokio::time::timeout(
        Duration::from_secs(20),
        probe(client, &Source::from_task(&task), &ActiveRt::new()),
    )
    .await
    .map_err(|_| "新链接验证超时，原任务未修改".to_string())?
    .map_err(|e| format!("新链接验证失败：{e}"))?;
    if !restart && original.downloaded > 0 {
        if !original.resumable || original.segments.is_empty() || !info.resumable {
            return Err("新旧任务不支持分段续传；可勾选从头下载，原任务尚未修改".into());
        }
        if original.total.is_none() || info.total != original.total {
            return Err("新链接文件大小不同，无法沿用已有分段；原任务尚未修改".into());
        }
        if !original.http_identity.matches(&info.identity) {
            return Err("ETag / 修改时间不匹配或旧任务缺少文件标识，无法确认同一文件；可勾选从头下载，原任务尚未修改".into());
        }
    }
    if restart || original.downloaded == 0 {
        task.segments.clear();
        task.downloaded = 0;
        task.resumable = false;
        task.force_single = false;
    }
    task.total = info.total;
    task.mime = info.mime;
    task.http_identity = info.identity;
    task.status = crate::model::Status::Paused;
    task.error = None;
    task.finished_at = None;
    task.speed = 0;
    task.active_conns = 0;
    task.boost = 0;
    Ok(task)
}

/// 下载入口：分段模式失败于 NoRange 时自动降级为单连接
pub async fn run(mgr: Arc<Manager>, id: String, rt: Arc<ActiveRt>) -> Result<(), DlError> {
    if mgr.task(&id).is_some_and(|task| task.torrent.is_some()) {
        return crate::torrent::run(&mgr, &id, &rt).await;
    }
    if mgr.task(&id).is_some_and(|task| task.hls) {
        return crate::hls::run(&mgr, &id, &rt).await;
    }
    loop {
        match run_once(&mgr, &id, &rt).await {
            Err(DlError::NoRange) => {
                mgr.update_task(&id, |t| {
                    t.force_single = true;
                    t.resumable = false;
                    t.segments.clear();
                    t.downloaded = 0;
                });
                rt.segs.lock().clear();
                rt.segmented.store(false, Ordering::Release);
            }
            r => return r,
        }
    }
}

async fn run_once(mgr: &Arc<Manager>, id: &str, rt: &Arc<ActiveRt>) -> Result<(), DlError> {
    let client = mgr.client();
    let mut task = mgr.task(id).ok_or(DlError::Cancelled)?;
    rt.single.store(task.downloaded, Ordering::Release);
    let dir = PathBuf::from(&task.save_dir);
    std::fs::create_dir_all(&dir)?;

    // 恢复任务也重新探测一次，避免服务器文件或 Range 能力发生变化后继续写旧分段。
    let mut probed = None;
    if task.resumable && !task.segments.is_empty() {
        let info = probe(&client, &Source::from_task(&task), rt).await?;
        if task.downloaded > 0
            && task.http_identity.validator().is_some()
            && (info.total != task.total || !task.http_identity.matches(&info.identity))
        {
            return Err(DlError::Other(
                "服务器文件已变化，已保留原有进度；请更新链接或重新下载".into(),
            ));
        }
        if info.resumable && info.total == task.total {
            probed = Some(info);
        } else {
            task.resumable = false;
            task.segments.clear();
            task.downloaded = 0;
            mgr.update_task(id, |t| {
                t.resumable = false;
                t.segments.clear();
                t.downloaded = 0;
            });
            probed = Some(info);
        }
    }

    // 没有可续传的进度时重新探测
    if !task.resumable || task.segments.is_empty() {
        let info = match probed {
            Some(info) => info,
            None => probe(&client, &Source::from_task(&task), rt).await?,
        };
        if !task.name_locked {
            let raw = info
                .file_name
                .clone()
                .unwrap_or_else(|| task.file_name.clone());
            let clean = util::sanitize(&raw);
            task.file_name = mgr.reserve_task_name(id, &task.save_dir, &clean);
            task.name_locked = true;
        }
        task.total = info.total;
        task.mime = info.mime;
        task.http_identity = info.identity;
        task.resumable = info.resumable && !task.force_single;
        task.segments = match (task.resumable, task.total) {
            (true, Some(total)) if total > 0 => plan_segments(total, task.connections),
            _ => Vec::new(),
        };
        task.downloaded = 0;
        let snapshot = task.clone();
        mgr.update_task(id, move |t| {
            t.file_name = snapshot.file_name;
            t.name_locked = true;
            t.total = snapshot.total;
            t.mime = snapshot.mime;
            t.http_identity = snapshot.http_identity;
            t.resumable = snapshot.resumable;
            t.segments = snapshot.segments;
            t.downloaded = 0;
        });
        mgr.emit_tasks();
    }

    let part = dir.join(format!("{}.part", task.file_name));

    if task.total == Some(0) {
        File::create(&part)?;
    } else if task.resumable {
        segmented(mgr, &task, &part, rt).await?;
    } else {
        single(mgr, &task, &part, rt).await?;
    }

    finalize(mgr, id, &dir, &part, &task.file_name)
}

pub(crate) fn finalize(
    mgr: &Arc<Manager>,
    id: &str,
    dir: &Path,
    part: &Path,
    name: &str,
) -> Result<(), DlError> {
    let mut final_name = name.to_string();
    if dir.join(&final_name).exists() {
        final_name = util::unique_name(dir, name, |_| false);
    }
    std::fs::rename(part, dir.join(&final_name))?;
    if final_name != name {
        mgr.update_task(id, |t| t.file_name = final_name);
    }
    Ok(())
}

async fn segmented(
    mgr: &Arc<Manager>,
    task: &crate::model::Task,
    part: &Path,
    rt: &Arc<ActiveRt>,
) -> Result<(), DlError> {
    let total = task.total.unwrap_or(0);
    let existed = part.exists();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(part)?;
    let size_matches = file.metadata()?.len() == total;
    if !size_matches {
        file.set_len(total)?;
    }
    let file = Arc::new(file);

    // .part 文件丢失时进度作废，从头开始
    let segments = if !existed || !size_matches {
        plan_segments(total, task.connections)
    } else {
        task.segments.clone()
    };
    {
        let mut segs = rt.segs.lock();
        *segs = segments
            .iter()
            .map(|s| Arc::new(SegRt::new(s.start, s.end, s.pos, false)))
            .collect();
    }
    rt.segmented.store(true, Ordering::Release);

    let worker_cancel = rt.cancel.child_token();
    let src = Source::from_task(task);
    let target = task.connections.max(1);
    rt.target.store(target, Ordering::Release);
    let mut set = JoinSet::new();
    let spawn = |set: &mut JoinSet<Result<(), DlError>>| {
        rt.workers.fetch_add(1, Ordering::AcqRel);
        set.spawn(worker(
            mgr.clone(),
            src.clone(),
            file.clone(),
            rt.clone(),
            worker_cancel.clone(),
        ));
    };
    for _ in 0..target {
        spawn(&mut set);
    }

    let mut first_err: Option<DlError> = None;
    let mut no_range = false;
    let mut tick = tokio::time::interval(OVERCLOCK_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let res = tokio::select! {
            r = set.join_next() => match r {
                Some(r) => r,
                None => break,
            },
            _ = tick.tick() => {
                // 超频：预判即将空闲 / 正在重试的连接，提前补上新连接
                if !worker_cancel.is_cancelled() && mgr.settings().overclock {
                    let latency = rt.latency_ms.load(Ordering::Acquire);
                    let streaming = rt.streaming.load(Ordering::Acquire);
                    let connecting = rt.conns.load(Ordering::Acquire).saturating_sub(streaming);
                    let finishing = if latency > 0 { rt.finishing(latency) } else { 0 };
                    let need = overclock_needed(
                        target,
                        overclock_limit(target),
                        rt.workers.load(Ordering::Acquire),
                        streaming,
                        connecting,
                        finishing,
                    );
                    for _ in 0..need {
                        if !rt.has_work() {
                            break;
                        }
                        spawn(&mut set);
                    }
                }
                continue;
            }
        };
        match res {
            Ok(Ok(())) | Ok(Err(DlError::Cancelled)) => {}
            Ok(Err(DlError::NoRange)) => {
                no_range = true;
                worker_cancel.cancel();
            }
            Ok(Err(e)) => {
                first_err.get_or_insert(e);
            }
            Err(e) => {
                first_err.get_or_insert(DlError::Other(format!("内部错误：{e}")));
            }
        }
    }
    file.sync_all().ok();

    if rt.cancel.is_cancelled() {
        return Err(DlError::Cancelled);
    }
    if no_range {
        return Err(DlError::NoRange);
    }
    let done = rt.segs.lock().iter().all(|s| s.is_done());
    if done {
        Ok(())
    } else {
        Err(first_err.unwrap_or_else(|| DlError::Other("下载未完成".into())))
    }
}

async fn worker(
    mgr: Arc<Manager>,
    src: Source,
    file: Arc<File>,
    rt: Arc<ActiveRt>,
    cancel: CancellationToken,
) -> Result<(), DlError> {
    let client = mgr.client();
    let mut guard = WorkerGuard {
        rt: rt.clone(),
        retired: false,
    };
    loop {
        if cancel.is_cancelled() {
            return Err(DlError::Cancelled);
        }
        // 连接数高于常规值（超频补上的连接已就位）时，传完当前分段就退出
        if rt.retire() {
            guard.retired = true;
            return Ok(());
        }
        let Some(seg) = rt.take_segment() else {
            return Ok(());
        };
        let mut attempt = 0u32;
        loop {
            let res = {
                let _guard = ConnGuard::new(&rt.conns);
                fetch_segment(&mgr.limiter, &client, &src, &file, &rt, &seg, &cancel).await
            };
            match res {
                Ok(()) => break,
                Err(DlError::Cancelled) => return Err(DlError::Cancelled),
                Err(DlError::NoRange) => return Err(DlError::NoRange),
                Err(e) => {
                    attempt += 1;
                    if attempt >= MAX_RETRIES {
                        // 释放分段，让其他连接有机会接手
                        seg.assigned.store(false, Ordering::Release);
                        return Err(e);
                    }
                    let backoff = Duration::from_millis(800 * 2u64.pow(attempt - 1));
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = cancel.cancelled() => return Err(DlError::Cancelled),
                    }
                }
            }
        }
    }
}

async fn fetch_segment(
    limiter: &crate::limiter::Limiter,
    client: &reqwest::Client,
    src: &Source,
    file: &File,
    rt: &ActiveRt,
    seg: &SegRt,
    cancel: &CancellationToken,
) -> Result<(), DlError> {
    let pos = seg.pos.load(Ordering::Acquire);
    let end = seg.end.load(Ordering::Acquire);
    if pos > end {
        return Ok(());
    }
    let t0 = Instant::now();
    let mut req = src.get(client).header(RANGE, format!("bytes={pos}-{end}"));
    if let Some(validator) = src.identity.validator() {
        req = req.header(IF_RANGE, validator);
    }
    let req = req.send();
    let resp = tokio::select! {
        r = req => r?,
        _ = cancel.cancelled() => return Err(DlError::Cancelled),
    };
    rt.record_latency(t0.elapsed());
    let status = resp.status();
    if status == StatusCode::OK {
        return Err(if src.identity.validator().is_some() {
            DlError::Other("文件标识或分段能力已变化，已停止续传".into())
        } else {
            DlError::NoRange
        });
    } else if status != StatusCode::PARTIAL_CONTENT {
        return Err(DlError::Other(format!(
            "服务器返回 HTTP {}",
            status.as_u16()
        )));
    } else {
        let range = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
            .ok_or_else(|| DlError::Other("服务器返回无效的 Content-Range".into()))?;
        if range.0 != pos || range.1 > end || range.2 != src.total {
            return Err(DlError::Other(
                "服务器返回的 Content-Range 与请求不匹配".into(),
            ));
        }
    }

    if let Some(etag) = resp.headers().get(ETAG).and_then(|v| v.to_str().ok()) {
        if src.identity.etag.as_deref().is_some_and(|old| old != etag) {
            return Err(DlError::Other("服务器文件 ETag 已变化，已停止续传".into()));
        }
    }

    let _streaming = StreamGuard::new(rt, seg);
    let mut stream = resp.bytes_stream();
    // 分段速度采样，用于预判何时传完
    let mut sample_t = Instant::now();
    let mut sample_pos = pos;
    loop {
        let next = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => return Err(DlError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk?;
        let wait = limiter.reserve(chunk.len());
        if !wait.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.cancelled() => return Err(DlError::Cancelled),
            }
        }
        let pos = seg.pos.load(Ordering::Acquire);
        let dt = sample_t.elapsed();
        if dt >= Duration::from_millis(300) {
            let inst = pos.saturating_sub(sample_pos) * 1000 / dt.as_millis().max(1) as u64;
            let old = seg.rate.load(Ordering::Acquire);
            seg.rate.store(
                if old == 0 {
                    inst
                } else {
                    (old * 6 + inst * 4) / 10
                },
                Ordering::Release,
            );
            sample_t = Instant::now();
            sample_pos = pos;
        }
        let end = seg.end.load(Ordering::Acquire);
        if pos > end {
            return Ok(());
        }
        // 分段可能已被拆分，只写到当前 end 为止
        let allowed = (chunk.len() as u64).min(end - pos + 1) as usize;
        util::write_at(file, &chunk[..allowed], pos)?;
        seg.pos.store(pos + allowed as u64, Ordering::Release);
        if allowed < chunk.len() {
            return Ok(());
        }
    }
    if seg.is_done() {
        Ok(())
    } else {
        Err(DlError::Other("连接提前断开".into()))
    }
}

async fn single(
    mgr: &Arc<Manager>,
    task: &crate::model::Task,
    part: &Path,
    rt: &Arc<ActiveRt>,
) -> Result<(), DlError> {
    let client = mgr.client();
    rt.segmented.store(false, Ordering::Release);
    rt.single.store(0, Ordering::Release);
    let _guard = ConnGuard::new(&rt.conns);

    let req = Source::from_task(task).get(&client).send();
    let resp = tokio::select! {
        r = req => r?,
        _ = rt.cancel.cancelled() => return Err(DlError::Cancelled),
    };
    if !resp.status().is_success() {
        return Err(DlError::Other(format!(
            "服务器返回 HTTP {}",
            resp.status().as_u16()
        )));
    }
    let mut writer = std::io::BufWriter::with_capacity(512 * 1024, File::create(part)?);
    let mut stream = resp.bytes_stream();
    let mut written = 0u64;
    loop {
        let next = tokio::select! {
            c = stream.next() => c,
            _ = rt.cancel.cancelled() => return Err(DlError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk?;
        let wait = mgr.limiter.reserve(chunk.len());
        if !wait.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = rt.cancel.cancelled() => return Err(DlError::Cancelled),
            }
        }
        writer.write_all(&chunk)?;
        written += chunk.len() as u64;
        rt.single.store(written, Ordering::Release);
    }
    writer.flush()?;
    drop(writer);

    match task.total {
        Some(total) if total != written => Err(DlError::Other("下载不完整，连接提前断开".into())),
        Some(_) => Ok(()),
        None => {
            mgr.update_task(&task.id, |t| t.total = Some(written));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partial_task(url: &str) -> crate::model::Task {
        serde_json::from_value(serde_json::json!({
            "id":"refresh-test", "url":url, "fileName":"kept.bin", "nameLocked":true,
            "saveDir":"unused", "total":16, "downloaded":8, "status":"error", "resumable":true,
            "connections":1, "createdAt":123, "finishedAt":null, "error":"HTTP 403", "mime":null,
            "segments":[{"start":0,"end":15,"pos":8}],
            "httpIdentity":{"etag":"\"content-v1\""},
            "headers":[["Cookie","old-secret"],["Authorization","Bearer old-secret"],["User-Agent","NDM-Test"],["Range","bytes=3-4"]]
        })).unwrap()
    }

    fn test_response(request: tiny_http::Request, total: u64, etag: Option<&str>, range: bool) {
        let mut response = tiny_http::Response::from_data(vec![0; total as usize]);
        if range {
            response = response.with_status_code(206).with_header(
                tiny_http::Header::from_bytes(
                    "Content-Range",
                    format!("bytes 0-{}/{total}", total - 1),
                )
                .unwrap(),
            );
        }
        if let Some(etag) = etag {
            response = response.with_header(tiny_http::Header::from_bytes("ETag", etag).unwrap());
        }
        request.respond(response).unwrap();
    }

    #[tokio::test]
    async fn refreshed_links_preserve_segments_only_for_the_same_resource() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let original = partial_task(&format!("{base}/expired"));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..6 {
                let req = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert!(req.headers().iter().any(|h| h.field.equiv("Cookie")));
                match req.url() {
                    "/ok" => test_response(req, 16, Some("\"content-v1\""), true),
                    "/changed" => test_response(req, 16, Some("\"content-v2\""), true),
                    "/size" => test_response(req, 17, Some("\"content-v1\""), true),
                    "/no-range" => test_response(req, 16, Some("\"content-v1\""), false),
                    "/forbidden" => req.respond(tiny_http::Response::empty(403)).unwrap(),
                    _ => panic!("unexpected URL"),
                }
            }
        });
        let next = refresh_link(&client, &original, &format!("{base}/ok"), false)
            .await
            .unwrap();
        assert_eq!(next.downloaded, 8);
        assert_eq!(next.segments[0].pos, 8);
        assert_eq!(next.file_name, original.file_name);
        assert_eq!(next.save_dir, original.save_dir);
        assert_eq!(next.status, crate::model::Status::Paused);
        assert!(next.error.is_none());
        for path in ["changed", "size", "no-range", "forbidden"] {
            assert!(
                refresh_link(&client, &original, &format!("{base}/{path}"), false)
                    .await
                    .is_err(),
                "{path}"
            );
        }
        let restarted = refresh_link(&client, &original, &format!("{base}/changed"), true)
            .await
            .unwrap();
        assert_eq!(restarted.downloaded, 0);
        assert!(restarted.segments.is_empty());
        assert_eq!(original.downloaded, 8);
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn refreshed_cross_origin_drops_credentials_and_requires_identity() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/fresh", server.server_addr());
        let original = partial_task("https://old.invalid/expired");
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let req = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert!(!req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Cookie") || h.field.equiv("Authorization")));
                assert!(req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("User-Agent") && h.value.as_str() == "NDM-Test"));
                test_response(req, 16, Some("\"content-v1\""), true);
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let next = refresh_link(&client, &original, &url, false).await.unwrap();
        assert_eq!(next.downloaded, 8);
        let mut legacy = original;
        legacy.http_identity = Default::default();
        assert!(refresh_link(&client, &legacy, &url, false).await.is_err());
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn resumed_range_checks_if_range_and_never_writes_changed_content() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let req = server
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap();
                assert!(req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("If-Range") && h.value.as_str() == "\"content-v1\""));
                assert!(req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Range") && h.value.as_str() == "bytes=8-15"));
                if req.url() == "/changed" {
                    test_response(req, 16, Some("\"changed\""), false);
                } else {
                    req.respond(
                        tiny_http::Response::from_data(b"ijklmnop".to_vec())
                            .with_status_code(206)
                            .with_header(
                                tiny_http::Header::from_bytes("Content-Range", "bytes 8-15/16")
                                    .unwrap(),
                            )
                            .with_header(
                                tiny_http::Header::from_bytes("ETag", "\"content-v1\"").unwrap(),
                            ),
                    )
                    .unwrap();
                }
            }
        });
        let path = std::env::temp_dir().join(format!("ndm-refresh-{}.part", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"abcdefgh00000000").unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let rt = ActiveRt::new();
        let cancel = CancellationToken::new();
        let seg = SegRt::new(0, 15, 8, false);
        let changed = Source::from_task(&partial_task(&format!("{base}/changed")));
        assert!(fetch_segment(
            &crate::limiter::Limiter::new(0),
            &client,
            &changed,
            &file,
            &rt,
            &seg,
            &cancel
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"abcdefgh00000000");
        let source = Source::from_task(&partial_task(&format!("{base}/ok")));
        fetch_segment(
            &crate::limiter::Limiter::new(0),
            &client,
            &source,
            &file,
            &rt,
            &seg,
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abcdefghijklmnop");
        drop(file);
        std::fs::remove_file(path).unwrap();
        worker.join().unwrap();
    }

    fn rt_with(segs: &[(u64, u64, u64, bool)]) -> ActiveRt {
        let rt = ActiveRt::new();
        *rt.segs.lock() = segs
            .iter()
            .map(|&(s, e, p, a)| Arc::new(SegRt::new(s, e, p, a)))
            .collect();
        rt.segmented.store(true, Ordering::Release);
        rt
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some((0, 99, Some(1000)))
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some((100, 199, None))
        );
        assert_eq!(parse_content_range("items 0-99/1000"), None);
        assert_eq!(parse_content_range("bytes 99-0/1000"), None);
    }

    #[test]
    fn takes_unassigned_first() {
        let rt = rt_with(&[(0, 99, 0, true), (100, 199, 100, false)]);
        let s = rt.take_segment().unwrap();
        assert_eq!(s.start, 100);
    }

    #[test]
    fn splits_largest_remaining() {
        let mb = 1024 * 1024;
        // 模拟截图场景：其余分段已完成，只剩一段还差 1MB 的慢连接
        let rt = rt_with(&[
            (0, 3 * mb - 1, 3 * mb, true),
            (3 * mb, 6 * mb - 1, 5 * mb, true),
        ]);
        let new = rt.take_segment().expect("剩余 1MB 应该被拆分");
        let segs = rt.segs.lock();
        assert_eq!(segs.len(), 3);
        // 慢连接保留前半，新连接接手后半，两者首尾相接
        assert_eq!(segs[1].end.load(Ordering::Acquire) + 1, new.start);
        assert_eq!(new.end.load(Ordering::Acquire), 6 * mb - 1);
        assert_eq!(new.start, 5 * mb + mb / 2);
        drop(segs);
        // 下载量统计不受拆分影响
        assert_eq!(rt.downloaded(), 3 * mb + 2 * mb);
    }

    #[test]
    fn overclock_fills_finishing_connections() {
        // 8 连接全部在传输，其中 3 个即将传完：补 3 个
        assert_eq!(overclock_needed(8, 4, 8, 8, 0, 3), 3);
        // 已有 2 个在连接中，只需再补 1 个
        assert_eq!(overclock_needed(8, 4, 10, 8, 2, 3), 1);
        // 超频上限：最多 常规 + 一半
        assert_eq!(overclock_needed(8, 4, 12, 4, 0, 4), 0);
        assert_eq!(overclock_needed(8, 4, 8, 2, 0, 0), 4);
        // 初始阶段全部在连接中，不超频
        assert_eq!(overclock_needed(8, 4, 8, 0, 8, 0), 0);
        // 单连接任务没有超频名额
        assert_eq!(overclock_needed(1, overclock_limit(1), 1, 1, 0, 1), 0);
    }

    #[test]
    fn finishing_uses_latency() {
        let mb = 1024 * 1024;
        let rt = rt_with(&[(0, mb - 1, mb - 100_000, true), (mb, 2 * mb - 1, mb, true)]);
        {
            let segs = rt.segs.lock();
            segs[0].rate.store(1_000_000, Ordering::Release); // 剩 100KB，约 100ms 传完
            segs[1].rate.store(1_000_000, Ordering::Release); // 剩 1MB，约 1s
        }
        assert_eq!(rt.finishing(50), 1);
        assert_eq!(rt.finishing(2000), 2);
    }

    #[test]
    fn retire_only_above_target() {
        let rt = ActiveRt::new();
        rt.target.store(4, Ordering::Release);
        rt.workers.store(6, Ordering::Release);
        assert_eq!(rt.boost(), 2);
        assert!(rt.retire());
        assert!(rt.retire());
        assert!(!rt.retire());
        assert_eq!(rt.workers.load(Ordering::Acquire), 4);
    }

    #[test]
    fn no_split_below_threshold() {
        let rt = rt_with(&[(0, 999_999, 999_999 - 300 * 1024, true)]);
        assert!(rt.take_segment().is_none());
    }
}
