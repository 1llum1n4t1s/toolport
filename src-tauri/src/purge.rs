//! Explicit data removal. Package removal and upgrades never call this module.
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub data_dir: String,
    pub resources: Vec<String>,
    pub report_path: String,
}

#[derive(Debug, Serialize)]
pub struct Leftover {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub clients: Vec<crate::clients::DisconnectResult>,
    pub removed: Vec<String>,
    pub leftovers: Vec<Leftover>,
    pub preserved: Vec<String>,
    pub notes: Vec<String>,
}

const AUTOSTART_NAMES: &[&str] = &["Toolport", "Conduit", "conduit", "ToolportNativePreview"];

/// Removal never flushes telemetry: doing so could recreate the deleted logs.
pub fn exit_after_removal(status: i32) -> ! {
    std::process::exit(status)
}

pub fn wait_for_desktop_exit() -> Result<(), String> {
    wait_for_exit_pipe(std::io::stdin(), std::time::Duration::from_secs(30))
}

fn wait_for_exit_pipe(
    mut input: impl std::io::Read + Send + 'static,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("purge-desktop-exit".into())
        .spawn(move || {
            let result = std::io::copy(&mut input, &mut std::io::sink())
                .map(|_| ())
                .map_err(|error| format!("Desktop exit pipe: {error}"));
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver.recv_timeout(timeout).map_err(|error| {
        format!("Toolport did not close within {} seconds ({error}). Nothing was removed. Close Toolport and retry.", timeout.as_secs())
    })?
}

fn data_dir() -> Result<PathBuf, String> {
    let dir =
        crate::registry::conduit_dir().ok_or("Could not resolve Toolport's data directory")?;
    let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    let shared = [
        dirs::home_dir(),
        dirs::config_dir(),
        dirs::data_dir(),
        Some(std::env::temp_dir()),
    ];
    if !dir.is_absolute()
        || canonical.parent().is_none()
        || shared
            .into_iter()
            .flatten()
            .any(|path| canonical == std::fs::canonicalize(&path).unwrap_or(path))
        || std::fs::symlink_metadata(&dir).is_ok_and(|metadata| is_link(&metadata))
    {
        return Err(format!("Refusing unsafe data directory: {}", dir.display()));
    }
    Ok(dir)
}

fn verify_data_ownership(dir: &Path) -> Result<(), String> {
    verify_ownership(dir, crate::brand::env_var("TOOLPORT_DATA_DIR", "CONDUIT_DATA_DIR").is_some())
}

fn verify_ownership(dir: &Path, overridden: bool) -> Result<(), String> {
    if !overridden && dir.file_name().is_some_and(|name| {
        matches!(
            name.to_str(),
            Some("Toolport" | "Conduit" | "Toolport-dev" | "Conduit-dev")
        )
    }) {
        return Ok(());
    }
    let owned = std::fs::read(dir.join("registry.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| {
            value["version"].is_u64() && value["servers"].is_array() && value["profiles"].is_array()
        });
    if owned {
        Ok(())
    } else {
        Err(format!(
            "Could not verify Toolport ownership of custom data directory {}. Nothing was removed.",
            dir.display()
        ))
    }
}

fn autostart_files(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let (base, ext) = (home.join("Library/LaunchAgents"), "plist");
    #[cfg(not(target_os = "macos"))]
    let (base, ext) = (home.join(".config/autostart"), "desktop");
    AUTOSTART_NAMES
        .iter()
        .map(|name| base.join(format!("{name}.{ext}")))
        .collect()
}

fn install_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut bases = vec![dir.parent().map(Path::to_path_buf), dirs::config_dir()];
    #[cfg(windows)]
    bases.push(dirs::home_dir().map(|home| crate::brand::windows_roaming_base(&home)));
    #[cfg(not(windows))]
    let _ = &mut bases;
    bases.into_iter().flatten().flat_map(|base| {
        ["Toolport", "Conduit", "Toolport-dev", "Conduit-dev"].map(|name| base.join(name))
    }).collect()
}

fn other_installs(dir: &Path, candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut others = Vec::new();
    for path in candidates {
        if path.is_dir() && std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()) != canonical
            && std::fs::read_dir(&path).map_or(true, |mut entries| entries.next().is_some())
            && !others.contains(&path)
        {
            others.push(path);
        }
    }
    others
}

fn credential_scope(dir: &Path, others: &[PathBuf], all_installs: bool) -> Result<crate::secrets::purge::Scope, String> {
    if all_installs {
        return Ok(crate::secrets::purge::Scope { all_installs: true, server_ids: Default::default() });
    }
    let registry_path = dir.join("registry.json");
    let mut ids = std::collections::HashSet::new();
    if registry_path.exists() {
        // Raw read: loading a registry can migrate it or create recovery files.
        let error = |error| format!("Credential ownership inventory for {}: {error}", registry_path.display());
        let registry: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry_path).map_err(|e| error(e.to_string()))?)
            .map_err(|e| error(e.to_string()))?;
        let servers = registry["servers"].as_array().ok_or_else(|| error("missing servers array".into()))?;
        ids.extend(servers.iter().filter_map(|server| server["id"].as_str().map(str::to_owned)));
    }
    for other in others {
        let registry_path = other.join("registry.json");
        let error = |error| format!("Credential ownership inventory for {}: {error}. Credentials were not removed; the selected registry was retained. Repair this registry and retry.", registry_path.display());
        let registry: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry_path).map_err(|e| error(e.to_string()))?)
            .map_err(|e| error(e.to_string()))?;
        let servers = registry["servers"].as_array().ok_or_else(|| error("missing servers array".into()))?;
        for server in servers {
            let id = server["id"].as_str().ok_or_else(|| error("missing server id".into()))?;
            ids.remove(id);
        }
    }
    Ok(crate::secrets::purge::Scope { all_installs, server_ids: ids })
}

fn app_resources(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    return ["Library/Caches/com.tsout.conduit", "Library/HTTPStorages/com.tsout.conduit",
        "Library/Preferences/com.tsout.conduit.plist", "Library/Saved Application State/com.tsout.conduit.savedState",
        "Library/WebKit/com.tsout.conduit"].map(|path| home.join(path)).into();
    #[cfg(target_os = "windows")]
    return vec![home.join("AppData/Local/com.tsout.conduit")];
    #[cfg(target_os = "linux")]
    {
        let _ = home;
        [dirs::cache_dir(), dirs::data_local_dir()].into_iter().flatten()
            .flat_map(|base| [base.join("com.tsout.conduit")]).collect()
    }
}

