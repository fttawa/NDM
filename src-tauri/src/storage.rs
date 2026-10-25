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
            if !storage.path.exists() {
                return Err(first);
            }
            let backup = storage.quarantine_path();
            std::fs::rename(&storage.path, &backup)
                .map_err(|e| format!("数据库损坏且无法备份：{e}; 原因：{first}"))?;
            for suffix in ["-wal", "-shm"] {
                let sidecar =
                    PathBuf::from(format!("{}{}", storage.path.to_string_lossy(), suffix));
                if sidecar.exists() {
                    let _ = std::fs::rename(
                        &sidecar,
                        format!("{}{}", backup.to_string_lossy(), suffix),
                    );
                }
            }
            eprintln!("数据库已损坏，已备份到 {}", backup.display());
            storage.initialize()?;
        }
        Ok(storage)
    }

    fn initialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.connection()?;
        conn.execute_batch(
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
        let mut version: i64 = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()?;
        if version > SCHEMA_VERSION {
            return Err(format!("不支持的数据库版本：{version}").into());
        }
        if version < 2 {
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(json_extract(payload, '$.status'));
                 CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(json_extract(payload, '$.createdAt'));
                 UPDATE meta SET value = '2' WHERE key = 'schema_version';",
            )?;
            version = 2;
        }
        if version != SCHEMA_VERSION {
            return Err(format!("数据库迁移失败，当前版本：{version}").into());
        }
        conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .and_then(|result| {
                if result == "ok" {
                    Ok(())
                } else {
                    Err(rusqlite::Error::InvalidQuery)
                }
            })?;
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
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, data)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub fn read_bundle(path: &Path) -> Result<ExportBundle, Box<dyn std::error::Error>> {
        let data = std::fs::read(path)?;
        let bundle: ExportBundle = serde_json::from_slice(&data)?;
        if bundle.version > SCHEMA_VERSION {
            return Err(format!("备份版本过高：{}", bundle.version).into());
        }
        Ok(bundle)
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;

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
