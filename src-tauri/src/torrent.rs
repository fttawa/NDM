//! BitTorrent queue metadata and owned paths; all protocols use native libtorrent.
use crate::{
    engine::{ActiveRt, DlError},
    manager::Manager,
    util,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};

pub const MAX_META: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub info_hash: String,
    #[serde(default = "version_one")]
    pub version: u8,
    /// Cached metainfo is stored with the task in SQLite and backups.
    #[serde(default)]
    pub metainfo: Vec<u8>,
    #[serde(default)]
    pub files: Vec<String>,
}

fn version_one() -> u8 {
    1
}

impl Source {
    pub fn magnet_url(&self) -> String {
        if self.version == 2 {
            format!("magnet:?xt=urn:btmh:1220{}", self.info_hash)
        } else {
            format!("magnet:?xt=urn:btih:{}", self.info_hash)
        }
    }
}

pub fn magnet_source(url: &str) -> Result<(Source, String), String> {
    let url = reqwest::Url::parse(url).map_err(|e| format!("磁力链接无效：{e}"))?;
    if url.scheme() != "magnet" {
        return Err("磁力链接协议无效".into());
    }
    let mut v1 = None;
    let mut v2 = None;
    let mut name = None;
    for (key, value) in url.query_pairs() {
        if key == "dn" {
            name = Some(value.into_owned());
            continue;
        }
        if key != "xt" {
            continue;
        }
        let value = value.to_ascii_lowercase();
        if let Some(hash) = value.strip_prefix("urn:btih:") {
            if hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                v1 = Some(hash.to_string());
            } else if hash.len() == 32 {
                let mut bytes = Vec::with_capacity(20);
                let mut bits = 0u32;
                let mut count = 0;
                for c in hash.bytes() {
                    let n = match c {
                        b'a'..=b'z' => c - b'a',
                        b'2'..=b'7' => c - b'2' + 26,
                        _ => return Err("磁力链接无效：BTv1 Base32 哈希错误".into()),
                    };
                    bits = (bits << 5) | n as u32;
                    count += 5;
                    if count >= 8 {
                        count -= 8;
                        bytes.push((bits >> count) as u8);
                    }
                }
                v1 = Some(bytes.iter().map(|b| format!("{b:02x}")).collect());
            } else {
                return Err("磁力链接无效：BTv1 哈希错误".into());
            }
        } else if let Some(hash) = value.strip_prefix("urn:btmh:1220") {
            if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("磁力链接无效：BTv2 哈希错误".into());
            }
            v2 = Some(hash.to_string());
        }
    }
    let (hash, version) = v1
        .map(|h| (h, 1))
        .or_else(|| v2.map(|h| (h, 2)))
        .ok_or("磁力链接无效：缺少支持的 BT 哈希")?;
    let name = util::sanitize(name.as_deref().unwrap_or(&format!("BT-{hash}")));
    Ok((
        Source {
            info_hash: hash,
            version,
            metainfo: vec![],
            files: vec![],
        },
        name,
    ))
}

pub fn parse_metainfo(data: &[u8]) -> Result<(Source, String, u64), String> {
    crate::libtorrent::inspect(data)
}
pub(crate) fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("种子文件路径为空".into());
    }
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err("种子包含不安全的文件路径".into());
        };
        let part = part.to_str().ok_or("种子文件名不是有效文本")?;
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || util::sanitize(part) != part
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("种子包含不安全或 Windows 不支持的文件名：{part}"));
        }
    }
    Ok(())
}

