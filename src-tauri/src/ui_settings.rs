//! Machine-local UI preferences. These are never part of the synced registry.

use fs2::FileExt;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const LANGUAGES: &[&str] = &[
    "system", "en", "ja", "zh-CN", "zh-TW", "ko", "es", "fr", "de", "pt-BR",
];

fn settings_path() -> Result<PathBuf, String> {
    crate::registry::conduit_dir()
        .map(|dir| dir.join("ui-settings.json"))
        .ok_or_else(|| "could not resolve UI settings directory".to_owned())
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read(path) {
        Ok(raw) => match serde_json::from_slice(&raw) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                preserve_corrupt_settings(path)?;
                // Preserve the unreadable bytes before publishing fresh defaults.
                crate::registry::atomic_write(path, "{}")?;
                Ok(Map::new())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(format!("could not read UI settings: {error}")),
    }
}

// Called only while holding the stable settings lock, so another shell cannot
// replace the file between reading it and moving the corrupt bytes aside.
fn preserve_corrupt_settings(path: &Path) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .ok_or("invalid UI settings path")?
        .to_string_lossy();
    let backup = path.with_file_name(format!(
        "{name}.corrupt-{timestamp}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::rename(path, backup)
        .map_err(|error| format!("could not preserve corrupt UI settings: {error}"))
}

fn language_from(settings: &Map<String, Value>) -> Result<String, String> {
    match settings.get("language") {
        None => Ok("system".to_owned()),
        Some(Value::String(language)) if LANGUAGES.contains(&language.as_str()) => {
            Ok(language.clone())
        }
        Some(_) => Err("invalid UI language setting".to_owned()),
    }
}

pub fn language() -> Result<String, String> {
    language_at(&settings_path()?)
}

fn language_at(path: &Path) -> Result<String, String> {
    let _lock = lock_settings(path)?;
    language_from(&read_settings(path)?)
}

pub fn set_language(language: &str) -> Result<(), String> {
    set_language_at(&settings_path()?, language)
}

fn set_language_at(path: &Path, language: &str) -> Result<(), String> {
    if !LANGUAGES.contains(&language) {
        return Err("invalid UI language choice".to_owned());
    }
    let _lock = lock_settings(path)?;
    let mut settings = read_settings(path)?;
    settings.insert("language".to_owned(), Value::String(language.to_owned()));
    let raw = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    crate::registry::atomic_write(path, &raw)
}

fn lock_settings(path: &Path) -> Result<std::fs::File, String> {
    let parent = path.parent().ok_or("invalid UI settings path")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    // A separate stable lock file protects read-modify-write across shells.
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(path.with_extension("lock"))
        .map_err(|error| format!("could not open UI settings lock: {error}"))?;
    lock.lock_exclusive()
        .map_err(|error| format!("could not lock UI settings: {error}"))?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "toolport-ui-settings-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_settings_default_to_system() {
        let dir = TestDir::new();
        assert_eq!(
            language_from(&read_settings(&dir.path().join("ui-settings.json")).unwrap()).unwrap(),
            "system"
        );
    }

    #[test]
    fn updates_preserve_other_fields_and_reject_invalid_choices() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        std::fs::write(&path, r#"{"future":{"enabled":true},"language":"de"}"#).unwrap();
        set_language_at(&path, "ja").unwrap();
        let settings = read_settings(&path).unwrap();
        assert_eq!(language_from(&settings).unwrap(), "ja");
        assert_eq!(settings["future"]["enabled"], true);
        let before = std::fs::read(&path).unwrap();
        assert!(set_language_at(&path, "../invalid").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn corrupt_settings_are_preserved_before_read_defaults_and_save() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        std::fs::write(&path, "broken").unwrap();
        assert_eq!(language_at(&path).unwrap(), "system");
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ui-settings.json.corrupt-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "broken");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        set_language_at(&path, "ja").unwrap();
        assert_eq!(language_at(&path).unwrap(), "ja");
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "broken");
        assert!(language_from(&serde_json::from_str(r#"{"language":"xx"}"#).unwrap()).is_err());
    }

    #[test]
    fn invalid_utf8_settings_are_also_preserved_and_recovered() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert_eq!(language_at(&path).unwrap(), "system");
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ui-settings.json.corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), [0xff, 0xfe]);
        set_language_at(&path, "ja").unwrap();
        assert_eq!(language_at(&path).unwrap(), "ja");
    }

    #[test]
    fn save_recovers_corrupt_settings_but_invalid_requests_leave_them_untouched() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        std::fs::write(&path, "[not valid JSON").unwrap();
        assert!(set_language_at(&path, "invalid").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[not valid JSON");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        set_language_at(&path, "de").unwrap();
        assert_eq!(language_at(&path).unwrap(), "de");
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ui-settings.json.corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "[not valid JSON");
    }
}
