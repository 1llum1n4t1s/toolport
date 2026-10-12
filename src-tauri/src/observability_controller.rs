//! Shell-neutral local observability mutations.

pub fn clear_activity_logs() -> Result<(), String> {
    let mut failed = Vec::new();
    if crate::audit::try_clear().is_err() {
        failed.push("audit log");
    }
    if crate::searchtrace::try_clear().is_err() {
        failed.push("search traces");
    }
    if crate::inspect::try_clear().is_err() {
        failed.push("inspector captures");
    }
    if crate::savings::try_clear().is_err() {
        failed.push("savings");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("Couldn't clear: {}", failed.join(", ")))
    }
}

/// Stores that can change an Activity panel. Exclude locks and backups: reading
/// a store must not itself invalidate the page.
pub fn activity_file(name: &str) -> bool {
    matches!(name, "audit.jsonl" | "security.jsonl" | "inspect.jsonl"
        | "search-trace.jsonl" | "search-trace-v2.jsonl" | "savings.jsonl"
        | "savings-v2.jsonl" | "savings-v3.jsonl" | "registry.json"
        | "client-locations.json" | "root-integrity-scopes.json")
        || (name.ends_with(".json") && (name.starts_with("tool-pins")
            || name.starts_with("quarantine")))
}

/// Cheap fallback for shells without a portable file watcher. No log parsing,
/// client scans or daemon probes unless a relevant store has changed.
pub fn activity_revision() -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let dir = crate::registry::conduit_dir().ok_or("Could not resolve Activity directory")?;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok("missing".into()),
        Err(error) => return Err(format!("Could not check Activity stores: {error}")),
    };
    let mut stores = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if activity_file(&name) {
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            stores.push((name, metadata.len(), metadata.modified().ok()));
        }
    }
    stores.sort();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    stores.hash(&mut hash);
    Ok(format!("{:x}", hash.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_revision_observes_appends_rewrites_replacements_and_removal() {
        let data = crate::registry::DataDirTestEnv::new("activity-revision");
        let empty = activity_revision().unwrap();
        std::fs::write(data.dir.join("audit.jsonl.lock"), "lock").unwrap();
        std::fs::write(data.dir.join("gateway.log"), "background log").unwrap();
        assert_eq!(activity_revision().unwrap(), empty);
        let path = data.dir.join("audit.jsonl");
        std::fs::write(&path, "one").unwrap();
        let first = activity_revision().unwrap();
        assert_ne!(first, empty);
        assert_eq!(activity_revision().unwrap(), first);
        std::fs::write(&path, "one two").unwrap();
        let appended = activity_revision().unwrap();
        assert_ne!(appended, first);
        std::fs::write(&path, "new two").unwrap();
        assert_ne!(activity_revision().unwrap(), appended);
        std::fs::write(data.dir.join("replacement"), "changed").unwrap();
        let rewritten = activity_revision().unwrap();
        std::fs::rename(data.dir.join("replacement"), &path).unwrap();
        assert_ne!(activity_revision().unwrap(), rewritten);
        std::fs::remove_file(path).unwrap();
        assert_eq!(activity_revision().unwrap(), empty);
        for name in ["tool-pins-v2-profile.json", "quarantine-v2-profile.json", "security.jsonl", "savings-v3.jsonl", "client-locations.json"] {
            assert!(activity_file(name));
            assert!(!activity_file(&format!("{name}.lock")));
            assert!(!activity_file(&format!("{name}.bak")));
        }
    }
}
