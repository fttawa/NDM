//! HLS VOD download: resolve playlists, validate the complete plan, stream and join pieces.
use crate::{
    engine::{ActiveRt, DlError},
    limiter::Limiter,
    manager::Manager,
};
use futures_util::StreamExt;
use m3u8_rs::{KeyMethod, MediaPlaylist, Playlist};
use reqwest::{Client, Url};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const PLAYLIST_LIMIT: usize = 8 * 1024 * 1024;
const PIECE_LIMIT: usize = 100_000;

fn invalid(message: &str) -> DlError {
    DlError::Other(message.into())
}

pub fn is_hls_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| url.path().to_ascii_lowercase().ends_with(".m3u8"))
}

fn http_url(url: &str) -> Result<Url, DlError> {
    let url = Url::parse(url).map_err(|_| invalid("HLS 链接无效"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("HLS 资源只支持 HTTP/HTTPS"));
    }
    Ok(url)
}

fn join(base: &Url, uri: &str) -> Result<Url, DlError> {
    let url = base.join(uri).map_err(|_| invalid("HLS 资源地址无效"))?;
    http_url(url.as_str())
}

// Cookies and authorization belong to the captured origin, not every referenced CDN.
fn request(
    client: &Client,
    url: &Url,
    source: &Url,
    headers: &[(String, String)],
) -> reqwest::RequestBuilder {
    let mut request = client.get(url.clone()).timeout(Duration::from_secs(120));
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if !matches!(
            lower.as_str(),
            "user-agent" | "referer" | "origin" | "cookie" | "authorization"
        ) {
            continue;
        }
        if matches!(lower.as_str(), "cookie" | "authorization" | "origin")
            && source.origin() != url.origin()
        {
            continue;
        }
        request = request.header(name.as_str(), value.as_str());
    }
    request
}

fn parse(data: &[u8]) -> Result<Playlist, DlError> {
    let text = std::str::from_utf8(data).map_err(|_| invalid("HLS 播放列表不是 UTF-8"))?;
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.lines().next() != Some("#EXTM3U") {
        return Err(invalid("返回的内容不是 m3u8 播放列表"));
    }
    if text
        .lines()
        .any(|line| line.starts_with("#EXT-X-SESSION-KEY:"))
    {
        return Err(invalid("暂不支持包含会话密钥的加密 HLS"));
    }
    // Reject partial/low latency streams and gaps rather than silently omitting media.
    if text.lines().any(|line| {
        line.starts_with("#EXT-X-GAP")
            || line.starts_with("#EXT-X-PART:")
            || line.starts_with("#EXT-X-SKIP:")
    }) {
        return Err(invalid("暂不支持含缺失分片或低延迟分片的 HLS"));
    }
    let (remaining, playlist) =
        m3u8_rs::parse_playlist(text.as_bytes()).map_err(|_| invalid("m3u8 播放列表格式无效"))?;
    if remaining.iter().any(|byte| !byte.is_ascii_whitespace()) {
        return Err(invalid("m3u8 播放列表不完整或格式无效"));
    }
    Ok(playlist)
}