fn remove_app_resources(report: &mut Report, shared: bool) {
    let Some(home) = dirs::home_dir() else { return; };
    remove_resources(app_resources(&home), shared, report);
}

fn remove_resources(paths: Vec<PathBuf>, shared: bool, report: &mut Report) {
    for path in paths {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                report.leftovers.push(Leftover { path: path.display().to_string(), error: error.to_string() });
                continue;
            }
        };
        if shared {
            report.preserved.push(format!("{} (shared app storage; another install or custom data directory is in use)", path.display()));
        } else {
            if metadata.is_dir() && !is_link(&metadata) {
                remove_contents(&path, &[], report);
            }
            remove_path(&path, report);
        }
    }
}

pub fn plan() -> Result<Plan, String> { plan_with_scope(false) }

pub fn plan_with_scope(all_installs: bool) -> Result<Plan, String> {

    let dir = data_dir()?;
    let home = dirs::home_dir().ok_or("Could not resolve your home directory")?;
    let report = home.join(format!(
        "Toolport-removal-report-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let mut resources = vec![
        format!("Toolport-created entries in {}: registry, settings, logs, caches, migration exports, client backups, encrypted secrets and published gateways in bin/. Other entries are preserved.", dir.display()),
        if all_installs { "ALL credentials in the shared conduit-mcp service for every Toolport install, including master keys and Team tokens. Other installations will lose access to these credentials.".into() }
        else { "Only accounts owned by this registry, excluding server ids used by detected other installs. Shared and unknown credentials are preserved because they may belong to another Toolport install. To remove them too, use toolport-gateway --remove-data --all-installs --confirm in the CLI.".into() },
        "Toolport's daemon for this data directory, after all active sessions have closed".into(),
    ];
    #[cfg(target_os = "windows")]
    resources.extend(AUTOSTART_NAMES.iter().map(|name| format!("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\{name} and HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run\\{name} (only Toolport commands)")));
    #[cfg(not(target_os = "windows"))]
    resources.extend(
        autostart_files(&home)
            .iter()
            .map(|path| format!("{} (only Toolport's startup entry)", path.display())),
    );
    resources.extend(app_resources(&home).iter().map(|path| format!("{} (app cache/preferences/WebView storage; preserved when shared)", path.display())));
    resources.extend(other_installs(&dir, install_candidates(&dir)).iter().map(|path| format!("{} (other install detected; its data is preserved)", path.display())));
    Ok(Plan {
        data_dir: dir.display().to_string(),
        resources,
        report_path: report.display().to_string(),
    })
}

/// The pipe stays open until the entire desktop process exits, including workers.
/// No timing assumption about desktop shutdown is needed by the child.
#[cfg(any(feature = "desktop", feature = "gtk-desktop"))]
pub fn launch_after_exit(report_path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let home = dirs::home_dir().ok_or("Could not resolve your home directory")?;
    if report_path.parent() != Some(home.as_path())
        || !report_path.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with("Toolport-removal-report-")
        })
    {
        return Err("Invalid removal report path".into());
    }
    let binary = crate::gateway_publish::bundled_gateway_source().ok_or(
        "Could not find the bundled gateway. Use toolport-gateway --remove-data --confirm instead.",
    )?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let report = options
        .open(report_path)
        .map_err(|error| error.to_string())?;
    let mut command = Command::new(binary);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command
        .args(["--remove-data", "--confirm", "--after-desktop-exit"])
        .stdin(Stdio::piped())
        .stdout(report)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    // Deliberately held by the OS until process exit. Child waits for EOF.
    if let Some(pipe) = child.stdin.take() {
        std::mem::forget(pipe);
    }
    Ok(())
}

pub fn run() -> Result<Report, String> { run_with_scope(false) }

pub fn run_with_scope(all_installs: bool) -> Result<Report, String> {
    let dir = data_dir()?;
    let others = other_installs(&dir, install_candidates(&dir));
    let scope = credential_scope(&dir, &others, all_installs)?;
    // The explicit global option needs no account inventory from a corrupt registry.
    // Cleanup still selects only Toolport-created names in this folder.
    if !all_installs || !dir.join("registry.json").is_file() {
        verify_data_ownership(&dir)?;
    }
    let mut report = run_with(
        |dir| {
            let mut blockers = crate::gateway_publish::purge_blockers(dir);
            blockers.extend(crate::daemon::purge_idle_blockers(dir));
            blockers
        },
        || crate::clients::disconnect_all_for_purge(all_installs),
        |dir| {
            let mut blockers = crate::daemon::stop_for_purge(dir);
            blockers.extend(crate::gateway_publish::purge_blockers(dir));
            blockers
        },
        || crate::secrets::purge::remove(&scope),
        |report| startup_entries(false, report),
        remove_autostart,
    )?;
    if report.leftovers.is_empty() {
        remove_app_resources(&mut report, !others.is_empty() || crate::brand::env_var("TOOLPORT_DATA_DIR", "CONDUIT_DATA_DIR").is_some());
    }
    Ok(report)
}

