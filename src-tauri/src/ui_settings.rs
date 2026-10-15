//! Machine-local UI preferences. These are never part of the synced registry.

use fs2::FileExt;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const LANGUAGES: &[&str] = &[
    "system", "en", "ja", "zh-CN", "zh-TW", "ko", "es", "fr", "de", "pt-BR",
];

fn settings_path() -> Result<PathBuf, String> {
    crate::registry::conduit_dir()
        .map(|dir| dir.join("ui-settings.json"))
        .ok_or_else(|| "could not resolve UI settings directory".to_owned())
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| format!("could not read UI settings: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(format!("could not read UI settings: {error}")),
    }
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
    language_from(&read_settings(&settings_path()?)?)
}

pub fn set_language(language: &str) -> Result<(), String> {
    set_language_at(&settings_path()?, language)
}

fn set_language_at(path: &Path, language: &str) -> Result<(), String> {
    if !LANGUAGES.contains(&language) {
        return Err("invalid UI language choice".to_owned());
    }
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
    let mut settings = read_settings(path)?;
    settings.insert("language".to_owned(), Value::String(language.to_owned()));
    let raw = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    crate::registry::atomic_write(path, &raw)
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
    fn corrupt_settings_are_not_overwritten() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(set_language_at(&path, "ja").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
        assert!(language_from(&serde_json::from_str(r#"{"language":"xx"}"#).unwrap()).is_err());
    }
}
