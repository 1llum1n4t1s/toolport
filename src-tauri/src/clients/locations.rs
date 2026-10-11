//! Machine-local adapter locations. Deliberately outside Registry and personal sync.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientLocation {
    pub id: String,
    #[serde(default)]
    pub manual: bool,
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

fn location_name(kind: &str, folder: &str) -> String {
    let folder: String = folder.trim_start_matches('.').chars().map(|c| {
        if c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '+' | '(' | ')' | '[' | ']') {
            c
        } else { '-' }
    }).take(60).collect();
    let mut label = format!("{kind} ({folder})");
    // Bound trusted folder names before they reach the strict telemetry label filter.
    while label.len() > 180 { label.pop(); }
    if !label.ends_with(')') { label.push(')'); }
    label
}

pub fn remove_client_location(id: &str) -> Result<(), String> {
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    let row = rows.iter().find(|row| row.id == id && row.manual)
        .ok_or("This is not a manually added client location")?;
    // Forgetting is separate from disconnect and never edits the client's config.
    let client = read_client(&find_def(id).ok_or("Unknown client location")?);
    if client.error.is_some() || client.gateway_installed {
        return Err("Disconnect this client location before removing it".into());
    }
    let _ = row;
    rows.retain(|row| row.id != id);
    crate::registry::atomic_write(&store, &serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?)?;
    Ok(())
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::registry::conduit_dir()
        .ok_or("Could not resolve local data folder")?
        .join("client-locations.json"))
}

