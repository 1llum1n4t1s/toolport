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
    defs().into_iter().filter(|def| !def.uses_connectors).filter_map(|def| {
        let file = (def.path)()?.file_name()?.to_str()?.to_string();
        Some(ClientLocationType { id: def.id.into(), name: def.name.into(), config_file: file })
    }).collect()
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::registry::conduit_dir().ok_or("Could not resolve local data folder")?.join("client-locations.json"))
}

fn saved() -> Result<Vec<ClientLocation>, String> {
    match std::fs::read(store_path()?) {
        Ok(bytes) => {
            let rows: Vec<ClientLocation> = serde_json::from_slice(&bytes)
                .map_err(|_| "Could not read saved client locations".to_string())?;
            if rows.iter().any(|row| !valid_id(&row.id) || !row.config_path.is_absolute()
                || !defs().iter().any(|def| def.id == row.client_type && !def.uses_connectors)) {
                return Err("Invalid saved client location".into());
            }
            Ok(rows)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("Could not read saved client locations".into()),
    }
}

fn valid_id(id: &str) -> bool {
    id.contains('@') && id.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.@%".contains(&c))
}

fn folder_key(folder: &str) -> String {
    folder.trim_start_matches('.').bytes().map(|byte| {
        if byte.is_ascii_alphanumeric() || b"-_".contains(&byte) { (byte as char).to_string() }
        else { format!("%{byte:02X}") }
    }).collect()
}

fn automatic(home: &Path) -> Vec<ClientLocation> {
    let primary = client_config_path_with_home("claude-code", Some(home.to_path_buf()));
    automatic_with_primary(home, primary.as_deref())
}

fn automatic_with_primary(home: &Path, primary: Option<&Path>) -> Vec<ClientLocation> {
    let mut rows = Vec::new();
    for folder in claude_profile_dirs(home).unwrap_or_default() {
        let config_path = home.join(&folder).join(".claude.json");
        if !config_path.is_file() || primary.is_some_and(|path| same_path(path, &config_path)) { continue; }
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
    a == b || std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(a, b)| a == b)
}

pub(super) fn all() -> Vec<ClientLocation> {
    let mut rows = home().map(|home| automatic(&home)).unwrap_or_default();
    match saved() {
        Ok(saved) => for row in saved {
            if !rows.iter().any(|auto| same_path(&auto.config_path, &row.config_path)) { rows.push(row); }
        },
        Err(error) => eprintln!("toolport: {error}"),
    }
    rows
}

/// A known automatic ID resolves without enumerating home or reading a config.
pub(super) fn lookup(id: &str) -> Option<ClientLocation> {
    if !valid_id(id) { return None; }
    if let Some(row) = saved().ok()?.into_iter().find(|row| row.id == id) { return Some(row); }
    let key = id.strip_prefix("claude-code@")?;
    let folder = format!(".{key}");
    if !folder.starts_with(".claude") || folder_key(&folder) != key { return None; }
    let root = home()?;
    let path = root.join(&folder);
    // Do not follow an automatically discovered directory outside home later.
    if !std::fs::symlink_metadata(&path).ok()?.is_dir() { return None; }
    Some(ClientLocation { id: id.into(), client_type: "claude-code".into(),
        name: format!("Claude Code ({key})"), config_path: path.join(".claude.json") })
}

pub fn add_client_location(client_type: &str, folder: &str) -> Result<String, String> {
    let kind = client_location_types().into_iter().find(|kind| kind.id == client_type)
        .ok_or("Choose a supported client type")?;
    let raw = PathBuf::from(folder.trim());
    let folder = if folder.trim() == "~" { home().ok_or("Could not resolve home")? }
        else if let Some(relative) = folder.trim().strip_prefix("~/") { home().ok_or("Could not resolve home")?.join(relative) }
        else { raw };
    if !folder.is_absolute() { return Err("Choose an absolute client config folder".into()); }
    let folder = std::fs::canonicalize(&folder).map_err(|_| "Choose an existing client config folder")?;
    if !folder.is_dir() { return Err("Choose a client config folder, not a file".into()); }
    let path = folder.join(&kind.config_file);
    if let Some(def) = defs().into_iter().find(|def| (def.path)().is_some_and(|existing| same_path(&existing, &path))) {
        return Ok(def.id.into());
    }
    let store = store_path()?;
    let _lock = crate::registry::lock_at(&store)?;
    let mut rows = saved()?;
    if let Some(row) = all().into_iter().find(|row| same_path(&row.config_path, &path)) { return Ok(row.id); }
    use sha2::{Digest, Sha256};
    let key = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    let id = format!("{client_type}@{}", &key[..16]);
    let label = folder.file_name().unwrap_or_default().to_string_lossy();
    rows.push(ClientLocation { id: id.clone(), client_type: client_type.into(),
        name: format!("{} ({})", kind.name, label.trim_start_matches('.')), config_path: path });
    crate::registry::atomic_write(&store, &serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_is_shallow_name_filtered_and_does_not_parse_configs() {
        let root = tempfile::tempdir().unwrap();
        for folder in [".claude", ".claude2", ".claude3", ".claude-work", ".claude-empty", "unrelated/.claude4"] {
            std::fs::create_dir_all(root.path().join(folder)).unwrap();
            if folder != ".claude-empty" { std::fs::write(root.path().join(folder).join(".claude.json"), "not JSON").unwrap(); }
        }
        let rows = automatic_with_primary(root.path(), None);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().any(|row| row.id == "claude-code@claude2" && row.name == "Claude Code (claude2)"));
        assert_eq!(automatic_with_primary(root.path(), Some(&root.path().join(".claude2/.claude.json"))).len(), 3);
        assert_ne!(folder_key(".claude+a"), folder_key(".claude-a"));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_symlinks_outside_home() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join(".claude.json"), "{}").unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join(".claude-outside")).unwrap();
        assert!(automatic_with_primary(home.path(), None).is_empty());
    }

    #[test]
    fn manual_locations_are_stable_deduplicated_and_separate_from_registry() {
        let _data = crate::registry::DataDirTestEnv::new("client-locations");
        let folder = tempfile::tempdir().unwrap();
        let id = add_client_location("codex", folder.path().to_str().unwrap()).unwrap();
        assert_eq!(add_client_location("codex", folder.path().to_str().unwrap()).unwrap(), id);
        assert_eq!(saved().unwrap().len(), 1);
        let def = find_def(&id).unwrap();
        assert_eq!(resolved_definition_path(&def).unwrap(), folder.path().join("config.toml"));
        assert_eq!(discovery_capabilities(&id).auto_mode, discovery_capabilities("codex").auto_mode);
        assert!(!crate::registry::resolved_path().unwrap().exists());
        assert!(add_client_location("codex", "relative").is_err());
        assert!(!valid_id("codex@../escape"));
    }

    #[test]
    fn existing_manual_gateway_is_adopted_without_duplicate_entries() {
        let _data = crate::registry::DataDirTestEnv::new("client-location-adoption");
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(".claude.json");
        std::fs::write(&path, r#"{"mcpServers":{"toolport":{"command":"/usr/bin/toolport-gateway","env":{"TOOLPORT_CLIENT_ID":"claude-code"}},"other":{"command":"other"}}}"#).unwrap();
        let id = add_client_location("claude-code", folder.path().to_str().unwrap()).unwrap();
        let def = find_def(&id).unwrap();
        let detected = read_client(&def);
        assert_eq!(resolve_entry_state(&detected.servers, None), GatewayEntryState::Managed);
        let mut entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport", "transport":"stdio", "env":[{"key":"TOOLPORT_CLIENT_ID", "value":id, "secret":false}]})).unwrap();
        entry.command = Some("/usr/bin/toolport-gateway".into());
        install_or_remove(&id, Some(&entry)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcpServers"].as_object().unwrap().len(), 2);
        assert_eq!(value["mcpServers"]["toolport"]["env"]["TOOLPORT_CLIENT_ID"], id);
        assert!(backup_dir(&id).unwrap().is_dir());
        assert_eq!(known_adapter_name(&id).unwrap(), detected.name);
    }

    #[test]
    #[ignore = "scan benchmark; run explicitly with --ignored --nocapture"]
    fn scan_timings() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..10_000 { std::fs::write(root.path().join(format!("unrelated-{i}")), "").unwrap(); }
        for i in 0..20 {
            let dir = root.path().join(format!(".claude{i}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join(".claude.json"), "{}").unwrap();
        }
        fn measure(home: &Path) {
            let start = std::time::Instant::now();
            for _ in 0..20 {
                let dirs: Vec<String> = std::fs::read_dir(home).unwrap().filter_map(Result::ok)
                    .filter(|entry| entry.path().is_dir()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
                let _ = claude_code_config_paths_from(home, None, &dirs).into_iter().filter(|path| path.is_file()).count();
            }
            let before = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
            let start = std::time::Instant::now();
            for _ in 0..20 { let _ = automatic_with_primary(home, None); }
            println!("entries={} before_ms={before:.3} after_ms={:.3}", std::fs::read_dir(home).unwrap().count(), start.elapsed().as_secs_f64() * 1000.0 / 20.0);
        }
        measure(root.path());
        // Only directory metadata; no config contents or writes in real home.
        if let Some(real_home) = home() { measure(&real_home); }
    }
}
