//! Privacy by construction: only event codes and numeric task state leave the app.
use parking_lot::Mutex;
use std::{io::Write, path::PathBuf, sync::OnceLock};
static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();

pub fn init(dir: &std::path::Path) {
    let _ = LOG.set(Mutex::new(dir.join("diagnostics.jsonl")));
    record("startup");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record("panic");
        previous(info);
    }));
}

pub fn record(code: &'static str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let path = log.lock();
    if std::fs::metadata(&*path)
        .map(|m| m.len() > 262144)
        .unwrap_or(false)
    {
        let previous = path.with_extension("previous.jsonl");
        if previous.exists() && std::fs::remove_file(&previous).is_err() {
            return;
        }
        if std::fs::rename(&*path, previous).is_err() {
            return;
        }
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*path)
    {
        let _ = writeln!(
            file,
            "{}",
            serde_json::json!({"time":crate::model::now_ms(),"event":code})
        );
    }
}

pub fn events() -> Vec<serde_json::Value> {
    let Some(log) = LOG.get() else {
        return Vec::new();
    };
    let path = log.lock();
    [path.with_extension("previous.jsonl"), path.clone()]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|s| {
            s.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}