pub(super) fn saved() -> Result<Vec<ClientLocation>, String> {
    match std::fs::read(store_path()?) {
        Ok(bytes) => {
            let values: Vec<serde_json::Value> = serde_json::from_slice(&bytes)
                .map_err(|_| "Could not read saved client locations".to_string())?;
            let mut rows = Vec::new();
            for value in values {
                let Ok(mut row) = serde_json::from_value::<ClientLocation>(value) else {
                    eprintln!("toolport: skipping saved client location with invalid fields");
                    continue;
                };
                if !valid_id(&row.id) || !row.config_path.is_absolute()
                    || row.id.split_once('@').map(|(kind, _)| kind) != Some(row.client_type.as_str())
                    || !defs().iter().any(|def| def.id == row.client_type && !def.uses_connectors)
                {
                    eprintln!("toolport: skipping saved client location with invalid identity, type or path");
                    continue;
                }
                // Earlier versions did not store the manual flag for path-derived IDs.
                row.manual |= row.id.split_once('@').is_some_and(|(_, key)|
                    key.len() == 16 && key.bytes().all(|byte| byte.is_ascii_hexdigit()));
                let kind = defs().into_iter().find(|def| def.id == row.client_type).unwrap();
                row.name = location_name(kind.name, row.config_path.parent()
                    .and_then(|path| path.file_name()).unwrap_or_default().to_string_lossy().as_ref());
                rows.push(row);
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
        if !std::fs::symlink_metadata(home.join(&folder)).is_ok_and(|metadata| metadata.is_dir()) {
            continue;
        }
        if !std::fs::symlink_metadata(&config_path).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        rows.push(ClientLocation {
            id: format!("claude-code@{}", folder_key(&folder)),
            manual: false,
            client_type: "claude-code".into(),
            name: location_name("Claude Code", &folder),
            config_path,
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

pub(super) fn all() -> Vec<ClientLocation> {
    let known = saved().unwrap_or_else(|error| {
        eprintln!("toolport: {error}");
        Vec::new()
    });
    merge_locations(known, home().map(|home| automatic(&home)).unwrap_or_default())
}

fn merge_locations(mut known: Vec<ClientLocation>, detected: Vec<ClientLocation>) -> Vec<ClientLocation> {
    for row in detected {
        if !known.iter().any(|saved| saved.id == row.id || saved.config_path == row.config_path) {
            known.push(row);
        }
    }
    known
}

fn claude_folder_id(home: &Path, folder: &Path) -> Option<String> {
    let name = folder.file_name()?.to_str()?;
    (folder.parent() == Some(home) && claude_profile_name(name))
        .then(|| format!("claude-code@{}", folder_key(name)))
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
    if !claude_profile_name(&folder) || folder_key(&folder) != key {
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
        manual: false,
        client_type: "claude-code".into(),
        name: location_name("Claude Code", &label),
        config_path: path.join(".claude.json"),
    })
}

pub(super) fn remember(row: &ClientLocation) -> Result<(), String> {
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    if let Some(existing) = rows.iter_mut().find(|saved| saved.id == row.id) {
        if !row.manual || existing.manual { return Ok(()); }
        existing.manual = true;
    } else {
        rows.push(row.clone());
    }
    {
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
    if let Some(def) = defs().into_iter().find(|def| {
        def.id == client_type && (def.path)().is_some_and(|existing| same_path(&existing, &path))
    }) {
        return Ok(def.id.into());
    }
    if let Some(row) = all()
        .into_iter()
        .find(|row| same_path(&row.config_path, &path))
    {
        if row.client_type != client_type {
            return Err("This config file is already registered for another client type".into());
        }
        let mut row = row;
        row.manual = true;
        remember(&row)?;
        return Ok(row.id);
    }
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    if let Some(row) = rows.iter_mut().find(|row| same_path(&row.config_path, &path)) {
        if row.client_type != client_type {
            return Err("This config file is already registered for another client type".into());
        }
        row.manual = true;
        let id = row.id.clone();
        crate::registry::atomic_write(&store, &serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?)?;
        return Ok(id);
    }
    use sha2::{Digest, Sha256};
    let key = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    let automatic_id = if client_type == "claude-code" {
        home().and_then(|root| std::fs::canonicalize(root).ok())
            .and_then(|root| claude_folder_id(&root, &folder))
    } else { None };
    let id = automatic_id.unwrap_or_else(|| format!("{client_type}@{}", &key[..16]));
    let label = folder.file_name().unwrap_or_default().to_string_lossy();
    rows.push(ClientLocation {
        id: id.clone(),
        manual: true,
        client_type: client_type.into(),
        name: location_name(&kind.name, &label),
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
    fn invalid_saved_rows_do_not_discard_valid_locations() {
        let _data = crate::registry::DataDirTestEnv::new("location-invalid-row");
        let folder = TempFolder::new();
        let id = add_client_location("codex", folder.path().to_str().unwrap()).unwrap();
        let store = store_path().unwrap();
        let mut rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
        rows.push(serde_json::json!({"id":"bad"}));
        rows.push(serde_json::json!({"id":"codex@bad", "clientType":"codex", "name":"bad", "configPath":"relative"}));
        std::fs::write(&store, serde_json::to_vec(&rows).unwrap()).unwrap();
        let rows = saved().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert!(find_def(&id).is_some());
    }

    #[test]
    fn removing_manual_locations_preserves_config_and_requires_disconnect() {
        let _data = crate::registry::DataDirTestEnv::new("location-remove");
        let folder = TempFolder::new();
        let path = folder.path().join(".claude.json");
        let text = r#"{"mcpServers":{"toolport":{"command":"/usr/bin/toolport-gateway"}}}"#;
        std::fs::write(&path, text).unwrap();
        let id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        assert!(read_client(&find_def(&id).unwrap()).location_removable);
        assert!(remove_client_location(&id).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::write(&path, "{}").unwrap();
        remove_client_location(&id).unwrap();
        assert!(saved().unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{}");
    }

    #[test]
    fn backup_folders_are_never_automatic_clients() {
        let root = TempFolder::new();
        for name in [".claude.bak", ".claudesync", ".claude-backup-2026", ".claude-work.bak", ".claude-old", ".claude2", ".claude-work"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
            std::fs::write(root.path().join(name).join(".claude.json"), "{}").unwrap();
        }
        let rows = automatic(root.path());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.id == "claude-code@claude2"));
    }

    #[test]
    fn folder_labels_and_encoded_ids_survive_the_audit_recording_path() {
        let _data = crate::registry::DataDirTestEnv::new("location-audit-record");
        let folder = TempFolder::new();
        for label in ["claude2".to_string(), "claude-work@office".to_string(), format!("claude-{}", "x".repeat(210))] {
            let path = folder.path().join(&label);
            std::fs::create_dir(&path).unwrap();
            let id = format!("claude-code@{}", folder_key(&label));
            remember(&ClientLocation { id: id.clone(), manual: true, client_type: "claude-code".into(),
                name: location_name("Claude Code", &label), config_path: path.join(".claude.json") }).unwrap();
            let principal = format!("adapter:{id}");
            crate::audit::record_internal("search", 1, false, Some(&principal), true);
            let rows = crate::audit::read_all().unwrap();
            let row = rows.last().unwrap();
            assert_eq!(row["client"], principal);
            assert_ne!(row["clientName"], "[private]");
            assert!(row["clientName"].as_str().unwrap().starts_with("Claude Code ("));
        }
        assert_eq!(crate::session_observability::display_label("reported@label").as_deref(), Some("[private]"));
    }

    #[test]
    fn location_disconnect_preserves_a_foreign_entry_using_the_gateway_binary() {
        let _data = crate::registry::DataDirTestEnv::new("location-restore-foreign");
        let folder = TempFolder::new();
        let path = folder.path().join(".claude.json");
        let original = serde_json::json!({"mcpServers":{
            "toolport":{"command":"/usr/bin/toolport-gateway", "env":{"TOOLPORT_CLIENT_ID":"claude-code"}},
            "foreign":{"command":"/usr/bin/toolport-gateway", "args":["--http","9999"]}}});
        std::fs::write(&path, original.to_string()).unwrap();
        let id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        let entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport", "transport":"stdio", "command":"/usr/bin/toolport-gateway", "env":[{"key":"TOOLPORT_CLIENT_ID", "value":id, "secret":false}]})).unwrap();
        install_or_remove(&id, Some(&entry)).unwrap();
        uninstall_gateway(&id).unwrap();
        let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["foreign"], original["mcpServers"]["foreign"]);
        assert!(after["mcpServers"].get("toolport").is_none());
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
            std::fs::canonicalize(folder.path()).unwrap().join("config.toml")
        );
        assert_eq!(
            discovery_capabilities(&id).auto_mode,
            discovery_capabilities("codex").auto_mode
        );
        assert!(!crate::registry::resolved_path().unwrap().exists());
        assert!(add_client_location("codex", "relative").is_err());
        assert!(!valid_id("codex@../escape"));
        let claude_id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        let claude = read_client(&find_def(&claude_id).unwrap());
        assert!(claude.app_present);
        assert!(!claude.config_exists);
    }

    #[test]
    fn environment_selected_location_has_one_identity_for_upkeep() {
        let base = defs().into_iter().find(|def| def.id == "codex").unwrap();
        let mut location = defs().into_iter().find(|def| def.id == "codex").unwrap();
        location.location = Some(ClientLocation {
            id: "codex@selected".into(),
            manual: true,
            client_type: "codex".into(),
            name: "Codex (selected)".into(),
            config_path: resolved_definition_path(&base).unwrap(),
        });
        let rows = unique_config_definitions(vec![base, location]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].client_id(), "codex");
    }

    #[test]
    fn a_manual_claude_folder_keeps_its_identity_when_the_config_appears() {
        let root = TempFolder::new();
        let folder = root.path().join(".claude4");
        std::fs::create_dir(&folder).unwrap();
        let id = claude_folder_id(root.path(), &folder).unwrap();
        assert_eq!(id, "claude-code@claude4");
        assert!(automatic(root.path()).is_empty());
        let saved = ClientLocation {
            id: id.clone(),
        manual: true, client_type: "claude-code".into(),
            name: "Claude Code (claude4)".into(), config_path: folder.join(".claude.json"),
        };
        std::fs::write(&saved.config_path, "{}").unwrap();
        let rows = merge_locations(vec![saved.clone()], automatic(root.path()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        // A previously registered ID also wins over newly discovered metadata.
        let old_id = ClientLocation { id: "claude-code@registered".into(), ..saved };
        let rows = merge_locations(vec![old_id], automatic(root.path()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "claude-code@registered");
        assert!(claude_folder_id(root.path(), &folder.join("nested")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn an_environment_path_alias_does_not_duplicate_a_registered_location() {
        let data = crate::registry::DataDirTestEnv::new("location-path-alias");
        let folder = data.dir.join("real");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("config.toml"), "").unwrap();
        std::os::unix::fs::symlink(&folder, data.dir.join("alias")).unwrap();
        fn alias_path() -> Option<PathBuf> {
            Some(crate::registry::conduit_dir()?.join("alias/config.toml"))
        }
        let mut base = defs().into_iter().find(|def| def.id == "codex").unwrap();
        base.path = alias_path;
        let id = add_client_location("codex", folder.to_str().unwrap()).unwrap();
        let location = find_def(&id).unwrap();
        let rows = unique_config_definitions(vec![base, location]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].client_id(), "codex");
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
        for id in ["work", "personal"] {
            registry.profiles.push(serde_json::from_value(
                serde_json::json!({"id": id, "name": id}),
            ).unwrap());
        }
        registry.client_scopes.insert("claude-code".into(), "work".into());
        crate::registry::save(&registry).unwrap();
        let text = r#"{"mcpServers":{"other":{"env":{"TOOLPORT_CLIENT_ID":"unrelated"}},"toolport":{"command":"toolport-gateway","env":{"TOOLPORT_CLIENT_ID":"claude-code"}}}}"#;
        inherit_location_access("claude-code@claude2", Format::JsonMcpServers, "toolport", text).unwrap();
        let mut registry = crate::registry::load().unwrap();
        assert_eq!(registry.client_scopes.get("claude-code@claude2"), Some(&"work".into()));
        registry.client_scopes.insert("claude-code@claude2".into(), "personal".into());
        crate::registry::save(&registry).unwrap();
        inherit_location_access("claude-code@claude2", Format::JsonMcpServers, "toolport", text).unwrap();
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
