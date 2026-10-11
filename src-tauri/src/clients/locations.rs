//! Machine-local adapter locations. Deliberately outside Registry and personal sync.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientLocation {
    pub id: String,
    pub client_type: String,
    pub name: String,
    pub config_path: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientLocationType {
    pub id: String,
    pub name: String,
    pub config_file: String,
}

/// The folder is the client's config folder, e.g. CODEX_HOME, not a project root.
pub fn client_location_types() -> Vec<ClientLocationType> {
    defs()
        .into_iter()
        .filter(|def| !def.uses_connectors)
        .filter_map(|def| {
            let file = (def.path)()?.file_name()?.to_str()?.to_string();
            Some(ClientLocationType {
                id: def.id.into(),
                name: def.name.into(),
                config_file: file,
            })
        })
        .collect()
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::registry::conduit_dir()
        .ok_or("Could not resolve local data folder")?
        .join("client-locations.json"))
}

pub(super) fn saved() -> Result<Vec<ClientLocation>, String> {
    match std::fs::read(store_path()?) {
        Ok(bytes) => {
            let rows: Vec<ClientLocation> = serde_json::from_slice(&bytes)
                .map_err(|_| "Could not read saved client locations".to_string())?;
            if rows.iter().any(|row| {
                !valid_id(&row.id)
                    || !row.config_path.is_absolute()
                    || !defs()
                        .iter()
                        .any(|def| def.id == row.client_type && !def.uses_connectors)
            }) {
                return Err("Invalid saved client location".into());
            }
            Ok(rows)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("Could not read saved client locations".into()),
    }
}

fn valid_id(id: &str) -> bool {
    id.split_once('@')
        .is_some_and(|(kind, key)| !kind.is_empty() && !key.is_empty() && key != "." && key != "..")
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.@%".contains(&c))
}

fn folder_key(folder: &str) -> String {
    folder
        .trim_start_matches('.')
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn automatic(home: &Path) -> Vec<ClientLocation> {
    let mut rows = Vec::new();
    for folder in claude_profile_dirs(home).unwrap_or_default() {
        let config_path = home.join(&folder).join(".claude.json");
        if !std::fs::symlink_metadata(&config_path).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        rows.push(ClientLocation {
            id: format!("claude-code@{}", folder_key(&folder)),
            client_type: "claude-code".into(),
            name: format!("Claude Code ({})", folder.trim_start_matches('.')),
            config_path,
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

pub(super) fn all() -> Vec<ClientLocation> {
    let mut rows = home().map(|home| automatic(&home)).unwrap_or_default();
    match saved() {
        Ok(saved) => {
            for row in saved {
                if !rows.iter().any(|auto| auto.config_path == row.config_path) {
                    rows.push(row);
                }
            }
        }
        Err(error) => eprintln!("toolport: {error}"),
    }
    rows
}

/// A known automatic ID resolves without enumerating home or reading a config.
pub(super) fn lookup(id: &str) -> Option<ClientLocation> {
    if !valid_id(id) {
        return None;
    }
    if let Some(row) = saved().ok()?.into_iter().find(|row| row.id == id) {
        return Some(row);
    }
    let key = id.strip_prefix("claude-code@")?;
    let mut bytes = Vec::new();
    let mut chars = key.as_bytes().iter().copied();
    while let Some(byte) = chars.next() {
        bytes.push(if byte == b'%' {
            u8::from_str_radix(
                std::str::from_utf8(&[chars.next()?, chars.next()?]).ok()?,
                16,
            )
            .ok()?
        } else {
            byte
        });
    }
    let label = String::from_utf8(bytes).ok()?;
    if label.contains(['/', '\\']) || label.chars().any(char::is_control) {
        return None;
    }
    let folder = format!(".{label}");
    if !folder.starts_with(".claude") || folder_key(&folder) != key {
        return None;
    }
    let root = home()?;
    let path = root.join(&folder);
    // Do not follow an automatically discovered directory outside home later.
    if !std::fs::symlink_metadata(&path).ok()?.is_dir() {
        return None;
    }
    Some(ClientLocation {
        id: id.into(),
        client_type: "claude-code".into(),
        name: format!("Claude Code ({label})"),
        config_path: path.join(".claude.json"),
    })
}

pub(super) fn remember(row: &ClientLocation) -> Result<(), String> {
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    if !rows.iter().any(|saved| saved.id == row.id) {
        rows.push(row.clone());
        crate::registry::atomic_write(
            &store,
            &serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

pub fn add_client_location(client_type: &str, folder: &str) -> Result<String, String> {
    let kind = client_location_types()
        .into_iter()
        .find(|kind| kind.id == client_type)
        .ok_or("Choose a supported client type")?;
    let raw = PathBuf::from(folder.trim());
    let folder = if folder.trim() == "~" {
        home().ok_or("Could not resolve home")?
    } else if let Some(relative) = folder.trim().strip_prefix("~/") {
        home().ok_or("Could not resolve home")?.join(relative)
    } else {
        raw
    };
    if !folder.is_absolute() {
        return Err("Choose an absolute client config folder".into());
    }
    let folder =
        std::fs::canonicalize(&folder).map_err(|_| "Choose an existing client config folder")?;
    if !folder.is_dir() {
        return Err("Choose a client config folder, not a file".into());
    }
    let path = folder.join(&kind.config_file);
    if let Some(row) = all()
        .into_iter()
        .find(|row| same_path(&row.config_path, &path))
    {
        if row.client_type != client_type {
            return Err("This config file is already registered for another client type".into());
        }
        return Ok(row.id);
    }
    if let Some(def) = defs().into_iter().find(|def| {
        def.id == client_type && (def.path)().is_some_and(|existing| same_path(&existing, &path))
    }) {
        return Ok(def.id.into());
    }
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    if let Some(row) = all()
        .into_iter()
        .find(|row| same_path(&row.config_path, &path))
    {
        if row.client_type != client_type {
            return Err("This config file is already registered for another client type".into());
        }
        return Ok(row.id);
    }
    use sha2::{Digest, Sha256};
    let key = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    let id = format!("{client_type}@{}", &key[..16]);
    let label = folder.file_name().unwrap_or_default().to_string_lossy();
    rows.push(ClientLocation {
        id: id.clone(),
        client_type: client_type.into(),
        name: format!("{} ({})", kind.name, label.trim_start_matches('.')),
        config_path: path,
    });
    crate::registry::atomic_write(
        &store,
        &serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?,
    )?;
    Ok(id)
}

/// Read-only benchmark helper, excluded from shipped builds.
#[cfg(any(test, feature = "test-support"))]
pub fn measure_scan(home: &Path, iterations: u32) -> (usize, f64, f64) {
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let dirs: Vec<String> = std::fs::read_dir(home)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        let _ = dirs
            .iter()
            .filter(|name| *name == ".claude" || name.starts_with(".claude-"))
            .filter(|name| home.join(name).join(".claude.json").is_file())
            .count();
    }
    let before = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = automatic(home);
    }
    let after = start.elapsed().as_secs_f64() * 1000.0 / iterations as f64;
    (std::fs::read_dir(home).unwrap().count(), before, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempFolder(PathBuf);
    impl TempFolder {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "toolport-location-{}-{}-{}",
                std::process::id(),
                epoch_millis(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn detection_is_shallow_name_filtered_and_does_not_parse_configs() {
        let root = TempFolder::new();
        for folder in [
            ".claude",
            ".claude2",
            ".claude3",
            ".claude-work",
            ".claude-empty",
            "unrelated/.claude4",
        ] {
            std::fs::create_dir_all(root.path().join(folder)).unwrap();
            if folder != ".claude-empty" {
                std::fs::write(root.path().join(folder).join(".claude.json"), "not JSON").unwrap();
            }
        }
        let rows = automatic(root.path());
        assert_eq!(rows.len(), 4);
        assert!(rows
            .iter()
            .any(|row| row.id == "claude-code@claude2" && row.name == "Claude Code (claude2)"));
        assert_ne!(folder_key(".claude+a"), folder_key(".claude-a"));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_symlinks_outside_home() {
        let home = TempFolder::new();
        let outside = TempFolder::new();
        std::fs::write(outside.path().join(".claude.json"), "{}").unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join(".claude-outside")).unwrap();
        std::fs::create_dir(home.path().join(".claude-linked-file")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join(".claude.json"),
            home.path().join(".claude-linked-file/.claude.json"),
        )
        .unwrap();
        assert!(automatic(home.path()).is_empty());
    }

    #[test]
    fn manual_locations_are_stable_deduplicated_and_separate_from_registry() {
        let _data = crate::registry::DataDirTestEnv::new("client-locations");
        let folder = TempFolder::new();
        let id = add_client_location("codex", folder.path().to_str().unwrap()).unwrap();
        assert_eq!(
            add_client_location("codex", folder.path().to_str().unwrap()).unwrap(),
            id
        );
        assert_eq!(saved().unwrap().len(), 1);
        let def = find_def(&id).unwrap();
        assert_eq!(
            resolved_definition_path(&def).unwrap(),
            folder.path().join("config.toml")
        );
        assert_eq!(
            discovery_capabilities(&id).auto_mode,
            discovery_capabilities("codex").auto_mode
        );
        assert!(!crate::registry::resolved_path().unwrap().exists());
        assert!(add_client_location("codex", "relative").is_err());
        assert!(!valid_id("codex@../escape"));
    }

    #[test]
    fn environment_selected_location_has_one_identity_for_upkeep() {
        let base = defs().into_iter().find(|def| def.id == "codex").unwrap();
        let mut location = defs().into_iter().find(|def| def.id == "codex").unwrap();
        location.location = Some(ClientLocation {
            id: "codex@selected".into(),
            client_type: "codex".into(),
            name: "Codex (selected)".into(),
            config_path: resolved_definition_path(&base).unwrap(),
        });
        let rows = unique_config_definitions(vec![base, location]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].client_id(), "codex@selected");
    }

    #[test]
    fn existing_manual_gateway_is_adopted_without_duplicate_entries() {
        let _data = crate::registry::DataDirTestEnv::new("client-location-adoption");
        let folder = TempFolder::new();
        let path = folder.path().join(".claude.json");
        std::fs::write(&path, r#"{"mcpServers":{"toolport":{"command":"/usr/bin/toolport-gateway","env":{"TOOLPORT_CLIENT_ID":"claude-code"}},"other":{"command":"other"}}}"#).unwrap();
        let id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        let def = find_def(&id).unwrap();
        let detected = read_client(&def);
        assert_eq!(
            resolve_entry_state(&detected.servers, None),
            GatewayEntryState::Managed
        );
        let mut entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport", "transport":"stdio", "env":[{"key":"TOOLPORT_CLIENT_ID", "value":id, "secret":false}]})).unwrap();
        entry.command = Some("/usr/bin/toolport-gateway".into());
        install_or_remove(&id, Some(&entry)).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcpServers"].as_object().unwrap().len(), 2);
        assert_eq!(
            value["mcpServers"]["toolport"]["env"]["TOOLPORT_CLIENT_ID"],
            id
        );
        assert!(backup_dir(&id).unwrap().is_dir());
        assert_eq!(known_adapter_name(&id).unwrap(), detected.name);
    }

    #[test]
    fn access_and_caller_names_remain_independent_per_location() {
        let _data = crate::registry::DataDirTestEnv::new("location-access");
        let a = TempFolder::new();
        let b = TempFolder::new();
        let first = add_client_location("codex", a.path().to_str().unwrap()).unwrap();
        let second = add_client_location("codex", b.path().to_str().unwrap()).unwrap();
        assert_ne!(first, second);
        let mut reg = crate::registry::Registry::default();
        for id in ["first-access", "second-access"] {
            reg.profiles
                .push(serde_json::from_value(serde_json::json!({"id":id,"name":id})).unwrap());
        }
        reg.client_scopes
            .insert(first.clone(), "first-access".into());
        reg.client_scopes
            .insert(second.clone(), "second-access".into());
        assert_eq!(
            resolve_launch_profile(&reg, Some(&first), &None),
            Some("first-access".into())
        );
        assert_eq!(
            resolve_launch_profile(&reg, Some(&second), &None),
            Some("second-access".into())
        );
        let entries = vec![
            serde_json::json!({"client":format!("adapter:{first}")}),
            serde_json::json!({"client":format!("adapter:{second}")}),
        ];
        let names = CallerNames::for_entries(&entries);
        assert_ne!(
            names.trusted_name(entries[0]["client"].as_str(), None),
            names.trusted_name(entries[1]["client"].as_str(), None)
        );
        assert_ne!(backup_dir(&first), backup_dir(&second));
    }

    #[test]
    fn later_background_adoption_keeps_existing_access_and_location_override() {
        let _data = crate::registry::DataDirTestEnv::new("later-location-access");
        let mut registry = crate::registry::Registry::default();
        registry.client_scopes.insert("claude-code".into(), "work".into());
        crate::registry::save(&registry).unwrap();
        let text = r#"{"mcpServers":{"toolport":{"command":"toolport-gateway","env":{"TOOLPORT_CLIENT_ID":"claude-code"}}}}"#;
        inherit_location_access("claude-code@claude2", text).unwrap();
        let mut registry = crate::registry::load().unwrap();
        assert_eq!(registry.client_scopes.get("claude-code@claude2"), Some(&"work".into()));
        registry.client_scopes.insert("claude-code@claude2".into(), "personal".into());
        crate::registry::save(&registry).unwrap();
        inherit_location_access("claude-code@claude2", text).unwrap();
        assert_eq!(crate::registry::load().unwrap().client_scopes.get("claude-code@claude2"), Some(&"personal".into()));
    }

    #[test]
    fn identity_only_adoption_preserves_existing_gateway_fields_and_restore_record() {
        let _data = crate::registry::DataDirTestEnv::new("location-identity-adoption");
        let folder = TempFolder::new();
        let path = folder.path().join(".claude.json");
        let raw = serde_json::json!({"mcpServers":{"toolport":{"command":"/usr/bin/toolport-gateway","args":["--private"],"env":{"TOOLPORT_CLIENT_ID":"claude-code","TOOLPORT_PROFILE":"work","KEEP":"value"},"unknown":42}}});
        std::fs::write(&path, raw.to_string()).unwrap();
        let id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        mutation::run(&id, &path, Format::JsonMcpServers, || {
            backfill_gateway_identity(&find_def(&id).unwrap(), &path, "toolport", &id)
        })
        .unwrap();
        let mut after: serde_json::Value =
            serde_json::from_str(&read_config_file(&path).unwrap()).unwrap();
        assert_eq!(
            after["mcpServers"]["toolport"]["env"][crate::brand::CLIENT_ID],
            id
        );
        after["mcpServers"]["toolport"]["env"][crate::brand::CLIENT_ID] =
            serde_json::json!("claude-code");
        after["mcpServers"]["toolport"]["env"]
            .as_object_mut()
            .unwrap()
            .remove(crate::brand::ATTRIBUTION_ID);
        assert_eq!(after, raw);
        assert!(restore::recorded_paths()
            .iter()
            .any(|(record_id, recorded, _)| record_id == &id && recorded == &path));
        let outcome = uninstall_gateway(&id).unwrap();
        assert!(outcome.warnings.is_empty());
        assert!(!read_client(&find_def(&id).unwrap()).gateway_installed);
        finish_uninstall(&id, &outcome).unwrap();
    }

    #[test]
    #[ignore = "scan benchmark; run explicitly with --ignored --nocapture"]
    fn scan_timings() {
        let root = TempFolder::new();
        for i in 0..10_000 {
            std::fs::write(root.path().join(format!("unrelated-{i}")), "").unwrap();
        }
        for i in 0..20 {
            let dir = root.path().join(format!(".claude-profile{i}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join(".claude.json"), "{}").unwrap();
        }
        fn measure(home: &Path) {
            let (entries, before, after) = measure_scan(home, 20);
            println!("entries={entries} before_ms={before:.3} after_ms={after:.3}");
        }
        measure(root.path());
        // Only directory metadata; no config contents or writes in real home.
        if let Some(real_home) = home() {
            measure(&real_home);
        }
    }
}