fn run_with(
    preflight: impl FnOnce(&Path) -> Vec<Leftover>,
    disconnect: impl FnOnce() -> Result<Vec<crate::clients::DisconnectResult>, String>,
    stop: impl FnOnce(&Path) -> Vec<Leftover>,
    keyring: impl FnOnce() -> Result<crate::secrets::purge::Removal, String>,
    inventory_startup: impl FnOnce(&mut Report) -> Result<(), String>,
    autostart: impl FnOnce(&mut Report) -> Result<(), String>,
) -> Result<Report, String> {
    use fs2::FileExt;
    let dir = data_dir()?;
    let mut report = Report::default();
    if !dir.exists() {
        return Err(format!(
            "No Toolport data directory at {}. Nothing was removed.",
            dir.display()
        ));
    }
    report.leftovers.extend(preflight(&dir));
    inventory_startup(&mut report)?;
    if !report.leftovers.is_empty() {
        report.notes.push("Preflight failed. Clients, data, credentials and startup entries are unchanged.".into());
        return Ok(report);
    }
    let owner_path = dir.join("broker/owner.lock");
    if std::fs::symlink_metadata(dir.join("broker"))
        .is_ok_and(|metadata| is_link(&metadata))
        || std::fs::symlink_metadata(&owner_path)
            .is_ok_and(|metadata| is_link(&metadata))
    {
        return Err(format!(
            "Refusing symlinked broker ownership path: {}",
            owner_path.display()
        ));
    }
    std::fs::create_dir_all(owner_path.parent().unwrap()).map_err(|error| error.to_string())?;
    let owner = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&owner_path)
        .map_err(|error| format!("{}: {error}", owner_path.display()))?;
    owner.try_lock_exclusive().map_err(|error| {
        format!(
            "Close Toolport before removing data. {}: {error}",
            owner_path.display()
        )
    })?;
    report.clients = disconnect()?;
    if report
        .clients
        .iter()
        .any(|client| client.error.is_some() || !client.warnings.is_empty())
    {
        for client in &report.clients {
            if let Some(error) = &client.error {
                report.leftovers.push(Leftover {
                    path: client.path.clone(),
                    error: error.clone(),
                });
            }
            if !client.warnings.is_empty() {
                report.leftovers.push(Leftover {
                    path: client.path.clone(),
                    error: client.warnings.join("; "),
                });
            }
        }
        report.leftovers.push(Leftover { path: dir.display().to_string(), error: "Client restoration failed or retained an edited connection. All Toolport data and credentials were retained for recovery.".into() });
        return Ok(report);
    }
    report.leftovers.extend(stop(&dir));
    if !report.leftovers.is_empty() {
        report.leftovers.push(Leftover {
            path: dir.display().to_string(),
            error: "Close the listed sessions and retry. Data and credentials were retained."
                .into(),
        });
        return Ok(report);
    }
    // Finish startup cleanup before touching credentials. Foreign entries are informational.
    autostart(&mut report)?;
    if !report.leftovers.is_empty() {
        report.notes.push("Startup cleanup failed. Data and credentials were retained for retry.".into());
        return Ok(report);
    }
    let mut team_token_in_file = false;
    match keyring() {
        Ok(removal) => {
            team_token_in_file = removal.team_token_in_file;
            if removal.team_token_removed {
                report.notes.push("Team tokens deleted from this machine were removed locally only; no server-side revoke endpoint is available in the Teams client.".into());
            }
            report.removed.extend(removal.removed);
            if !removal.preserved.is_empty() {
                report.notes.push("Preserved credentials may belong to another Toolport install. To remove all shared credentials, use toolport-gateway --remove-data --all-installs --confirm.".into());
            }
            report.preserved.extend(removal.preserved);
            report.notes.extend(removal.notes);
            report.leftovers.extend(removal.leftovers);
        }
        Err(error) => report.leftovers.push(Leftover {
            path: "keychain:conduit-mcp (inventory unavailable)".into(), error,
        }),
    }
    if !report.leftovers.is_empty() {
        report.notes.push("Credential cleanup failed. Successfully removed items are listed under removed; failed items remain. Data and the registry were retained so retry uses the same ownership inventory.".into());
        return Ok(report);
    }
    let registry_path = dir.join("registry.json");
    remove_data_contents(&dir, &[owner_path.clone(), registry_path.clone()], &mut report);
    if team_token_in_file && report.removed.contains(&dir.join("secrets.enc").display().to_string()) {
        report.notes.push("Team tokens deleted from this machine were removed locally only; no server-side revoke endpoint is available in the Teams client.".into());
    }
    if report.leftovers.is_empty() {
        remove_path(&registry_path, &mut report);
    } else if registry_path.exists() {
        report.leftovers.push(Leftover {
            path: registry_path.display().to_string(),
            error: "Retained as ownership and recovery evidence for retry.".into(),
        });
    }
    // Windows cannot unlink an open owner lock. It is the last resource removed.
    drop(owner);
    remove_path(&owner_path, &mut report);
    remove_empty_dirs(&dir.join("broker"), &mut report);
    Ok(report)
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

fn remove_path(path: &Path, report: &mut Report) {
    let result = std::fs::symlink_metadata(path).and_then(|metadata| {
        // Windows reports directory junctions as links, so is_dir() is false.
        // RemoveDirectory unlinks them without traversing their target.
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let directory = metadata.is_dir() && !is_link(&metadata);
        if directory {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        }
    });
    match result {
        Ok(()) => report.removed.push(path.display().to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.leftovers.push(Leftover {
            path: path.display().to_string(),
            error: error.to_string(),
        }),
    }
}

// Names come from registry/profile stores, clients, telemetry, daemon/broker,
// secrets, Teams, native-shell state and the gateway's persisted caches.
const DATA_FILES: &[&str] = &[
    "registry.json", "registry-recovery.json", "registry.personal-sync-status.json", "routines.json",
    "secrets.enc", "embeddings.json", "root-integrity-scopes.json",
    "tool-cache.json", "tool-pins.json", "quarantine.json",
    "audit.jsonl", "security.jsonl", "inspect.jsonl", "gateway.log", "daemon.log",
    "savings.jsonl", "savings-v2.jsonl", "savings-v3.jsonl",
    "search-trace.jsonl", "search-trace-v2.jsonl", "oauth-debug.log",
    "last-daemon-exit.json", "approval-endpoint.json", "secret-reference-approvals.json",
    "rate_limit_counters.json", "downstream_backoff.json", "team-sync-status.json",
    "http-bridge-update-resume.json", "gtk-security-dismissed.json", "gtk-window-state.json",
    ".secrets-file-migrated", ".keychain-dpk-migrated", ".tray-hint-shown",
    ".gtk-star-prompt-done", "gtk-onboarding-complete", "agent-activity-removed",
    "agent-guard-removed", "legacy-topology-retired.notice",
];

fn hex_name(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) && value.parse::<u128>().is_ok()
}

// registry::atomic_write and lock_at generate siblings of a known file, never
// siblings of a directory. Apply the same rule at every selected depth.
fn generated_file_name(name: &str, owned: fn(&str) -> bool) -> bool {
    if owned(name) { return true; }
    if let Some(base) = name.strip_suffix(".conduit-tmp") {
        return base.rsplit_once('.').is_some_and(|(base, seq)| {
            decimal(seq) && base.rsplit_once('.').is_some_and(|(base, pid)| {
                decimal(pid) && pid.parse::<u32>().is_ok() && owned(base)
            })
        });
    }
    name.strip_suffix(".lock").is_some_and(owned)
}

fn owned_data_name(name: &str) -> bool {
    generated_file_name(name, owned_data_file)
        || matches!(name, "secret-service.lock" | "task-handle-key.lock" | "client-config-mutation.lock")
        || name.strip_prefix("daemon-").and_then(|s| s.strip_suffix(".lock")).is_some_and(|s| hex_name(s, 64))
}

