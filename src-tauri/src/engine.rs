//! 下载引擎：探测、多连接分段下载、动态分段、单流回退

use crate::manager::Manager;
use crate::model::Segment;
use crate::util;
use futures_util::StreamExt;
use parking_lot::Mutex;
use reqwest::header::{ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::StatusCode;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// 分段最小尺寸
const MIN_SEGMENT: u64 = 1024 * 1024;
/// 动态拆分时每一半的最小尺寸：剩余 ≥ 2 × MIN_SPLIT 的分段才会被拆分，
/// 粒度足够细，空闲连接才能接手小文件和尾段
const MIN_SPLIT: u64 = 256 * 1024;
const MAX_RETRIES: u32 = 5;

#[derive(Debug)]
pub enum DlError {
    Cancelled,
    /// 服务器不支持 Range 请求
    NoRange,
    Other(String),
}

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DlError::Cancelled => write!(f, "已取消"),
            DlError::NoRange => write!(f, "服务器不支持分段下载"),
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
}

impl SegRt {
    fn new(start: u64, end: u64, pos: u64, assigned: bool) -> Self {
        Self {
            start,
            end: AtomicU64::new(end),
            pos: AtomicU64::new(pos),
            assigned: AtomicBool::new(assigned),
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
    pub conns: AtomicU32,
}

impl ActiveRt {
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            segs: Mutex::new(Vec::new()),
            segmented: AtomicBool::new(false),
            single: AtomicU64::new(0),
            conns: AtomicU32::new(0),
        }
    }

