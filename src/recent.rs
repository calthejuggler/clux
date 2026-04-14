use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
pub struct RecentEntry {
    pub session_id: String,
    pub switched_at: u64,
}

const MAX_ENTRIES: usize = 100;

fn state_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        })?;
    Some(base.join("clux").join("recent.json"))
}

pub fn load() -> Vec<RecentEntry> {
    let Some(path) = state_path() else {
        return Vec::new();
    };
    load_from(&path)
}

pub fn load_from(path: &Path) -> Vec<RecentEntry> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn record_switch(session_id: &str) -> anyhow::Result<()> {
    let path = state_path().ok_or_else(|| anyhow::anyhow!("cannot determine state directory"))?;
    record_switch_to(session_id, &path)
}

pub fn record_switch_to(session_id: &str, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut entries = load_from(path);

    entries.retain(|entry| entry.session_id != session_id);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |dur| dur.as_secs());

    entries.insert(
        0,
        RecentEntry {
            session_id: session_id.to_owned(),
            switched_at: now,
        },
    );

    entries.truncate(MAX_ENTRIES);

    let json = serde_json::to_string(&entries)?;
    std::fs::write(path, json)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("clux").join("recent.json")
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let entries = load_from(&test_path(&dir));
        assert!(entries.is_empty());
    }

    #[test]
    fn record_switch_creates_file_and_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = test_path(&dir);
        record_switch_to("sess-1", &path).expect("record");
        let entries = load_from(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.first().expect("entry").session_id, "sess-1");
        assert!(path.exists());
    }

    #[test]
    fn record_switch_upserts_to_front() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = test_path(&dir);
        record_switch_to("sess-1", &path).expect("record");
        record_switch_to("sess-2", &path).expect("record");
        record_switch_to("sess-1", &path).expect("record");
        let entries = load_from(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.first().expect("entry").session_id, "sess-1");
        assert_eq!(entries.get(1).expect("entry").session_id, "sess-2");
    }

    #[test]
    fn record_switch_caps_at_max() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = test_path(&dir);
        for i in 0..150 {
            record_switch_to(&format!("sess-{i}"), &path).expect("record");
        }
        let entries = load_from(&path);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries.first().expect("entry").session_id, "sess-149");
    }

    #[test]
    fn load_bad_json_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = test_path(&dir);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "not json").expect("write");
        let entries = load_from(&path);
        assert!(entries.is_empty());
    }
}