fn owned_data_file(name: &str) -> bool {
    if DATA_FILES.contains(&name) || matches!(name, "gateway.log.1" | "daemon.log.1" | "tool-cache.servers.json") {
        return true;
    }
    if let Some(suffix) = name.strip_prefix("registry.json.") {
        return matches!(suffix, "bak" | "bak.seq")
            || suffix.strip_prefix("bak.").is_some_and(decimal)
            || suffix.strip_prefix('v').and_then(|s| s.strip_suffix(".bak")).is_some_and(|s| {
                s.split_once('-').is_some_and(|(v, ms)| decimal(v) && v.parse::<u32>().is_ok() && decimal(ms))
            })
            || suffix.strip_prefix("unreadable-").is_some_and(|s| {
                decimal(s) || s.strip_prefix("sha256-").is_some_and(|hash| hex_name(hash, 64))
            });
    }
    if name.strip_prefix("daemon-").and_then(|s| s.strip_suffix(".json")).is_some_and(|s| hex_name(s, 64))
        || name.strip_prefix("team-activity-").and_then(|s| s.strip_suffix(".json")).is_some_and(|s| hex_name(s, 32)) {
        return true;
    }
    ["tool-cache-", "tool-pins-", "quarantine-"].iter().any(|prefix| {
        let Some(suffix) = name.strip_prefix(prefix) else { return false; };
        let key = suffix.strip_suffix(".json").and_then(|s| s.strip_prefix("v2-"));
        if let Some(key) = key {
            let key = if *prefix == "tool-cache-" { key.strip_suffix(".servers").unwrap_or(key) } else { key };
            return hex_name(key, 64);
        }
        // Pre-v2 stores used registry::slugify, without arbitrary punctuation.
        suffix.strip_suffix(".json").is_some_and(|slug| !slug.is_empty() && crate::registry::slugify(slug) == slug)
    })
}

fn gateway_file(name: &str) -> bool {
    if name == "gateway-manifest.json" { return true; }
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    ["toolport-gateway", "conduit-gateway"].iter().any(|base| {
        if stem == *base { return true; }
        let Some(suffix) = stem.strip_prefix(base).and_then(|s| s.strip_prefix('-')) else { return false; };
        if ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "x86_64-apple-darwin", "aarch64-apple-darwin"].contains(&suffix) {
            return true;
        }
        // Published versions, including prerelease versions and a 12-hex digest.
        let core = suffix.split(['-', '+']).next().unwrap_or_default();
        core.split('.').count() == 3 && core.split('.').all(decimal)
            && suffix.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    })
}

fn bin_file(name: &str) -> bool {
    if generated_file_name(name, gateway_file) { return true; }
    if let Some(base) = name.strip_prefix('.').and_then(|s| s.strip_suffix(".tmp")) {
        return base.rsplit_once('.').is_some_and(|(base, stamp)| decimal(stamp)
            && base.rsplit_once('.').is_some_and(|(base, pid)| decimal(pid) && gateway_file(base)));
    }
    // clients::refresh_legacy_gateway uses with_extension(<128-bit id>.tmp).
    name.strip_suffix(".tmp").and_then(|s| s.rsplit_once('.'))
        .is_some_and(|(base, id)| hex_name(id, 32) && gateway_file(base))
}

fn backup_file(name: &str) -> bool {
    name == "moved-servers.json"
        || name.strip_prefix("original-").and_then(|s| s.strip_suffix(".json")).is_some_and(|s| hex_name(s, 64))
        // clients::backup_file_named accepts a config basename, including overrides.
        || name.split_once('-').is_some_and(|(stamp, basename)| decimal(stamp) && !basename.is_empty())
}

fn export_file(name: &str) -> bool {
    [("rules-", ".md"), ("routines-", ".json"), ("agent-permissions-", ".json")].iter().any(|(prefix, ext)| {
        let Some(date) = name.strip_prefix(prefix).and_then(|s| s.strip_suffix(ext)) else { return false; };
        let parts: Vec<_> = date.split('-').collect();
        (parts.len() == 3 || parts.len() == 4) && parts[0].len() == 4 && decimal(parts[0])
            && parts[1].len() == 2 && parts[1].parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
            && parts[2].len() == 2 && parts[2].parse::<u8>().is_ok_and(|d| (1..=31).contains(&d))
            && (parts.len() == 3 || decimal(parts[3]) && parts[3].parse::<u32>().is_ok_and(|n| n >= 2))
    })
}

fn pid_json(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|s| decimal(s) && s.parse::<u32>().is_ok())
}

fn oauth_file(name: &str) -> bool {
    if let Some(hash) = name.strip_prefix("auth-write-").and_then(|s| s.strip_suffix(".lock")) {
        return hex_name(hash, 64);
    }
    [".lock", ".owner"].iter().any(|ext| name.strip_suffix(ext).is_some_and(|s| hex_name(s, 64)))
}

fn vector_file(name: &str) -> bool {
    hex_name(name, 64) || name.strip_suffix(".tmp").is_some_and(|base| {
        base.rsplit_once('.').is_some_and(|(base, seq)| decimal(seq)
            && base.rsplit_once('.').is_some_and(|(hash, pid)| hex_name(hash, 64) && decimal(pid)))
    })
}

#[derive(PartialEq)]
enum DataEntry { File, Directory }

fn owned_data_entry(relative: &Path) -> Option<DataEntry> {
    let parts: Option<Vec<_>> = relative.iter().map(|part| part.to_str()).collect();
    let parts = parts?;
    let file = match parts.as_slice() {
        [name] if matches!(*name, "logs" | "bin" | "broker" | "backups" | "exports" | "children" | "daemon-runs" | "oauth-locks" | "search-vectors") => return Some(DataEntry::Directory),
        [name] => owned_data_name(name),
        ["bin", name] => bin_file(name),
        ["broker", name] => matches!(*name, "owner.lock" | "approval.sock"),
        ["backups", client] if crate::clients::purge_backup_client_id(client) => return Some(DataEntry::Directory),
        ["backups", client, name] => crate::clients::purge_backup_client_id(client) && generated_file_name(name, backup_file),
        ["exports", name] => generated_file_name(name, export_file),
        ["children" | "daemon-runs", name] => generated_file_name(name, pid_json),
        ["oauth-locks", name] => oauth_file(name),
        ["search-vectors", name] => vector_file(name),
        // Current log writers use the data root. A generic logs/ folder is not owned.
        _ => false,
    };
    file.then_some(DataEntry::File)
}

