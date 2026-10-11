//! Authoritative Windows launch-at-login status.
//!
//! `auto-launch` infers the Task Manager state from timestamp bytes in
//! `StartupApproved`. Windows records the state in the first byte, and the
//! timestamp is not a reliable enabled flag after a restart.

use std::io::ErrorKind;
use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

const RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Read Toolport's Run entry and Windows' Task Manager override without
/// converting an unknown or malformed value into a false Off state.
pub fn is_enabled(app_name: &str) -> Result<bool, String> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let run = match current_user.open_subkey_with_flags(RUN_KEY, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Could not read the Windows startup registry: {error}"
            ))
        }
    };
    match run.get_value::<String, _>(app_name) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Could not read Toolport's startup entry: {error}")),
    }

    let approved = match current_user.open_subkey_with_flags(STARTUP_APPROVED_KEY, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(format!("Could not read Windows startup approval: {error}")),
    };
    let raw = match approved.get_raw_value(app_name) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(format!("Could not read Windows startup approval: {error}")),
    };
    startup_approved_value_enabled(raw.vtype, &raw.bytes)
        .ok_or_else(|| "Windows returned an invalid startup approval value".to_string())
}

fn startup_approved_value_enabled(vtype: RegType, bytes: &[u8]) -> Option<bool> {
    if vtype != RegType::REG_BINARY || bytes.len() != 12 {
        return None;
    }
    startup_approved_enabled(bytes)
}