    pub fn downloaded(&self) -> u64 {
        if self.segmented.load(Ordering::Acquire) {
            self.segs
                .lock()
                .iter()
                .map(|s| {
                    let end = s.end.load(Ordering::Acquire) + 1;
                    s.pos.load(Ordering::Acquire).min(end).saturating_sub(s.start)
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

/// 下载源：URL 与附加请求头（来自浏览器的 Cookie、Referer、User-Agent 等）
#[derive(Clone)]
struct Source {
    url: String,
    headers: Arc<Vec<(String, String)>>,
}

impl Source {
    fn from_task(task: &crate::model::Task) -> Self {
        Self { url: task.url.clone(), headers: Arc::new(task.headers.clone()) }
    }

    fn get(&self, client: &reqwest::Client) -> reqwest::RequestBuilder {
        let mut req = client.get(&self.url);
        for (k, v) in self.headers.iter() {
            // 非法的头名称/值直接跳过，不影响下载
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                req = req.header(name, value);
            }
        }
        req
    }
}

struct ProbeInfo {
    total: Option<u64>,
    resumable: bool,
    file_name: Option<String>,
    mime: Option<String>,
}

async fn probe(client: &reqwest::Client, src: &Source, cancel: &CancellationToken) -> Result<ProbeInfo, DlError> {
    let req = src.get(client).header(RANGE, "bytes=0-").send();
    let resp = tokio::select! {
        r = req => r?,
        _ = cancel.cancelled() => return Err(DlError::Cancelled),
    };
    let status = resp.status();
    if !status.is_success() {
        return Err(DlError::Other(format!("服务器返回 HTTP {}", status.as_u16())));
    }
    let headers = resp.headers();
    let header_str = |k| headers.get(k).map(|v: &reqwest::header::HeaderValue| String::from_utf8_lossy(v.as_bytes()).to_string());

    let (total, resumable) = if status == StatusCode::PARTIAL_CONTENT {
        // Content-Range: bytes 0-999/1000
        let total = header_str(CONTENT_RANGE)
            .and_then(|v| v.rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()));
        (total, total.is_some())
    } else {
        let accept = header_str(ACCEPT_RANGES).map(|v| v.eq_ignore_ascii_case("bytes")).unwrap_or(false);
        let total = resp.content_length();
        (total, accept && total.is_some())
    };

    let file_name = header_str(CONTENT_DISPOSITION)
        .and_then(|v| util::filename_from_disposition(&v))
        .or_else(|| util::filename_from_url(resp.url().as_str()));
    let mime = header_str(CONTENT_TYPE).map(|v| v.split(';').next().unwrap_or("").trim().to_string());
    Ok(ProbeInfo { total, resumable, file_name, mime })
}

fn plan_segments(total: u64, conns: u32) -> Vec<Segment> {
    let n = (total / MIN_SEGMENT).clamp(1, conns.max(1) as u64);
    let size = total / n;
    (0..n)
        .map(|i| {
            let start = i * size;
            let end = if i == n - 1 { total - 1 } else { start + size - 1 };
            Segment { start, end, pos: start }
        })
        .collect()
}

/// 下载入口：分段模式失败于 NoRange 时自动降级为单连接
pub async fn run(mgr: Arc<Manager>, id: String, rt: Arc<ActiveRt>) -> Result<(), DlError> {
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
    let dir = PathBuf::from(&task.save_dir);
    std::fs::create_dir_all(&dir)?;

    // 没有可续传的进度时重新探测
    if !(task.resumable && !task.segments.is_empty()) {
        let info = probe(&client, &Source::from_task(&task), &rt.cancel).await?;
        if !task.name_locked {
            let raw = info.file_name.clone().unwrap_or_else(|| task.file_name.clone());
            let clean = util::sanitize(&raw);
            task.file_name = util::unique_name(&dir, &clean, |n| mgr.name_taken(&task.save_dir, n, id));
            task.name_locked = true;
        }
        task.total = info.total;
        task.mime = info.mime;
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

fn finalize(mgr: &Arc<Manager>, id: &str, dir: &Path, part: &Path, name: &str) -> Result<(), DlError> {
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
    let file = OpenOptions::new().read(true).write(true).create(true).open(part)?;
    if file.metadata()?.len() != total {
        file.set_len(total)?;
    }
    let file = Arc::new(file);

    // .part 文件丢失时进度作废，从头开始
    let had_progress = task.segments.iter().any(|s| s.pos > s.start);
    let segments = if !existed && had_progress {
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
    let mut set = JoinSet::new();
    for _ in 0..task.connections.max(1) {
        set.spawn(worker(
            mgr.clone(),
            src.clone(),
            file.clone(),
            rt.clone(),
            worker_cancel.clone(),
        ));
    }

    let mut first_err: Option<DlError> = None;
    let mut no_range = false;
    while let Some(res) = set.join_next().await {
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
    loop {
        if cancel.is_cancelled() {
            return Err(DlError::Cancelled);
        }
        let Some(seg) = rt.take_segment() else {
            return Ok(());
        };
        let mut attempt = 0u32;
        loop {
            let res = {
                let _guard = ConnGuard::new(&rt.conns);
                fetch_segment(&mgr, &client, &src, &file, &seg, &cancel).await
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
    mgr: &Arc<Manager>,
    client: &reqwest::Client,
    src: &Source,
    file: &File,
    seg: &SegRt,
    cancel: &CancellationToken,
) -> Result<(), DlError> {
    let pos = seg.pos.load(Ordering::Acquire);
    let end = seg.end.load(Ordering::Acquire);
    if pos > end {
        return Ok(());
    }
    let req = src.get(client).header(RANGE, format!("bytes={pos}-{end}")).send();
    let resp = tokio::select! {
        r = req => r?,
        _ = cancel.cancelled() => return Err(DlError::Cancelled),
    };
    let status = resp.status();
    if status == StatusCode::OK {
        if pos != 0 {
            return Err(DlError::NoRange);
        }
    } else if status != StatusCode::PARTIAL_CONTENT {
        return Err(DlError::Other(format!("服务器返回 HTTP {}", status.as_u16())));
    }

    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => return Err(DlError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk?;
        let wait = mgr.limiter.reserve(chunk.len());
        if !wait.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = cancel.cancelled() => return Err(DlError::Cancelled),
            }
        }
        let pos = seg.pos.load(Ordering::Acquire);
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
        return Err(DlError::Other(format!("服务器返回 HTTP {}", resp.status().as_u16())));
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

    fn rt_with(segs: &[(u64, u64, u64, bool)]) -> ActiveRt {
        let rt = ActiveRt::new();
        *rt.segs.lock() = segs.iter().map(|&(s, e, p, a)| Arc::new(SegRt::new(s, e, p, a))).collect();
        rt.segmented.store(true, Ordering::Release);
        rt
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
        let rt = rt_with(&[(0, 3 * mb - 1, 3 * mb, true), (3 * mb, 6 * mb - 1, 5 * mb, true)]);
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
    fn no_split_below_threshold() {
        let rt = rt_with(&[(0, 999_999, 999_999 - 300 * 1024, true)]);
        assert!(rt.take_segment().is_none());
    }
}
