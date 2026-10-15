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
    language_at(
        &settings_path()?,
        crate::registry::resolved_path().as_deref(),
    )
}

fn language_at(path: &Path, registry: Option<&Path>) -> Result<String, String> {
    let _lock = lock_settings(path)?;
    language_from(&initialized_settings(path, registry)?)
}

// Both shells call language() before their startup work can create a registry.
// Persist the result once so later backend writes cannot reclassify a fresh run.
// Use the registry's existing first-run/recovery decision, not data-dir contents.
fn initialized_settings(
    path: &Path,
    registry: Option<&Path>,
) -> Result<Map<String, Value>, String> {
    let mut settings = read_settings(path)?;
    if !settings.contains_key("language") {
        let source = match registry {
            Some(path) => crate::registry::load_from_with_source(path)?.1,
            None => crate::registry::LoadSource::FirstRun,
        };
        // An empty 2.0 install may never have saved a registry, but both shells
        // record their completed one-time startup cleanup. Ignore arbitrary logs
        // and locks, which a genuine first launch can create before the UI mounts.
        let prior_launch = path
            .parent()
            .is_some_and(|dir| dir.join(crate::guard_cleanup::COMPLETION_MARKER).is_file());
        let existing = source != crate::registry::LoadSource::FirstRun || prior_launch;
        settings.insert(
            "language".into(),
            Value::String(if existing { "en" } else { "system" }.into()),
        );
        settings.insert("languageSuggestionPending".into(), Value::Bool(existing));
        write_settings(path, &settings)?;
    }
    Ok(settings)
}

/// Machine-local upgrade suggestion; never synced with the registry.
pub fn language_suggestion_pending() -> Result<bool, String> {
    suggestion_pending_at(
        &settings_path()?,
        crate::registry::resolved_path().as_deref(),
    )
}

fn suggestion_pending_at(path: &Path, registry: Option<&Path>) -> Result<bool, String> {
    let _lock = lock_settings(path)?;
    let settings = initialized_settings(path, registry)?;
    Ok(settings
        .get("languageSuggestionPending")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    crate::registry::atomic_write(path, &raw)
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
    settings.insert("languageSuggestionPending".into(), Value::Bool(false));
    write_settings(path, &settings)
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

    fn language_at(path: &Path) -> Result<String, String> {
        super::language_at(path, Some(&path.with_file_name("registry.json")))
    }

    fn suggestion_pending_at(path: &Path) -> Result<bool, String> {
        super::suggestion_pending_at(path, Some(&path.with_file_name("registry.json")))
    }

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
    fn fresh_choice_is_pinned_before_later_backend_registry_writes() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        // A first launch may already have locks, logs and other backend files.
        std::fs::write(dir.path().join("gateway.log"), "startup").unwrap();
        assert_eq!(language_at(&path).unwrap(), "system");
        assert!(!suggestion_pending_at(&path).unwrap());
        std::fs::write(
            dir.path().join(crate::guard_cleanup::COMPLETION_MARKER),
            "startup complete",
        )
        .unwrap();
        crate::registry::save_to(
            &dir.path().join("registry.json"),
            &crate::registry::Registry::default(),
        )
        .unwrap();
        assert_eq!(language_at(&path).unwrap(), "system");
        assert!(!suggestion_pending_at(&path).unwrap());
    }

    #[test]
    fn a_previously_opened_empty_install_keeps_english_without_a_registry() {
        let dir = TestDir::new();
        std::fs::write(
            dir.path().join(crate::guard_cleanup::COMPLETION_MARKER),
            "startup complete",
        )
        .unwrap();
        let path = dir.path().join("ui-settings.json");
        assert_eq!(language_at(&path).unwrap(), "en");
        assert!(suggestion_pending_at(&path).unwrap());
        assert!(!dir.path().join("registry.json").exists());
    }

    #[test]
    fn existing_install_keeps_english_until_a_choice_clears_its_suggestion() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        crate::registry::save_to(
            &dir.path().join("registry.json"),
            &crate::registry::Registry::default(),
        )
        .unwrap();
        assert_eq!(language_at(&path).unwrap(), "en");
        assert!(suggestion_pending_at(&path).unwrap());
        let before = std::fs::read(&path).unwrap();
        assert_eq!(language_at(&path).unwrap(), "en");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "migration saves only once"
        );
        assert!(set_language_at(&path, "unsupported").is_err());
        assert!(suggestion_pending_at(&path).unwrap());
        for choice in ["en", "ja", "system", "de"] {
            let mut settings = read_settings(&path).unwrap();
            settings.insert("languageSuggestionPending".into(), Value::Bool(true));
            write_settings(&path, &settings).unwrap();
            set_language_at(&path, choice).unwrap();
            assert_eq!(language_at(&path).unwrap(), choice);
            assert!(!suggestion_pending_at(&path).unwrap());
        }
    }

    #[test]
    fn legacy_and_recovered_registries_are_existing_installs() {
        let _lock = crate::registry::data_dir_test_lock();
        for legacy in [true, false] {
            let dir = TestDir::new();
            let _data = crate::registry::DataDirOverride::set(dir.path());
            let registry = dir.path().join("registry.json");
            let mut document = serde_json::to_value(crate::registry::Registry::default()).unwrap();
            if legacy {
                document["version"] = Value::from(1);
                std::fs::write(&registry, serde_json::to_string(&document).unwrap()).unwrap();
            } else {
                std::fs::write(
                    registry.with_extension("json.bak"),
                    serde_json::to_string(&document).unwrap(),
                )
                .unwrap();
            }
            let path = dir.path().join("ui-settings.json");
            assert_eq!(language_at(&path).unwrap(), "en");
            assert!(suggestion_pending_at(&path).unwrap());
        }
    }

    #[test]
    fn saved_choices_are_never_reclassified_or_rewritten() {
        let dir = TestDir::new();
        let path = dir.path().join("ui-settings.json");
        for choice in LANGUAGES {
            let raw = format!(r#"{{"language":"{choice}","future":42}}"#);
            std::fs::write(&path, &raw).unwrap();
            assert_eq!(language_at(&path).unwrap(), *choice);
            assert!(!suggestion_pending_at(&path).unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
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
        assert_eq!(read_settings(&path).unwrap()["language"], "system");
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