/// Windows uses 1, 3, and 7 for disabled states. Known enabled states include
/// 0, 2, and 6. Reject other values so Settings can show unreadable, not Off.
fn startup_approved_enabled(bytes: &[u8]) -> Option<bool> {
    match bytes.first().copied()? {
        0 | 2 | 6 => Some(true),
        1 | 3 | 7 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{startup_approved_enabled, startup_approved_value_enabled};
    use winreg::enums::RegType;

    #[test]
    fn startup_approval_uses_state_byte_not_timestamp() {
        assert_eq!(
            startup_approved_enabled(&[2, 0, 0, 0, 0xa5, 0x20, 0xf6, 0x4a, 0x95, 0xd7, 0xd9, 1]),
            Some(true)
        );
        assert_eq!(
            startup_approved_enabled(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Some(false)
        );
        assert_eq!(startup_approved_enabled(&[7, 0, 0, 0]), Some(false));
        assert_eq!(startup_approved_enabled(&[]), None);
        assert_eq!(startup_approved_enabled(&[9]), None);
    }

    #[test]
    fn startup_approval_rejects_wrong_type_and_length() {
        let enabled = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            startup_approved_value_enabled(RegType::REG_SZ, &enabled),
            None
        );
        assert_eq!(
            startup_approved_value_enabled(RegType::REG_BINARY, &enabled[..4]),
            None
        );
        assert_eq!(
            startup_approved_value_enabled(RegType::REG_BINARY, &enabled),
            Some(true)
        );
    }
}

/// Inventory first, then remove only Toolport commands or an orphan approval keyed by our name.
pub(crate) fn purge_entries(names: &[&str], remove: bool, report: &mut crate::purge::Report) {
    purge_entries_under(&RegKey::predef(HKEY_CURRENT_USER), names, remove, report);
}

fn purge_entries_under(user: &RegKey, names: &[&str], remove: bool, report: &mut crate::purge::Report) {
    use winreg::enums::KEY_WRITE;
    for name in names {
        let run_path = format!("HKCU\\{RUN_KEY}\\{name}");
        let approval_path = format!("HKCU\\{STARTUP_APPROVED_KEY}\\{name}");
        let result: Result<(), crate::purge::Leftover> = (|| {
            let run = match user.open_subkey_with_flags(RUN_KEY, KEY_READ) {
                Ok(run) => Some(run),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(crate::purge::Leftover { path: run_path.clone(), error: error.to_string() }),
            };
            let command = match run.as_ref().map(|run| run.get_value::<String, _>(*name)) {
                Some(Ok(command)) => Some(command),
                None => None,
                Some(Err(error)) if error.kind() == ErrorKind::NotFound => None,
                Some(Err(error)) => return Err(crate::purge::Leftover { path: run_path.clone(), error: error.to_string() }),
            };
            if let Some(command) = &command {
                let binary = if let Some(rest) = command.strip_prefix('"') {
                    rest.split('"').next().unwrap_or("")
                } else { command.split_whitespace().next().unwrap_or("") };
                let basename = std::path::Path::new(binary).file_name().and_then(|name| name.to_str()).unwrap_or("");
                if !matches!(basename.to_ascii_lowercase().as_str(), "conduit.exe" | "toolport.exe") || !command.split_whitespace().any(|arg| arg == "--hidden") {
                    let preserved = format!("{run_path} (foreign startup entry preserved)");
                    if !report.preserved.contains(&preserved) { report.preserved.push(preserved); }
                    return Ok(());
                }
            }
            // auto-launch disable removes Run but leaves StartupApproved. Our exact
            // reserved value name is sufficient ownership for that orphan.
            match user.open_subkey_with_flags(STARTUP_APPROVED_KEY, KEY_READ) {
                Ok(key) => match key.get_raw_value(name) {
                    Ok(_) if remove => {
                        user.open_subkey_with_flags(STARTUP_APPROVED_KEY, KEY_WRITE)
                            .and_then(|key| key.delete_value(name))
                            .map_err(|error| crate::purge::Leftover { path: approval_path.clone(), error: error.to_string() })?;
                        report.removed.push(approval_path.clone());
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(crate::purge::Leftover { path: approval_path.clone(), error: error.to_string() }),
                },
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(crate::purge::Leftover { path: approval_path.clone(), error: error.to_string() }),
            }
            if remove && command.is_some() {
                user.open_subkey_with_flags(RUN_KEY, KEY_WRITE).and_then(|key| key.delete_value(name))
                    .map_err(|error| crate::purge::Leftover { path: run_path.clone(), error: error.to_string() })?;
                report.removed.push(run_path);
            }
            Ok(())
        })();
        if let Err(leftover) = result { report.leftovers.push(leftover); }
    }
}

#[cfg(test)]
mod purge_tests {
    use super::*;
    #[test]
    fn purge_orphan_approval_converges_and_foreign_run_is_preserved() {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let fixture = format!("Software\\ToolportPurgeTest-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let (root, _) = user.create_subkey(&fixture).unwrap();
        let (approved, _) = root.create_subkey(STARTUP_APPROVED_KEY).unwrap();
        approved.set_raw_value("Toolport", &winreg::RegValue { vtype: RegType::REG_BINARY, bytes: vec![2; 12] }).unwrap();
        let mut report = crate::purge::Report::default();
        purge_entries_under(&root, &["Toolport"], false, &mut report);
        assert!(report.removed.is_empty() && report.leftovers.is_empty());
        assert!(approved.get_raw_value("Toolport").is_ok());
        purge_entries_under(&root, &["Toolport"], true, &mut report);
        assert!(report.leftovers.is_empty());
        assert!(approved.get_raw_value("Toolport").is_err());
        purge_entries_under(&root, &["Toolport"], true, &mut report);
        assert!(report.leftovers.is_empty());
        let (run, _) = root.create_subkey(RUN_KEY).unwrap();
        run.set_value("Toolport", &"native.exe --hidden").unwrap();
        approved.set_raw_value("Toolport", &winreg::RegValue { vtype: RegType::REG_BINARY, bytes: vec![2; 12] }).unwrap();
        purge_entries_under(&root, &["Toolport"], true, &mut report);
        assert_eq!(report.preserved.len(), 1);
        assert!(run.get_value::<String, _>("Toolport").is_ok());
        assert!(approved.get_raw_value("Toolport").is_ok());
        drop(run); drop(approved); drop(root);
        user.delete_subkey_all(fixture).unwrap();
    }
}