/// Revalidate untrusted imported metainfo; never trust a serialized file list.
pub fn validate_source(source: &Source, url: &str) -> Result<(), String> {
    let expected = magnet_source(url)?.0;
    if expected.info_hash != source.info_hash || expected.version != source.version {
        return Err("BT 任务哈希不匹配".into());
    }
    if !source.metainfo.is_empty() {
        let cached = crate::libtorrent::inspect_for(&source.metainfo, source.version)?.0;
        if cached.info_hash != expected.info_hash || cached.version != expected.version {
            return Err("BT 种子与任务哈希不匹配".into());
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Engine {
    rate: std::sync::atomic::AtomicU64,
}
impl Engine {
    pub fn limit(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }
    pub fn set_limit(&self, bps: u64) {
        self.rate.store(bps, Ordering::Relaxed);
    }
}
pub async fn run(mgr: &Arc<Manager>, id: &str, rt: &Arc<ActiveRt>) -> Result<(), DlError> {
    crate::libtorrent::run(mgr, id, rt).await
}
pub(crate) fn validate_output(root: &Path, files: &[String]) -> Result<(), String> {
    // Reject symlinks/junctions in the task directory and all existing content components.
    for relative in std::iter::once(PathBuf::new()).chain(files.iter().map(PathBuf::from)) {
        let mut path = root.to_path_buf();
        for component in std::iter::once(None).chain(relative.components().map(Some)) {
            if let Some(component) = component {
                path.push(component);
            }
            match std::fs::symlink_metadata(&path) {
                Ok(meta) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if meta.file_attributes() & 0x400 != 0 {
                            return Err("BT 下载路径包含链接或目录联接".into());
                        }
                    }
                    if meta.file_type().is_symlink() {
                        return Err("BT 下载路径包含符号链接".into());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    Ok(())
}

pub(crate) fn prepare_output(root: &Path, hash: &str) -> Result<(), String> {
    use std::io::{Read, Write};
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let marker = root.join(".ndm-torrent");
    match std::fs::create_dir(root) {
        Ok(()) => {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)
                .map_err(|e| e.to_string())?;
            file.write_all(hash.as_bytes()).map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_output(root, &[".ndm-torrent".into()])?;
            let mut saved_hash = String::new();
            std::fs::File::open(marker)
                .map_err(|_| "任务文件夹已存在且不属于此 BT 任务，请更换保存位置或重新下载")?
                .take(65)
                .read_to_string(&mut saved_hash)
                .map_err(|e| e.to_string())?;
            if saved_hash != hash {
                return Err("任务文件夹属于另一个 BT 任务，已阻止覆盖".into());
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

/// Delete only validated torrent-owned files, never recurse through an arbitrary task folder.
pub fn delete_files(save_dir: &str, name: &str, source: &Source) -> Result<(), String> {
    if source.metainfo.is_empty() {
        return Ok(());
    }
    let validated = parse_metainfo(&source.metainfo)?.0;
    let root = Path::new(save_dir).join(name);
    if !root.exists() {
        return Ok(());
    }
    prepare_output(&root, &source.info_hash)?;
    validate_output(&root, &validated.files)?;
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    for file in validated.files {
        let path = root.join(file);
        if !path.exists() {
            continue;
        }
        if !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&canonical)
        {
            return Err("BT 文件路径越过下载目录".into());
        }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir == root || !dir.starts_with(&root) {
                break;
            }
            let _ = std::fs::remove_dir(dir);
            parent = dir.parent();
        }
    }
    let _ = std::fs::remove_file(root.join(".ndm-torrent"));
    let _ = std::fs::remove_dir(root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ndm-bt-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn validates_magnets_and_windows_paths() {
        let hash = "0123456789012345678901234567890123456789";
        let (source, name) =
            magnet_source(&format!("magnet:?xt=urn:btih:{hash}&dn=Demo%20Video")).unwrap();
        assert_eq!(source.info_hash, hash);
        assert_eq!(name, "Demo Video");
        assert!(magnet_source("magnet:?dn=missing").is_err());
        assert!(magnet_source("magnet:?xt=urn:btih:bad").is_err());
        let base32 = magnet_source("magnet:?xt=urn:btih:AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQT")
            .unwrap()
            .0;
        assert_eq!(base32.info_hash, "000102030405060708090a0b0c0d0e0f10111213");
        assert!(magnet_source(
            "https://example.test/?xt=urn:btih:0123456789012345678901234567890123456789"
        )
        .is_err());
        assert!(parse_metainfo(b"not a torrent").is_err());
        for path in [
            "../outside",
            "/absolute",
            "C:\\outside",
            "AUX.txt",
            "sub/CON",
            "file.",
            "movie:stream",
        ] {
            assert!(validate_relative(Path::new(path)).is_err(), "{path}");
        }
        assert!(validate_relative(Path::new("sub/电影.mkv")).is_ok());
    }

    #[test]
    fn output_ownership_blocks_existing_unrelated_content() {
        let temp = Temp::new();
        let task = temp.0.join("task");
        let hash = "0".repeat(40);
        prepare_output(&task, &hash).unwrap();
        prepare_output(&task, &hash).unwrap();
        assert!(prepare_output(&task, &"1".repeat(40)).is_err());
        let other = temp.0.join("user-folder");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("keep.txt"), b"keep").unwrap();
        assert!(prepare_output(&other, &hash).is_err());
        assert_eq!(std::fs::read(other.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn v2_magnets_and_old_sources_select_the_correct_engine() {
        let hash = "0123456789abcdef".repeat(4);
        let (source, name) =
            magnet_source(&format!("magnet:?xt=urn:btmh:1220{hash}&dn=BTv2%20sample")).unwrap();
        assert_eq!(source.info_hash, hash);
        assert_eq!(source.version, 2);
        assert_eq!(name, "BTv2 sample");
        validate_source(&source, &source.magnet_url()).unwrap();
        let mut wrong = source.clone();
        wrong.version = 1;
        assert!(validate_source(&wrong, &source.magnet_url()).is_err());
        assert!(magnet_source("magnet:?xt=urn:btmh:1220abcd").is_err());
        let hybrid = magnet_source(&format!(
            "{}&xt=urn:btih:{}",
            source.magnet_url(),
            "1".repeat(40)
        ))
        .unwrap()
        .0;
        assert_eq!(hybrid.version, 1);
        let old: Source = serde_json::from_value(serde_json::json!({
            "infoHash": "1".repeat(40)
        }))
        .unwrap();
        assert_eq!(old.version, 1);
        validate_source(&old, &old.magnet_url()).unwrap();
    }

    #[test]
    fn v2_metainfo_roundtrip_ownership_and_scoped_deletion() {
        let data = include_bytes!("../tests/fixtures/bt/v2.torrent");
        let (source, name, total) = parse_metainfo(data).unwrap();
        assert_eq!(source.version, 2);
        assert_eq!(source.info_hash.len(), 64);
        assert_eq!(name, "sample");
        assert_eq!(total, 70010);
        assert_eq!(source.files, ["sample/a.bin", "sample/b.bin"]);
        validate_source(&source, &source.magnet_url()).unwrap();
        let saved = serde_json::to_vec(&source).unwrap();
        let restored: Source = serde_json::from_slice(&saved).unwrap();
        validate_source(&restored, &source.magnet_url()).unwrap();
        assert_eq!(restored.metainfo, data);
        let mut wrong = source.clone();
        wrong.metainfo = include_bytes!("../tests/fixtures/bt/hybrid.torrent").to_vec();
        assert!(validate_source(&wrong, &source.magnet_url()).is_err());

        let temp = Temp::new();
        let root = temp.0.join("download");
        prepare_output(&root, &source.info_hash).unwrap();
        prepare_output(&root, &source.info_hash).unwrap();
        assert!(prepare_output(&root, &"0".repeat(64)).is_err());
        std::fs::create_dir_all(root.join("sample")).unwrap();
        std::fs::write(root.join("sample/a.bin"), b"partial").unwrap();
        std::fs::write(root.join("sample/b.bin"), b"partial").unwrap();
        std::fs::write(root.join("keep.txt"), b"user content").unwrap();
        delete_files(temp.0.to_str().unwrap(), "download", &restored).unwrap();
        assert!(!root.join("sample").exists());
        assert_eq!(
            std::fs::read(root.join("keep.txt")).unwrap(),
            b"user content"
        );
    }

    #[test]
    fn hybrid_metainfo_keeps_v1_identity_and_excludes_padding() {
        let (source, _, total) =
            parse_metainfo(include_bytes!("../tests/fixtures/bt/hybrid.torrent")).unwrap();
        assert_eq!(source.version, 1);
        assert_eq!(source.info_hash.len(), 40);
        assert_eq!(total, 70010); // Pad files are excluded from content totals.
        validate_source(&source, &source.magnet_url()).unwrap();
    }
}