fn remove_data_contents(dir: &Path, keep: &[PathBuf], report: &mut Report) {
    remove_selected_contents(dir, dir, keep, report);
}

fn remove_selected_contents(root: &Path, dir: &Path, keep: &[PathBuf], report: &mut Report) {
    let entries = match std::fs::read_dir(dir).and_then(|entries| entries.collect::<Result<Vec<_>, _>>()) {
        Ok(entries) => entries,
        Err(error) => {
            report.leftovers.push(Leftover { path: dir.display().to_string(), error: error.to_string() });
            return;
        }
    };
    for entry in entries {
        let path = entry.path();
        if keep.contains(&path) { continue; }
        let selected = path.strip_prefix(root).ok().and_then(owned_data_entry);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if selected == Some(DataEntry::Directory) && metadata.is_dir() && !is_link(&metadata) => {
                remove_selected_contents(root, &path, keep, report);
                if !keep.iter().any(|keep| keep.starts_with(&path)) {
                    remove_empty_dirs(&path, report);
                }
            }
            Ok(metadata) if selected.is_some() && (!metadata.is_dir() || is_link(&metadata)) => remove_path(&path, report),
            Ok(_) => report.preserved.push(format!("{} (not a Toolport-created entry)", path.display())),
            Err(error) => report.leftovers.push(Leftover { path: path.display().to_string(), error: error.to_string() }),
        }
    }
}

fn remove_contents(dir: &Path, keep: &[PathBuf], report: &mut Report) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            report.leftovers.push(Leftover {
                path: dir.display().to_string(),
                error: error.to_string(),
            });
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if keep.iter().any(|keep| keep == &path) {
                    continue;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() && std::fs::symlink_metadata(&path).is_ok_and(|metadata| !is_link(&metadata)) => {
                        remove_contents(&path, keep, report);
                        // A directory holding the owner lock stays until it is released.
                        if !keep.iter().any(|keep| keep.starts_with(&path)) {
                            remove_path(&path, report);
                        }
                    }
                    Ok(_) => remove_path(&path, report),
                    Err(error) => report.leftovers.push(Leftover {
                        path: path.display().to_string(),
                        error: error.to_string(),
                    }),
                }
            }
            Err(error) => report.leftovers.push(Leftover {
                path: dir.display().to_string(),
                error: error.to_string(),
            }),
        }
    }
}

fn remove_empty_dirs(dir: &Path, report: &mut Report) {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => match entries.next() {
            None => remove_path(dir, report),
            Some(Ok(_)) => {}, // Selected cleanup already reported preserved children.
            Some(Err(error)) => report.leftovers.push(Leftover { path: dir.display().to_string(), error: error.to_string() }),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => report.leftovers.push(Leftover { path: dir.display().to_string(), error: error.to_string() }),
    }
}

fn owned_autostart(contents: &str, name: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        contents.contains(&format!("<string>{name}</string>"))
            && contents.contains("<string>--hidden</string>")
    }
    #[cfg(not(target_os = "macos"))]
    {
        contents.lines().any(|line| line == format!("Name={name}"))
            && contents
                .lines()
                .any(|line| line == format!("Comment={name}startup script"))
    }
}

fn remove_autostart(report: &mut Report) -> Result<(), String> { startup_entries(true, report) }