async fn fetch_playlist(
    client: &Client,
    url: &Url,
    source: &Url,
    headers: &[(String, String)],
    cancel: &CancellationToken,
) -> Result<(Url, Playlist), DlError> {
    let response = tokio::select! {
        response = request(client, url, source, headers).send() => response?,
        _ = cancel.cancelled() => return Err(DlError::Cancelled),
    };
    if !response.status().is_success() {
        return Err(invalid(&format!(
            "HLS 播放列表返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > PLAYLIST_LIMIT as u64)
    {
        return Err(invalid("HLS 播放列表过大"));
    }
    let base = response.url().clone();
    let mut stream = response.bytes_stream();
    let mut data = Vec::new();
    loop {
        let chunk = tokio::select! { chunk = stream.next() => chunk, _ = cancel.cancelled() => return Err(DlError::Cancelled) };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        if data.len().saturating_add(chunk.len()) > PLAYLIST_LIMIT {
            return Err(invalid("HLS 播放列表过大"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok((base, parse(&data)?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub url: String,
    pub label: String,
    pub supported: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub variants: Vec<Variant>,
    pub segments: usize,
    pub duration: f64,
    pub format: Option<String>,
}

fn variants(base: &Url, master: &m3u8_rs::MasterPlaylist) -> Result<Vec<Variant>, DlError> {
    let mut streams: Vec<_> = master
        .variants
        .iter()
        .filter(|variant| !variant.is_i_frame)
        .collect();
    streams.sort_by_key(|variant| std::cmp::Reverse(variant.bandwidth));
    if streams.is_empty() {
        return Err(invalid("HLS 主播放列表没有可用清晰度"));
    }
    streams
        .into_iter()
        .map(|variant| {
            let separate = [&variant.audio, &variant.video].iter().any(|group| {
                group.as_ref().is_some_and(|group| {
                    master
                        .alternatives
                        .iter()
                        .any(|media| &media.group_id == group && media.uri.is_some())
                })
            });
            let resolution = variant
                .resolution
                .as_ref()
                .map(|res| format!("{}×{} · ", res.width, res.height))
                .unwrap_or_default();
            Ok(Variant {
                url: join(base, &variant.uri)?.into(),
                label: format!(
                    "{resolution}{} kbps{}",
                    variant.bandwidth / 1000,
                    if separate {
                        "（独立音视频轨，暂不支持）"
                    } else {
                        ""
                    }
                ),
                supported: !separate,
            })
        })
        .collect()
}

pub async fn inspect(
    client: &Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<Info, DlError> {
    let source = http_url(url)?;
    let (base, playlist) =
        fetch_playlist(client, &source, &source, headers, &CancellationToken::new()).await?;
    match playlist {
        Playlist::MasterPlaylist(master) => Ok(Info {
            variants: variants(&base, &master)?,
            segments: 0,
            duration: 0.0,
            format: None,
        }),
        Playlist::MediaPlaylist(media) => {
            let plan = plan(&base, &media)?;
            Ok(Info {
                variants: vec![],
                segments: media.segments.len(),
                duration: media.segments.iter().map(|seg| seg.duration as f64).sum(),
                format: Some(plan.extension.into()),
            })
        }
    }
}

#[derive(Debug)]
struct Piece {
    url: Url,
    range: Option<(u64, u64)>,
}
#[derive(Debug)]
struct Plan {
    pieces: Vec<Piece>,
    extension: &'static str,
}

fn piece(
    base: &Url,
    uri: &str,
    range: Option<&m3u8_rs::ByteRange>,
    previous: &mut Option<(Url, u64)>,
) -> Result<Piece, DlError> {
    let url = join(base, uri)?;
    let range = if let Some(range) = range {
        let offset = range
            .offset
            .or_else(|| {
                previous
                    .as_ref()
                    .filter(|(prev, _)| prev == &url)
                    .map(|(_, end)| *end)
            })
            .ok_or_else(|| invalid("HLS 字节范围缺少起点"))?;
        let end = offset
            .checked_add(range.length)
            .filter(|end| *end > offset)
            .ok_or_else(|| invalid("HLS 字节范围无效"))?;
        *previous = Some((url.clone(), end));
        Some((offset, end - 1))
    } else {
        *previous = None;
        None
    };
    Ok(Piece { url, range })
}

fn plan(base: &Url, media: &MediaPlaylist) -> Result<Plan, DlError> {
    if !media.end_list {
        return Err(invalid(
            "暂不支持直播或尚未结束的 HLS，请使用已结束的点播流",
        ));
    }
    if media.i_frames_only || media.segments.is_empty() || media.segments.len() > PIECE_LIMIT {
        return Err(invalid("HLS 分片列表为空、过大或仅含关键帧"));
    }
    let mut pieces = Vec::new();
    let mut previous = None;
    let mut map = None;
    let mut encrypted = false;
    for segment in &media.segments {
        if let Some(key) = &segment.key {
            encrypted = key.method != KeyMethod::None;
        }
        if encrypted {
            return Err(invalid("暂不支持加密 HLS（AES-128 / SAMPLE-AES / DRM）"));
        }
        if segment.discontinuity {
            return Err(invalid("暂不支持包含音视频时间线切换的 HLS"));
        }
        if !segment.duration.is_finite() || segment.duration <= 0.0 {
            return Err(invalid("HLS 分片时长无效"));
        }
        if let Some(next_map) = &segment.map {
            if map.as_ref().is_some_and(|map| map != next_map) {
                return Err(invalid("暂不支持初始化分片切换的 HLS"));
            }
            if map.is_none() {
                if !pieces.is_empty() {
                    return Err(invalid("暂不支持在媒体分片中切换容器格式的 HLS"));
                }
                pieces.push(piece(
                    base,
                    &next_map.uri,
                    next_map.byte_range.as_ref(),
                    &mut None,
                )?);
                map = Some(next_map.clone());
            }
        }
        pieces.push(piece(
            base,
            &segment.uri,
            segment.byte_range.as_ref(),
            &mut previous,
        )?);
    }
    Ok(Plan {
        pieces,
        extension: if map.is_some() { "mp4" } else { "ts" },
    })
}

async fn resolve(
    client: &Client,
    source: &Url,
    headers: &[(String, String)],
    cancel: &CancellationToken,
) -> Result<Plan, DlError> {
    let mut url = source.clone();
    let mut visited = std::collections::HashSet::new();
    for _ in 0..5 {
        if !visited.insert(url.to_string()) {
            return Err(invalid("HLS 播放列表循环引用"));
        }
        let (base, playlist) = fetch_playlist(client, &url, source, headers, cancel).await?;
        match playlist {
            Playlist::MasterPlaylist(master) => {
                let variant = variants(&base, &master)?
                    .into_iter()
                    .find(|variant| variant.supported)
                    .ok_or_else(|| invalid("没有可直接合并的 HLS 清晰度，独立音视频轨暂不支持"))?;
                url = http_url(&variant.url)?;
            }
            Playlist::MediaPlaylist(media) => return plan(&base, &media),
        }
    }
    Err(invalid("HLS 播放列表嵌套过深"))
}

async fn transfer(
    client: &Client,
    source: &Url,
    headers: &[(String, String)],
    piece: &Piece,
    file: &mut File,
    rt: &ActiveRt,
    limiter: &Limiter,
) -> Result<u64, DlError> {
    let mut request =
        request(client, &piece.url, source, headers).header("Accept-Encoding", "identity");
    if let Some((start, end)) = piece.range {
        request = request.header("Range", format!("bytes={start}-{end}"));
    }
    let response = tokio::select! { response = request.send() => response?, _ = rt.cancel.cancelled() => return Err(DlError::Cancelled) };
    if !response.status().is_success() {
        return Err(invalid(&format!(
            "HLS 分片返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    if let Some((start, end)) = piece.range {
        let expected = format!("bytes {start}-{end}/");
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || !response
                .headers()
                .get("Content-Range")
                .and_then(|header| header.to_str().ok())
                .is_some_and(|header| header.starts_with(&expected))
        {
            return Err(invalid("服务器未按 HLS 字节范围返回分片"));
        }
    } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(invalid("HLS 分片意外返回部分内容"));
    }
    let expected = piece
        .range
        .map(|(start, end)| end - start + 1)
        .or(response.content_length());
    let mut stream = response.bytes_stream();
    let mut written = 0u64;
    loop {
        let chunk = tokio::select! { chunk = stream.next() => chunk, _ = rt.cancel.cancelled() => return Err(DlError::Cancelled) };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        if expected.is_some_and(|expected| written.saturating_add(chunk.len() as u64) > expected) {
            return Err(invalid("HLS 分片长度不符"));
        }
        let wait = limiter.reserve(chunk.len());
        tokio::select! { _ = tokio::time::sleep(wait) => {}, _ = rt.cancel.cancelled() => return Err(DlError::Cancelled) }
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
        rt.single.fetch_add(chunk.len() as u64, Ordering::AcqRel);
    }
    if written == 0 || expected.is_some_and(|expected| expected != written) {
        return Err(invalid("HLS 分片为空或传输不完整"));
    }
    Ok(written)
}

async fn download(
    client: &Client,
    source: &Url,
    headers: &[(String, String)],
    plan: &Plan,
    part: &Path,
    rt: &ActiveRt,
    limiter: &Limiter,
) -> Result<(), DlError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)?;
    rt.single.store(0, Ordering::Release);
    for (index, piece) in plan.pieces.iter().enumerate() {
        let checkpoint = file.stream_position()?;
        let mut success = false;
        for attempt in 0..3 {
            if rt.cancel.is_cancelled() {
                return Err(DlError::Cancelled);
            }
            rt.conns.store(1, Ordering::Release);
            let result = transfer(client, source, headers, piece, &mut file, rt, limiter)
                .await
                .and_then(|length| {
                    validate_piece(&mut file, checkpoint, length, plan.extension, index == 0)?;
                    Ok(length)
                });
            rt.conns.store(0, Ordering::Release);
            match result {
                Ok(_) => {
                    success = true;
                    break;
                }
                Err(error) => {
                    file.set_len(checkpoint)?;
                    file.seek(SeekFrom::Start(checkpoint))?;
                    rt.single.store(checkpoint, Ordering::Release);
                    if matches!(error, DlError::Cancelled) || attempt == 2 {
                        return Err(error);
                    }
                    tokio::select! { _ = tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))) => {}, _ = rt.cancel.cancelled() => return Err(DlError::Cancelled) }
                }
            }
        }
        if !success {
            return Err(invalid("HLS 分片下载失败"));
        }
    }
    file.sync_all()?;
    Ok(())
}

fn validate_piece(
    file: &mut File,
    offset: u64,
    length: u64,
    extension: &str,
    first: bool,
) -> Result<(), DlError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut prefix = [0u8; 8];
    file.read_exact(&mut prefix)?;
    let valid = if extension == "ts" {
        prefix[0] == 0x47 && length.is_multiple_of(188)
    } else if first {
        matches!(&prefix[4..], b"ftyp" | b"moov")
    } else {
        matches!(
            &prefix[4..],
            b"styp" | b"sidx" | b"moof" | b"emsg" | b"prft"
        )
    };
    file.seek(SeekFrom::Start(offset + length))?;
    if !valid {
        return Err(invalid(
            "HLS 分片不是支持的 TS / 分片 MP4，或服务器返回了错误页面",
        ));
    }
    Ok(())
}

pub async fn run(mgr: &Arc<Manager>, id: &str, rt: &Arc<ActiveRt>) -> Result<(), DlError> {
    let task = mgr.task(id).ok_or(DlError::Cancelled)?;
    let source = http_url(&task.url)?;
    mgr.update_task(id, |task| {
        task.total = None;
        task.downloaded = 0;
        task.segments.clear();
        task.resumable = false;
    });
    mgr.emit_tasks();
    let client = mgr.client();
    let plan = resolve(&client, &source, &task.headers, &rt.cancel).await?;
    let dir = Path::new(&task.save_dir);
    std::fs::create_dir_all(dir)?;
    let stem = Path::new(&task.file_name)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("video");
    let name = crate::util::sanitize(&format!("{stem}.{}", plan.extension));
    let name = if task.file_name == name && task.name_locked {
        name
    } else {
        mgr.reserve_task_name(id, &task.save_dir, &name)
    };
    mgr.update_task(id, |task| {
        task.file_name = name.clone();
        task.name_locked = true;
        task.mime = Some(
            if plan.extension == "mp4" {
                "video/mp4"
            } else {
                "video/mp2t"
            }
            .into(),
        );
    });
    mgr.emit_tasks();
    download(
        &client,
        &source,
        &task.headers,
        &plan,
        &dir.join(format!("{name}.part")),
        rt,
        &mgr.limiter,
    )
    .await?;
    if rt.cancel.is_cancelled() || mgr.task(id).is_none() {
        return Err(DlError::Cancelled);
    }
    mgr.update_task(id, |task| task.total = Some(rt.downloaded()));
    crate::engine::finalize(mgr, id, dir, &dir.join(format!("{name}.part")), &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tiny_http::{Header, Request, Response, Server};

    struct TestServer {
        url: Url,
        stopped: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl TestServer {
        fn start(mut handler: impl FnMut(Request) + Send + 'static) -> Self {
            let server = Server::http("127.0.0.1:0").unwrap();
            let url = Url::parse(&format!("http://{}/", server.server_addr())).unwrap();
            let stopped = Arc::new(AtomicBool::new(false));
            let stop = stopped.clone();
            let thread = std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    if let Some(request) = server.recv_timeout(Duration::from_millis(50)).unwrap() {
                        handler(request);
                    }
                }
            });
            Self {
                url,
                stopped,
                thread: Some(thread),
            }
        }
    }
    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn client() -> Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONNECTION,
            reqwest::header::HeaderValue::from_static("close"),
        );
        Client::builder()
            .no_proxy()
            .default_headers(headers)
            .build()
            .unwrap()
    }
    fn media(text: &str) -> MediaPlaylist {
        match parse(text.as_bytes()).unwrap() {
            Playlist::MediaPlaylist(media) => media,
            _ => panic!("not media"),
        }
    }
    fn temporary_part() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ndm-hls-{}.part", uuid::Uuid::new_v4()))
    }
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/hls")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn validates_playlists_before_any_media_write() {
        let base = Url::parse("https://example.com/video/index.m3u8?token=abc").unwrap();
        let valid = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n";
        let plan = plan(&base, &media(valid)).unwrap();
        assert_eq!(
            plan.pieces[0].url.as_str(),
            "https://example.com/video/a.ts"
        );
        assert_eq!(plan.extension, "ts");
        assert!(super::plan(&base, &media(&valid.replace("#EXT-X-ENDLIST", ""))).is_err());
        for tag in [
            "#EXT-X-KEY:METHOD=AES-128,URI=\"key\"",
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key\"",
            "#EXT-X-DISCONTINUITY",
        ] {
            assert!(super::plan(
                &base,
                &media(&valid.replace("#EXTINF", &format!("{tag}\n#EXTINF")))
            )
            .is_err());
        }
        assert!(parse(b"<html>login required</html>").is_err());
        assert!(parse(b"#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"key\"\n").is_err());
        assert!(parse(valid.replace("#EXTINF", "#EXT-X-GAP\n#EXTINF").as_bytes()).is_err());
        assert!(super::plan(
            &base,
            &media(&valid.replace("a.ts", "file:///local/file.ts"))
        )
        .is_err());
    }

    #[test]
    fn byte_ranges_and_map_are_validated() {
        let base = Url::parse("https://example.com/index.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"100@0\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:20@100\nvideo.mp4\n#EXTINF:1,\n#EXT-X-BYTERANGE:30\nvideo.mp4\n#EXT-X-ENDLIST\n";
        let plan = plan(&base, &media(text)).unwrap();
        assert_eq!(plan.extension, "mp4");
        assert_eq!(plan.pieces.len(), 3);
        assert_eq!(plan.pieces[2].range, Some((120, 149)));
        assert!(super::plan(&base, &media(&text.replace("20@100", "20"))).is_err());
        assert!(super::plan(
            &base,
            &media(&text.replace("20@100", "20@18446744073709551615"))
        )
        .is_err());
    }

    #[test]
    fn sensitive_headers_stay_at_source_origin() {
        let source = Url::parse("https://source.test/master.m3u8").unwrap();
        let other = Url::parse("https://cdn.test/file.ts").unwrap();
        let headers = vec![
            ("Cookie".into(), "session=secret".into()),
            ("Authorization".into(), "Bearer secret".into()),
            ("Referer".into(), "https://page.test/".into()),
            ("Range".into(), "bytes=12-13".into()),
        ];
        let same = request(&client(), &source, &source, &headers)
            .build()
            .unwrap();
        assert!(same.headers().contains_key("Cookie"));
        let cross = request(&client(), &other, &source, &headers)
            .build()
            .unwrap();
        assert!(!cross.headers().contains_key("Cookie"));
        assert!(!cross.headers().contains_key("Authorization"));
        assert!(!cross.headers().contains_key("Range"));
        assert!(cross.headers().contains_key("Referer"));
    }

    #[tokio::test]
    async fn downloads_real_ts_and_fmp4_fixtures_in_order() {
        for format in ["ts", "mp4"] {
            let server = TestServer::start(|request| {
                let name = request.url().trim_start_matches('/').to_string();
                let _ = request.respond(Response::from_data(fixture(&name)));
            });
            let url = server.url.join(&format!("{format}.m3u8")).unwrap();
            let client = client();
            let rt = ActiveRt::new();
            let plan = resolve(&client, &url, &[], &rt.cancel).await.unwrap();
            let part = temporary_part();
            download(&client, &url, &[], &plan, &part, &rt, &Limiter::new(0))
                .await
                .unwrap();
            let mut expected = vec![];
            if format == "mp4" {
                expected.extend(fixture("init.mp4"));
            }
            for index in 0..3 {
                expected.extend(fixture(&format!(
                    "{format}-{index}.{}",
                    if format == "ts" { "ts" } else { "m4s" }
                )));
            }
            assert_eq!(std::fs::read(&part).unwrap(), expected);
            assert_eq!(rt.downloaded(), expected.len() as u64);
            if let Some(dir) = std::env::var_os("NDM_HLS_TEST_OUTPUT_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::copy(&part, Path::new(&dir).join(format!("joined.{format}"))).unwrap();
            }
            std::fs::remove_file(part).unwrap();
        }
    }

    #[tokio::test]
    async fn master_redirect_uses_final_base_and_skips_external_audio() {
        let server = TestServer::start(|request| {
            let response = match request.url() {
                "/start" => Response::from_string("").with_status_code(302).with_header(Header::from_bytes("Location", "/nested/master.m3u8").unwrap()),
                "/nested/master.m3u8" => Response::from_string("#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"en\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=900000,AUDIO=\"audio\"\nsilent.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360\nmedia.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=100000\nlow.m3u8\n"),
                "/nested/media.m3u8" => Response::from_data(fixture("ts.m3u8")),
                _ => Response::from_string("unexpected request").with_status_code(404),
            };
            let _ = request.respond(response);
        });
        let url = server.url.join("start").unwrap();
        let info = inspect(&client(), url.as_str(), &[]).await.unwrap();
        assert!(!info.variants[0].supported);
        assert!(info.variants[1].label.contains("640×360"));
        let plan = resolve(&client(), &url, &[], &CancellationToken::new())
            .await
            .unwrap();
        assert!(plan.pieces[0].url.path().ends_with("/nested/ts-0.ts"));
    }

    #[tokio::test]
    async fn retries_invalid_body_without_duplicating_output() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let server = TestServer::start(move |request| {
            let data = if count.fetch_add(1, Ordering::AcqRel) == 0 {
                b"<html>login required</html>".to_vec()
            } else {
                fixture("ts-0.ts")
            };
            let _ = request.respond(Response::from_data(data));
        });
        let url = server.url.join("a.ts").unwrap();
        let plan = Plan {
            pieces: vec![Piece {
                url: url.clone(),
                range: None,
            }],
            extension: "ts",
        };
        let part = temporary_part();
        let rt = ActiveRt::new();
        download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(0))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), fixture("ts-0.ts"));
        assert_eq!(rt.downloaded(), fixture("ts-0.ts").len() as u64);
        assert_eq!(attempts.load(Ordering::Acquire), 2);
        std::fs::remove_file(part).unwrap();
    }

    #[tokio::test]
    async fn range_response_is_required_and_cancellation_is_prompt() {
        let data = fixture("ts-0.ts");
        let server = TestServer::start(move |request| {
            let range = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Range"))
                .unwrap()
                .value
                .as_str();
            assert_eq!(range, "bytes=188-375");
            let response = Response::from_data(data[188..376].to_vec())
                .with_status_code(206)
                .with_header(
                    Header::from_bytes("Content-Range", format!("bytes 188-375/{}", data.len()))
                        .unwrap(),
                );
            let _ = request.respond(response);
        });
        let url = server.url.join("a.ts").unwrap();
        let plan = Plan {
            pieces: vec![Piece {
                url: url.clone(),
                range: Some((188, 375)),
            }],
            extension: "ts",
        };
        let part = temporary_part();
        let rt = ActiveRt::new();
        download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(0))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), fixture("ts-0.ts")[188..376]);
        rt.cancel.cancel();
        assert!(matches!(
            download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(0)).await,
            Err(DlError::Cancelled)
        ));
        std::fs::remove_file(part).unwrap();
    }

    #[tokio::test]
    async fn truncated_piece_is_retried_from_checkpoint() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let server = TestServer::start(move |request| {
            let bytes = fixture("ts-0.ts");
            let full_length = bytes.len();
            let data = if count.fetch_add(1, Ordering::AcqRel) == 0 {
                bytes[..376].to_vec()
            } else {
                bytes
            };
            let response = Response::new(
                tiny_http::StatusCode(200),
                vec![],
                std::io::Cursor::new(data),
                Some(full_length),
                None,
            );
            let _ = request.respond(response);
        });
        let url = server.url.join("part.ts").unwrap();
        let plan = Plan {
            pieces: vec![Piece {
                url: url.clone(),
                range: None,
            }],
            extension: "ts",
        };
        let part = temporary_part();
        let rt = ActiveRt::new();
        download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(0))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), fixture("ts-0.ts"));
        assert_eq!(attempts.load(Ordering::Acquire), 2);
        std::fs::remove_file(part).unwrap();
    }

    #[tokio::test]
    async fn server_ignoring_range_never_creates_successful_media() {
        let server = TestServer::start(|request| {
            let _ = request.respond(Response::from_data(fixture("ts-0.ts")));
        });
        let url = server.url.join("part.ts").unwrap();
        let plan = Plan {
            pieces: vec![Piece {
                url: url.clone(),
                range: Some((188, 375)),
            }],
            extension: "ts",
        };
        let part = temporary_part();
        let rt = ActiveRt::new();
        assert!(
            download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(0))
                .await
                .is_err()
        );
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 0);
        assert_eq!(rt.downloaded(), 0);
        std::fs::remove_file(part).unwrap();
    }

    #[tokio::test]
    async fn pause_interrupts_rate_limit_wait_without_a_partial_piece() {
        let server = TestServer::start(|request| {
            let _ = request.respond(Response::from_data(fixture("ts-0.ts")));
        });
        let url = server.url.join("part.ts").unwrap();
        let plan = Plan {
            pieces: vec![Piece {
                url: url.clone(),
                range: None,
            }],
            extension: "ts",
        };
        let part = temporary_part();
        let rt = ActiveRt::new();
        let token = rt.cancel.clone();
        let pause = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            download(&client(), &url, &[], &plan, &part, &rt, &Limiter::new(1)),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(DlError::Cancelled)));
        pause.await.unwrap();
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 0);
        assert_eq!(rt.conns.load(Ordering::Acquire), 0);
        std::fs::remove_file(part).unwrap();
    }
}
