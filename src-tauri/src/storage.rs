//! SQLite 持久化：任务与设置都存放在 data_dir/state.db。

use crate::model::{Settings, Task};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 2;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportBundle {
    pub version: i64,
    pub settings: Settings,
    pub tasks: Vec<Task>,
}

pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let storage = Self { path: path.into() };
        if let Err(first) = storage.initialize() {
            if !storage.path.exists() || !is_corruption(first.as_ref()) {
                return Err(first);
            }
            let backup = storage.quarantine_path();
            std::fs::rename(&storage.path, &backup)
                .map_err(|e| format!("数据库损坏且无法备份：{e}; 原因：{first}"))?;
            for suffix in ["-wal", "-shm"] {
                let sidecar =
                    PathBuf::from(format!("{}{}", storage.path.to_string_lossy(), suffix));
                if sidecar.exists() {
                    std::fs::rename(&sidecar, format!("{}{}", backup.to_string_lossy(), suffix))?;
                }
            }
            eprintln!("数据库已损坏，已备份到 {}", backup.display());
            storage.initialize()?;
        }
        Ok(storage)
    }

    fn initialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.connection()?;
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(11),
                Some(integrity),
            )
            .into());
        }
        let tx = conn.transaction()?;
        tx.execute_batch(
            "\
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY NOT NULL,
                payload TEXT NOT NULL
            );
            INSERT INTO meta(key, value) VALUES ('schema_version', '1')
                ON CONFLICT(key) DO NOTHING;
            ",
        )?;
        let mut version: i64 = tx
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()?;
        if !(1..=SCHEMA_VERSION).contains(&version) {
            return Err(format!("不支持的数据库版本：{version}").into());
        }
        if version < 2 {
            tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(json_extract(payload, '$.status'));
                 CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(json_extract(payload, '$.createdAt'));
                 UPDATE meta SET value = '2' WHERE key = 'schema_version';",
            )?;
            version = 2;
        }
        if version != SCHEMA_VERSION {
            return Err(format!("数据库迁移失败，当前版本：{version}").into());
        }
        tx.commit()?;
        Ok(())
    }

    fn quarantine_path(&self) -> PathBuf {
        let stamp = crate::model::now_ms();
        PathBuf::from(format!("{}.corrupt-{}", self.path.to_string_lossy(), stamp))
    }

    fn connection(&self) -> rusqlite::Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Ok(conn)
    }

    pub fn load_settings(&self) -> Result<Option<Settings>, Box<dyn std::error::Error>> {
        let conn = self.connection()?;
        let payload: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'settings'", [], |row| {
                row.get(0)
            })
            .optional()?;
        payload
            .map(|json| Ok(serde_json::from_str(&json)?))
            .transpose()
    }

    pub fn load_tasks(&self) -> Result<Option<Vec<Task>>, Box<dyn std::error::Error>> {
        let conn = self.connection()?;
        let initialized: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'tasks_initialized'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if initialized.is_none() {
            return Ok(None);
        }
        let mut stmt = conn.prepare("SELECT payload FROM tasks ORDER BY rowid")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(serde_json::from_str(&row?)?);
        }
        Ok(Some(tasks))
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.connection()?;
        let payload = serde_json::to_string(settings)?;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![payload],
        )?;
        Ok(())
    }

    pub fn save_tasks(&self, tasks: &[Task]) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM tasks", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO tasks(id, payload) VALUES (?1, ?2)")?;
            for task in tasks {
                stmt.execute(params![task.id, serde_json::to_string(task)?])?;
            }
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('tasks_initialized', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn export_bundle(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let settings = self.load_settings()?.unwrap_or_default();
        let tasks = self.load_tasks()?.unwrap_or_default();
        let data = serde_json::to_vec_pretty(&ExportBundle {
            version: SCHEMA_VERSION,
            settings,
            tasks,
        })?;
        atomic_write(path, &data)?;
        Ok(())
    }

    pub fn replace_bundle(
        &self,
        bundle: &ExportBundle,
        previous: &ExportBundle,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let backup = self
            .path
            .with_file_name(format!("before-import-{}.json", uuid::Uuid::new_v4()));
        atomic_write(&backup, &serde_json::to_vec_pretty(previous)?)?;
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM tasks", [])?;
        for task in &bundle.tasks {
            tx.execute(
                "INSERT INTO tasks(id,payload) VALUES (?1,?2)",
                params![task.id, serde_json::to_string(task)?],
            )?;
        }
        tx.execute("INSERT INTO meta(key,value) VALUES ('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![serde_json::to_string(&bundle.settings)?])?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES ('tasks_initialized','1')",
            [],
        )?;
        tx.commit()?;
        Ok(backup)
    }

    pub fn read_bundle(path: &Path) -> Result<ExportBundle, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let mut bundle: ExportBundle = serde_json::from_slice(&data)?;
        bundle.settings.bt.validate()?;
        bundle.settings.task_view.validate()?;
        bundle.settings.download_window.validate()?;
        if !(1..=SCHEMA_VERSION).contains(&bundle.version) {
            return Err(format!("备份版本过高：{}", bundle.version).into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut bt_hashes = std::collections::HashSet::new();
        for task in &bundle.tasks {
            crate::schedule::validate_time(task.scheduled_at)?;
            if task.id.is_empty()
                || !ids.insert(&task.id)
                || task.file_name != crate::util::sanitize(&task.file_name)
            {
                return Err("备份包含重复任务 ID 或非法文件名".into());
            }
            let url = reqwest::Url::parse(&task.url)?;
            if let Some(source) = &task.torrent {
                crate::torrent::validate_source(source, &task.url)?;
                if !bt_hashes.insert(&source.info_hash) {
                    return Err("备份包含重复的 BT 任务".into());
                }
            } else if !matches!(url.scheme(), "http" | "https") {
                return Err("备份任务链接类型无效".into());
            }
            if !valid_segments(task) {
                return Err("备份包含无效的下载分段进度".into());
            }
        }
        Ok(bundle)
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn valid_segments(task: &Task) -> bool {
    if task.segments.is_empty() {
        return true;
    }
    let mut segments: Vec<_> = task.segments.iter().collect();
    segments.sort_unstable_by_key(|segment| segment.start);
    let mut next = 0;
    for segment in segments {
        let Some(end) = segment.end.checked_add(1) else {
            return false;
        };
        if segment.start != next
            || segment.start >= end
            || segment.pos < segment.start
            || segment.pos > end
        {
            return false;
        }
        next = end;
    }
    task.total == Some(next)
}

fn is_corruption(error: &(dyn std::error::Error + 'static)) -> bool {
    matches!(error.downcast_ref::<rusqlite::Error>(), Some(rusqlite::Error::SqliteFailure(e, _))
        if matches!(e.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Both null-terminated UTF-16 buffers remain alive for the synchronous call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ndm-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validates_imported_segment_ranges() {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id":"task", "url":"https://example.com/file", "fileName":"file",
            "saveDir":"", "total":10, "downloaded":0, "status":"paused",
            "connections":2, "createdAt":0, "finishedAt":null, "error":null, "mime":null,
            "segments":[{"start":5,"end":9,"pos":7},{"start":0,"end":4,"pos":5}]
        }))
        .unwrap();
        assert!(valid_segments(&task));
        task.segments[0].pos = 11;
        assert!(!valid_segments(&task));
        task.segments[0].pos = 7;
        task.segments[0].start = 4;
        assert!(!valid_segments(&task));
        task.segments[0].start = 6;
        assert!(!valid_segments(&task));
        task.segments[0].start = 5;
        task.segments[0].end = u64::MAX;
        assert!(!valid_segments(&task));
        task.segments[0].end = 9;
        task.total = Some(11);
        assert!(!valid_segments(&task));
    }

    #[test]
    fn export_overwrite_and_successful_import() {
        let dir = test_dir();
        let storage = Storage::open(dir.join("state.db")).unwrap();
        storage.save_settings(&Settings::default()).unwrap();
        storage.save_tasks(&[]).unwrap();
        let file = dir.join("export.json");
        storage.export_bundle(&file).unwrap();
        storage.export_bundle(&file).unwrap();
        let mut bundle = Storage::read_bundle(&file).unwrap();
        bundle.settings.connections = 17;
        bundle.settings.bt.max_peers = 139;
        bundle.settings.bt.trackers = vec!["http://127.0.0.1:9000/announce".into()];
        let previous = ExportBundle {
            version: 2,
            settings: Settings {
                connections: 19,
                ..Settings::default()
            },
            tasks: vec![],
        };
        let backup = storage.replace_bundle(&bundle, &previous).unwrap();
        assert_eq!(
            Storage::read_bundle(&backup).unwrap().settings.connections,
            19
        );
        assert_eq!(storage.load_settings().unwrap().unwrap().connections, 17);
        let restored = storage.load_settings().unwrap().unwrap();
        assert_eq!(restored.bt.max_peers, 139);
        assert_eq!(restored.bt.trackers, ["http://127.0.0.1:9000/announce"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_version_is_preserved() {
        let dir = test_dir();
        let path = dir.join("state.db");
        Storage::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value='999' WHERE key='schema_version'", [])
            .unwrap();
        drop(conn);
        assert!(Storage::open(&path).is_err());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
                .count(),
            0
        );
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "999"
        );
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_database_is_quarantined() {
        let dir = test_dir();
        let path = dir.join("state.db");
        std::fs::write(&path, b"this is not a database").unwrap();
        Storage::open(&path).unwrap();
        let backup = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .unwrap();
        assert_eq!(
            std::fs::read(backup.path()).unwrap(),
            b"this is not a database"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_failure_rolls_back_settings_and_tasks() {
        let dir = test_dir();
        let storage = Storage::open(dir.join("state.db")).unwrap();
        let original = Settings::default();
        storage.save_settings(&original).unwrap();
        storage.save_tasks(&[]).unwrap();
        let conn = storage.connection().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_settings BEFORE UPDATE ON meta WHEN NEW.key='settings' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
        drop(conn);
        let incoming = Settings {
            connections: 23,
            ..original.clone()
        };
        assert!(storage
            .replace_bundle(
                &ExportBundle {
                    version: 2,
                    settings: incoming,
                    tasks: vec![]
                },
                &ExportBundle {
                    version: 2,
                    settings: original.clone(),
                    tasks: vec![]
                }
            )
            .is_err());
        assert_eq!(
            storage.load_settings().unwrap().unwrap().connections,
            original.connections
        );
        assert!(storage.load_tasks().unwrap().unwrap().is_empty());
        let backup = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("before-import"))
            .unwrap();
        assert_eq!(
            Storage::read_bundle(&backup.path())
                .unwrap()
                .settings
                .connections,
            original.connections
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("ndm-storage-{}", crate::model::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = Storage::open(dir.join("state.db")).unwrap();
        assert!(storage.load_tasks().unwrap().is_none());

        let settings = Settings::default();
        storage.save_settings(&settings).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().unwrap().connections,
            settings.connections
        );
        storage.save_tasks(&[]).unwrap();
        assert!(storage.load_tasks().unwrap().unwrap().is_empty());
        assert!(storage.path().exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