fn startup_entries(remove: bool, report: &mut Report) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::windows_autostart::purge_entries(AUTOSTART_NAMES, remove, report);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().ok_or("Could not resolve your home directory")?;
        startup_files(autostart_files(&home), remove, report);
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn startup_files(files: Vec<PathBuf>, remove: bool, report: &mut Report) {
    for (path, name) in files.iter().zip(AUTOSTART_NAMES) {
        match std::fs::read_to_string(path) {
            Ok(contents) if owned_autostart(&contents, name) => { if remove { remove_path(path, report); } },
            Ok(_) => {
                let preserved = format!("{} (foreign startup entry preserved)", path.display());
                if !report.preserved.contains(&preserved) { report.preserved.push(preserved); }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => report.leftovers.push(Leftover {
                path: path.display().to_string(),
                error: error.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_handoff_observes_eof_and_reports_a_stuck_parent() {
        assert!(wait_for_exit_pipe(
            std::io::Cursor::new(Vec::<u8>::new()),
            std::time::Duration::from_secs(5)
        )
        .is_ok());
        struct Pending(std::sync::mpsc::Receiver<()>);
        impl std::io::Read for Pending {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }
        let (release, pending) = std::sync::mpsc::channel();
        let error =
            wait_for_exit_pipe(Pending(pending), std::time::Duration::from_millis(20)).unwrap_err();
        assert!(error.contains("Nothing was removed. Close Toolport and retry."));
        drop(release);
    }
    #[test]
    fn purge_temp_data_never_follows_symlinks_or_touches_native_client_data() {
        let env = crate::registry::DataDirTestEnv::new("purge_temp_data");
        let dir = env.dir.join("data");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/toolport-gateway"), "fixture").unwrap();
        std::fs::write(dir.join("registry.json"), "fixture").unwrap();
        let native = dir.parent().unwrap().join("native-client");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("config.json"), "native bytes").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&native, dir.join("link")).unwrap();
        let mut report = Report::default();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd").args(["/C", "mklink", "/J"])
                .arg(dir.join("junction")).arg(&native).status().unwrap();
            assert!(status.success(), "Windows junction fixture could not be created");
        }
        remove_contents(&dir, &[], &mut report);
        assert!(report.leftovers.is_empty(), "{:?}", report.leftovers);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(
            std::fs::read_to_string(native.join("config.json")).unwrap(),
            "native bytes"
        );
        drop(env);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn purge_autostart_requires_toolports_marker() {
        assert!(owned_autostart(
            &crate::autostart::linux_desktop_entry(
                "Toolport",
                Path::new("/fixture/toolport-gtk"),
                &["--hidden"]
            ),
            "Toolport"
        ));
        assert!(!owned_autostart(
            "[Desktop Entry]\nName=Toolport\nExec=native-app",
            "Toolport"
        ));
    }
    #[test]
    fn failed_disconnect_retains_recovery_data_and_never_calls_keyring() {
        let _env = crate::registry::DataDirTestEnv::new("purge_failed_disconnect");
        let dir = crate::registry::conduit_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("registry.json"), "recovery bytes").unwrap();
        let report = run_with(
            |_| Vec::new(),
            || {
                Ok(vec![crate::clients::DisconnectResult {
                    client_id: "fixture".into(),
                    path: "/fixture/client.json".into(),
                    dry_run: false,
                    error: Some("native config conflict".into()),
                    warnings: Vec::new(),
                }])
            },
            |_| panic!("must not stop before restoration"),
            || panic!("must not remove secrets"),
            |_| Ok(()),
            |_| panic!("must not remove startup entries"),
        )
        .unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(report.clients[0].path, "/fixture/client.json");
        assert_eq!(
            std::fs::read_to_string(dir.join("registry.json")).unwrap(),
            "recovery bytes"
        );
    }

    #[test]
    fn fake_keyring_failure_reports_exact_leftovers_and_keeps_data_for_retry() {
        let _env = crate::registry::DataDirTestEnv::new("purge_keyring_failure");
        let dir = crate::registry::conduit_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("secrets.enc"), "recovery bytes").unwrap();
        let report = run_with(
            |_| Vec::new(),
            || Ok(Vec::new()),
            |_| Vec::new(),
            || {
                Ok(crate::secrets::purge::Removal { leftovers: vec![Leftover {
                    path: "secret-service:/fixture/locked-item".into(), error: "locked".into(),
                }], ..Default::default() })
            },
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            report.leftovers[0].path,
            "secret-service:/fixture/locked-item"
        );
        assert!(dir.join("secrets.enc").exists());
        let report = run_with(
            |_| Vec::new(),
            || Ok(Vec::new()),
            |_| Vec::new(),
            || Ok(crate::secrets::purge::Removal::default()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
        assert!(report.leftovers.is_empty(), "{:?}", report.leftovers);
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    }

    #[test]
    fn active_session_prevents_secret_or_data_removal() {
        let _env = crate::registry::DataDirTestEnv::new("purge_busy_session");
        let dir = crate::registry::conduit_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("registry.json"), "keep").unwrap();
        let report = run_with(
            |_| Vec::new(),
            || Ok(Vec::new()),
            |_| {
                vec![Leftover {
                    path: "/fixture/toolport-gateway".into(),
                    error: "PID 123 is busy".into(),
                }]
            },
            || panic!("busy session"),
            |_| Ok(()),
            |_| panic!("busy session"),
        )
        .unwrap();
        assert_eq!(report.leftovers[0].path, "/fixture/toolport-gateway");
        assert!(dir.join("registry.json").exists());
    }

    #[test]
    fn temp_home_autostart_inventory_does_not_include_native_clients() {
        let _env = crate::registry::DataDirTestEnv::new("purge_temp_home");
        let home = _env.dir.join("home");
        let files = autostart_files(&home);
        assert_eq!(files.len(), AUTOSTART_NAMES.len());
        assert!(files.iter().all(|file| file.starts_with(&home)));
        assert!(!files
            .iter()
            .any(|file| file.to_string_lossy().contains("Claude")));
    }
    #[test]
    fn custom_native_directory_is_not_a_toolport_data_directory() {
        let env = crate::registry::DataDirTestEnv::new("purge_ownership");
        std::fs::write(
            env.dir.join("registry.json"),
            r#"{"mcpServers":{"native":{"command":"native"}}}"#,
        )
        .unwrap();
        assert!(verify_data_ownership(&env.dir).is_err());
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        assert!(verify_data_ownership(&env.dir).is_ok());
    }
    #[test]
    fn retained_edited_gateway_blocks_purge_even_without_a_disconnect_error() {
        let _env = crate::registry::DataDirTestEnv::new("purge_retained_gateway");
        let report = run_with(
            |_| Vec::new(),
            || {
                Ok(vec![crate::clients::DisconnectResult {
                    client_id: "fixture".into(),
                    path: "/fixture/edited-client.json".into(),
                    dry_run: false,
                    error: None,
                    warnings: vec!["kept your edited toolport entry".into()],
                }])
            },
            |_| panic!("connection remains"),
            || panic!("connection remains"),
            |_| Ok(()),
            |_| panic!("connection remains"),
        )
        .unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(report.leftovers[0].path, "/fixture/edited-client.json");
    }
    #[test]
    fn purge_named_override_requires_registry_evidence() {
        let env = crate::registry::DataDirTestEnv::new("purge_named_override");
        let dir = env.dir.join("Toolport");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("personal.txt"), "keep").unwrap();
        assert!(verify_ownership(&dir, true).is_err());
        std::fs::write(dir.join("registry.json"), r#"{"version":1,"servers":[],"profiles":[]}"#).unwrap();
        assert!(verify_ownership(&dir, true).is_ok());
    }

    #[test]
    fn purge_preflight_blocks_before_disconnect_or_any_deletion() {
        let env = crate::registry::DataDirTestEnv::new("purge_read_only_preflight");
        let report = run_with(
            |_| vec![Leftover { path: "busy daemon".into(), error: "active listener".into() }],
            || panic!("must not disconnect"), |_| panic!("must not stop"),
            || panic!("must not delete credentials"), |_| Ok(()), |_| panic!("must not delete startup"),
        ).unwrap();
        assert!(report.clients.is_empty() && report.removed.is_empty());
        assert_eq!(report.leftovers[0].path, "busy daemon");
        assert!(env.dir.exists());
    }

    #[test]
    fn purge_startup_inventory_failure_keeps_clients_and_credentials() {
        let _env = crate::registry::DataDirTestEnv::new("purge_startup_inventory");
        let report = run_with(
            |_| Vec::new(), || panic!("must not disconnect"), |_| panic!("must not stop"),
            || panic!("must not delete credentials"),
            |report| { report.leftovers.push(Leftover { path: "startup".into(), error: "unreadable".into() }); Ok(()) },
            |_| panic!("must not delete startup"),
        ).unwrap();
        assert!(report.removed.is_empty() && report.clients.is_empty());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn purge_foreign_startup_does_not_block_data_removal() {
        let env = crate::registry::DataDirTestEnv::new("purge_foreign_startup");
        let home = env.dir.with_extension("fake-home");
        let files = autostart_files(&home);
        std::fs::create_dir_all(files[0].parent().unwrap()).unwrap();
        std::fs::write(&files[0], "foreign startup entry").unwrap();
        let data = env.dir.clone();
        // The startup file lives outside the selected data folder.
        std::fs::write(data.join("secrets.enc"), "fixture").unwrap();
        let report = run_with(
            |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
            || Ok(crate::secrets::purge::Removal::default()),
            |report| { startup_files(files.clone(), false, report); Ok(()) },
            |report| { startup_files(files.clone(), true, report); Ok(()) },
        ).unwrap();
        assert!(report.leftovers.is_empty());
        assert_eq!(report.preserved.len(), 1);
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "foreign startup entry");
        assert!(!data.join("secrets.enc").exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn purge_startup_deletion_failure_precedes_credentials() {
        let env = crate::registry::DataDirTestEnv::new("purge_startup_delete");
        std::fs::write(env.dir.join("registry.json"), "keep").unwrap();
        let report = run_with(
            |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
            || panic!("must not delete credentials"), |_| Ok(()),
            |report| { report.leftovers.push(Leftover { path: "startup".into(), error: "denied".into() }); Ok(()) },
        ).unwrap();
        assert!(report.removed.is_empty());
        assert!(env.dir.join("registry.json").exists());
    }

    #[test]
    fn purge_install_inventory_protects_main_shared_ids_and_team_token() {
        let env = crate::registry::DataDirTestEnv::new("purge_installs");
        let dev = env.dir.join("Toolport-dev");
        let main = env.dir.join("Toolport");
        let legacy = env.dir.join("Conduit");
        for dir in [&dev, &main, &legacy] { std::fs::create_dir(dir).unwrap(); }
        std::fs::write(dev.join("registry.json"), r#"{"servers":[{"id":"shared"},{"id":"dev-only"}],"team":{}}"#).unwrap();
        std::fs::write(main.join("registry.json"), r#"{"servers":[{"id":"shared"}]}"#).unwrap();
        std::fs::write(legacy.join("registry.json"), r#"{"servers":[]}"#).unwrap();
        let others = other_installs(&dev, vec![dev.clone(), main.clone(), legacy.clone()]);
        let candidates = install_candidates(&dev);
        assert!(candidates.contains(&main) && candidates.contains(&legacy));
        assert!(others.contains(&main) && others.contains(&legacy));
        let scope = credential_scope(&dev, &others, false).unwrap();
        assert_eq!(scope.server_ids, std::collections::HashSet::from(["dev-only".into()]));
        assert!(!scope.all_installs);
        let scope = credential_scope(&dev, &[], false).unwrap();
        assert!(!scope.all_installs);
        assert_eq!(scope.server_ids, std::collections::HashSet::from(["shared".into(), "dev-only".into()]));
        assert!(credential_scope(&dev, &others, true).unwrap().all_installs);
        std::fs::remove_file(main.join("registry.json")).unwrap();
        assert!(credential_scope(&dev, &others, false).is_err());
    }

    #[test]
    fn purge_app_storage_inventory_uses_bundle_id() {
        let home = Path::new("/fixture/home");
        let paths = app_resources(home);
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|path| path.to_string_lossy().contains("com.tsout.conduit")));
        #[cfg(target_os = "macos")]
        assert_eq!(paths.len(), 5);
    }

    #[test]
    fn purge_owned_app_storage_is_removed_and_shared_storage_is_reported() {
        let env = crate::registry::DataDirTestEnv::new("purge_app_storage");
        let path = env.dir.join("com.tsout.conduit");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("cookies"), "fixture").unwrap();
        let mut report = Report::default();
        remove_resources(vec![path.clone()], true, &mut report);
        assert!(path.join("cookies").exists());
        assert_eq!(report.preserved.len(), 1);
        remove_resources(vec![path.clone()], false, &mut report);
        assert!(!path.exists());
        assert!(report.leftovers.is_empty());
    }

    #[test]
    fn purge_data_names_preserve_user_files_after_first_run() {
        let env = crate::registry::DataDirTestEnv::new("purge_data_names");
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        for name in ["personal.txt", "registry.json.owner-notes", "registry.json.bak.notes", "daemon-notes", "quarantine-notes.txt"] {
            std::fs::write(env.dir.join(name), "keep").unwrap();
        }
        std::fs::create_dir_all(env.dir.join("documents/broker")).unwrap();
        std::fs::write(env.dir.join("documents/broker/notes"), "keep nested").unwrap();
        for name in ["registry.json.bak", "registry.json.bak.123", "registry.json.v2-123.bak", "registry.json.123.1.conduit-tmp", "registry.json.unreadable-123", "registry.personal-sync-status.json", "daemon-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json", "tool-cache-v2-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json", "team-activity-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json.lock", "secret-service.lock", "downstream_backoff.json", "agent-guard-removed"] {
            std::fs::write(env.dir.join(name), "fixture").unwrap();
        }
        for name in ["bin/toolport-gateway", "bin/gateway-manifest.json", "broker/approval.sock",
            "backups/claude-code/123-.claude.json", "backups/claude-code/moved-servers.json",
            "backups/claude-code/original-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "exports/rules-2026-10-11.md", "exports/routines-2026-10-11-2.json", "exports/agent-permissions-2026-10-11.json",
            "children/123.json", "daemon-runs/123.json",
            "oauth-locks/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.lock",
            "oauth-locks/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.owner",
            "oauth-locks/auth-write-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.lock",
            "search-vectors/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
            let path = env.dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fixture").unwrap();
        }
        std::fs::create_dir(env.dir.join("logs")).unwrap();
        let report = run_with(
            |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
            || Ok(crate::secrets::purge::Removal::default()), |_| Ok(()), |_| Ok(()),
        ).unwrap();
        assert!(report.leftovers.is_empty(), "{:?}", report.leftovers);
        assert_eq!(report.preserved.len(), 6);
        assert!(!env.dir.join("registry.json").exists());
        assert!(!env.dir.join("secret-service.lock").exists());
        assert_eq!(std::fs::read_to_string(env.dir.join("documents/broker/notes")).unwrap(), "keep nested");
        assert_eq!(std::fs::read_dir(&env.dir).unwrap().count(), 6);
    }

    #[test]
    fn purge_corrupt_registry_requires_global_scope_and_names_the_file() {
        let env = crate::registry::DataDirTestEnv::new("purge_corrupt_registry");
        let path = env.dir.join("registry.json");
        std::fs::write(&path, "broken").unwrap();
        let error = credential_scope(&env.dir, &[], false).err().unwrap();
        assert!(error.contains(path.to_str().unwrap()), "{error}");
        assert!(credential_scope(&env.dir, &[], true).unwrap().all_installs);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn purge_global_credentials_require_explicit_consent() {
        let env = crate::registry::DataDirTestEnv::new("purge_sole_install");
        assert!(!credential_scope(&env.dir, &[], false).unwrap().all_installs);
        std::fs::write(env.dir.join("registry.json"), r#"{"servers":[]}"#).unwrap();
        assert!(!credential_scope(&env.dir, &[], false).unwrap().all_installs);
        assert!(credential_scope(&env.dir, &[], true).unwrap().all_installs);
        let other = env.dir.join("other");
        std::fs::create_dir(&other).unwrap();
        assert!(credential_scope(&env.dir, &[other], false).is_err());
    }

    #[test]
    fn purge_team_note_requires_a_deleted_team_token() {
        let env = crate::registry::DataDirTestEnv::new("purge_team_note");
        for (team_token_removed, team_token_in_file) in [(false, false), (true, false), (false, true)] {
            std::fs::write(env.dir.join("secrets.enc"), "fixture").unwrap();
            let report = run_with(
                |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
                || Ok(crate::secrets::purge::Removal { removed: vec!["fixture credential".into()], team_token_removed, team_token_in_file, ..Default::default() }),
                |_| Ok(()), |_| Ok(()),
            ).unwrap();
            assert!(report.leftovers.is_empty());
            assert_eq!(report.notes.iter().any(|note| note.contains("removed locally only")), team_token_removed || team_token_in_file);
        }
    }

    #[test]
    fn purge_undiscovered_custom_install_never_enables_global_credentials() {
        let env = crate::registry::DataDirTestEnv::new("purge_undiscovered");
        let selected = env.dir.join("config/Toolport");
        let custom = env.dir.join("data/toolport-work");
        for dir in [&selected, &custom] { std::fs::create_dir_all(dir).unwrap(); }
        std::fs::write(selected.join("registry.json"), r#"{"servers":[{"id":"mine"}]}"#).unwrap();
        std::fs::write(custom.join("registry.json"), r#"{"servers":[{"id":"work"}]}"#).unwrap();
        let candidates = install_candidates(&selected).into_iter().filter(|path| path.starts_with(&env.dir)).collect();
        let others = other_installs(&selected, candidates);
        assert!(!others.contains(&custom));
        let scope = credential_scope(&selected, &others, false).unwrap();
        assert!(!scope.all_installs, "discovery cannot prove sole ownership");
        assert_eq!(scope.server_ids, std::collections::HashSet::from(["mine".into()]));
    }

    #[test]
    fn purge_generated_path_collisions_preserve_user_bytes() {
        let env = crate::registry::DataDirTestEnv::new("purge_collisions");
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        let collisions = ["backups/tax.zip", "bin/personal-script", "logs/other-app.log", "daemon-notes.json",
            "broker/notes", "broker/nested/broker/notes", "children/notes.json", "exports/rules-notes.md",
            "oauth-locks/notes.lock", "search-vectors/notes", "daemon-runs/notes.json"];
        for name in collisions {
            let path = env.dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "user bytes").unwrap();
        }
        std::fs::write(env.dir.join("bin/toolport-gateway"), "gateway").unwrap();
        let report = run_with(
            |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
            || Ok(crate::secrets::purge::Removal::default()), |_| Ok(()), |_| Ok(()),
        ).unwrap();
        assert!(report.leftovers.is_empty(), "{:?}", report.leftovers);
        for name in collisions {
            assert_eq!(std::fs::read_to_string(env.dir.join(name)).unwrap(), "user bytes", "{name}");
            assert!(report.preserved.iter().any(|path| path.contains(name)), "{name}");
        }
        assert!(!env.dir.join("bin/toolport-gateway").exists());
        assert!(!env.dir.join("registry.json").exists());
    }

    #[test]
    fn purge_unreadable_other_registry_retains_ownership_then_retry_removes_tokens() {
        let env = crate::registry::DataDirTestEnv::new("purge_other_retry");
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        let selected = env.dir.join("registry.json");
        let original = br#"{"version":1,"servers":[{"id":"mine"}],"profiles":[]}"#;
        std::fs::write(&selected, original).unwrap();
        let other = env.dir.join("Toolport-dev");
        std::fs::create_dir(&other).unwrap();
        let other_registry = other.join("registry.json");
        std::fs::write(&other_registry, "broken").unwrap();
        let error = credential_scope(&env.dir, &[other.clone()], false).err().expect("ownership failure must block cleanup");
        assert!(error.contains(other_registry.to_str().unwrap()), "{error}");
        assert!(error.contains("Credentials were not removed"), "{error}");
        assert_eq!(std::fs::read(&selected).unwrap(), original);
        std::fs::write(&other_registry, r#"{"servers":[{"id":"theirs"}]}"#).unwrap();
        let scope = credential_scope(&env.dir, &[other], false).unwrap();
        let report = run_with(
            |_| Vec::new(), || Ok(Vec::new()), |_| Vec::new(),
            || {
                assert!(!scope.all_installs);
                assert!(scope.server_ids.contains("mine") && !scope.server_ids.contains("theirs"));
                Ok(crate::secrets::purge::Removal { removed: vec!["mine::TOKEN".into()], ..Default::default() })
            }, |_| Ok(()), |_| Ok(()),
        ).unwrap();
        assert!(report.removed.contains(&"mine::TOKEN".into()));
        assert!(report.leftovers.is_empty());
        assert!(!selected.exists());
    }

    #[test]
    fn purge_generated_names_match_writers_and_reject_loose_prefixes() {
        for path in ["daemon-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "daemon-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.lock",
            "bin/toolport-gateway-2.0.0-beta.1", "bin/toolport-gateway-2.0.0-0123456789ab.exe",
            "bin/.toolport-gateway.123.456.tmp", "bin/toolport-gateway.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tmp",
            "bin/gateway-manifest.json.123.1.conduit-tmp", "tool-cache-v2-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.servers.json",
            "search-vectors/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.123.1.tmp",
            "backups/claude-code/123-claude-settings-aaaaaaaaaaaaaaaa.json.123.1.conduit-tmp"] {
            assert!(owned_data_entry(Path::new(path)).is_some(), "{path}");
        }
        for path in ["daemon-notes.json", "daemon-123.json", "team-activity-notes.json", "tool-cache-v2-work.json",
            "registry.json.notes.conduit-tmp", "bin/personal-script", "bin/toolport-gateway-notes", "bin/toolport-gateway.unknown.tmp",
            "backups/tax.zip", "backups/claude-code/tax.zip", "backups/personal/123-file.json", "logs/other-app.log",
            "broker/personal", "exports/rules-notes.md", "oauth-locks/notes.lock", "children/notes.json"] {
            assert!(owned_data_entry(Path::new(path)).is_none(), "{path}");
        }
    }

}
