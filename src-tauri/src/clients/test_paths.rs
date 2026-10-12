//! Unit tests never resolve a client path through the runner's home or environment.
//! Explicit overrides are thread-local, so parallel tests cannot borrow each other's
//! fixtures. Production and subprocess integration tests retain the real resolvers.
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

thread_local! {
    static ENV: std::cell::RefCell<HashMap<&'static str, OsString>> = std::cell::RefCell::new(HashMap::new());
    static SCRATCH: Scratch = Scratch::new();
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!("toolport-client-home-{}-{}",
            std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&root).expect("scratch client home");
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

pub(super) fn env(key: &str) -> Option<OsString> {
    ENV.with(|slot| slot.borrow().get(key).cloned())
}

pub(super) fn set(key: &'static str, value: Option<OsString>) -> Option<OsString> {
    ENV.with(|slot| match value {
        Some(value) => slot.borrow_mut().insert(key, value),
        None => slot.borrow_mut().remove(key),
    })
}

pub(super) fn home() -> PathBuf {
    env("HOME").filter(|value| !value.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| SCRATCH.with(|scratch| scratch.0.clone()))
}

pub(super) fn dir(key: &str, fallback: impl FnOnce() -> PathBuf) -> Option<PathBuf> {
    Some(env(key).filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or_else(fallback))
}

#[test]
fn inherited_paths_and_parallel_overrides_cannot_reach_client_configs() {
    use super::*;
    let data = crate::registry::DataDirTestEnv::new("ambient-client-paths");
    let _lock = env_test_lock();
    let sentinel = data.dir.join("sentinel");
    std::fs::create_dir_all(&sentinel).unwrap();
    let path = sentinel.join(".claude.json");
    let original = r#"{"mcpServers":{"toolport":{"command":"/old/toolport-gateway"}}}"#;
    std::fs::write(&path, original).unwrap();
    let _env: Vec<_> = [
        "HOME", "USERPROFILE", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "APPDATA", "LOCALAPPDATA",
        "CLAUDE_CONFIG_DIR", "GOOSE_PATH_ROOT", "CODEX_HOME", "GEMINI_CLI_HOME",
        "CRUSH_GLOBAL_CONFIG", "AMP_SETTINGS_FILE", "COPILOT_HOME", "GROK_HOME",
        "QWEN_HOME", "KIMI_CODE_HOME",
    ].into_iter().map(|key| EnvRestore::set(key, &sentinel)).collect();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    // A new worker has no fixture override, even while the process environment
    // and another thread's overrides both name valid client configs.
    std::thread::spawn(|| {
        let scratch = home().unwrap();
        for def in defs() {
            assert!((def.path)().unwrap().starts_with(&scratch), "{}", def.id);
            if let Some(target) = client_rules_target(def.id, crate::instructions::Scope::Team) {
                assert!(target.path.starts_with(&scratch), "{}", def.id);
            }
        }
        assert!(try_claude_code_config_paths().unwrap().is_empty());
        assert!(detect_clients().iter().all(|client| !client.config_exists));
        for dry_run in [true, false] { assert!(disconnect_all(dry_run).unwrap().is_empty()); }
        crate::hooks::cleanup_on_startup_with(&claude_settings_paths());
        crate::guard_cleanup::run_once();
    }).join().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    assert_eq!(std::fs::read_dir(sentinel).unwrap().count(), 1);
}

#[test]
fn nested_fixture_overrides_restore_thread_local_paths() {
    use super::*;
    let data = crate::registry::DataDirTestEnv::new("nested-client-paths");
    let _lock = env_test_lock();
    let default = client_config_path("claude-code");
    {
        let _first = EnvRestore::set("CLAUDE_CONFIG_DIR", &data.dir);
        assert_eq!(client_config_path("claude-code"), Some(data.dir.join(".claude.json")));
        {
            let nested = data.dir.join("nested");
            let _second = EnvRestore::set("CLAUDE_CONFIG_DIR", &nested);
            assert_eq!(client_config_path("claude-code"), Some(nested.join(".claude.json")));
        }
        assert_eq!(client_config_path("claude-code"), Some(data.dir.join(".claude.json")));
    }
    assert_eq!(client_config_path("claude-code"), default);
}
