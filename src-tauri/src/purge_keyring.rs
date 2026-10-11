//! Read-only credential inventory followed by explicit account-scoped removal.
use crate::purge::Leftover;

#[derive(Default)]
pub(crate) struct Removal {
    pub removed: Vec<String>,
    pub team_token_removed: bool,
    pub team_token_in_file: bool,
    pub preserved: Vec<String>,
    pub notes: Vec<String>,
    pub leftovers: Vec<Leftover>,
}

/// Accounts have no install id. Never infer ownership from the service alone.
pub(crate) struct Scope {
    pub all_installs: bool,
    pub server_ids: std::collections::HashSet<String>,
}

impl Scope {
    fn owns(&self, account: &str) -> bool {
        self.all_installs || account.split_once("::")
            .is_some_and(|(id, _)| id != "__toolport_internal__" && id != "__conduit_master_key__"
                && id != crate::teams::TEAM_TOKEN_SERVER && self.server_ids.contains(id))
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn remove_owned<T>(
    entries: impl IntoIterator<Item = T>,
    scope: &Scope,
    mut attributes: impl FnMut(&T) -> Result<(String, String), String>,
    mut path: impl FnMut(&T) -> String,
    mut delete: impl FnMut(&T) -> Result<(), String>,
) -> Removal {
    let mut report = Removal::default();
    // Finish the read-only inventory before deleting even one credential.
    let inventory: Vec<_> = entries.into_iter().map(|entry| {
        let attrs = attributes(&entry);
        (entry, attrs)
    }).collect();
    for (entry, attrs) in &inventory {
        if let Err(error) = attrs {
            report.leftovers.push(Leftover { path: path(entry), error: error.clone() });
        }
    }
    if !report.leftovers.is_empty() {
        return report;
    }
    for (entry, attrs) in inventory {
        let (service, account) = attrs.expect("inventory checked");
        if service != super::SERVICE {
            continue;
        }
        let path = path(&entry);
        if !scope.owns(&account) {
            report.preserved.push(path);
        } else if let Err(error) = delete(&entry) {
            report.leftovers.push(Leftover { path, error });
        } else {
            record_deleted(&account, path, &mut report);
        }
    }
    report
}

fn record_deleted(account: &str, path: String, report: &mut Removal) {
    let team = format!("{}::{}", crate::teams::TEAM_TOKEN_SERVER, crate::teams::TEAM_TOKEN_KEY);
    report.team_token_removed |= account == team
        || account.strip_prefix(&team).is_some_and(|suffix| suffix.starts_with("::toolport-chunk::"));
    report.removed.push(path);
}

#[cfg(target_os = "linux")]
fn skipped_keyring(reason: &str) -> Removal {
    Removal { leftovers: vec![Leftover {
        path: "secret-service:conduit-mcp (credential cleanup not done)".into(),
        error: format!("{reason}. Credentials may still exist. Start or unlock your Secret Service provider, then retry; the registry was retained."),
    }], ..Removal::default() }
}

#[cfg(target_os = "linux")]
fn no_keyring(team_token_in_file: bool) -> Removal {
    Removal { team_token_in_file, notes: vec!["No keyring in use; encrypted file secrets are removed with the data folder.".into()], ..Removal::default() }
}

#[cfg(target_os = "linux")]
fn unavailable(error: &dbus_secret_service::Error) -> bool {
    match error {
        dbus_secret_service::Error::Unavailable => true,
        dbus_secret_service::Error::Dbus(error) => matches!(error.name(),
            Some("org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" | "org.freedesktop.DBus.Error.NoServer" | "org.freedesktop.DBus.Error.FileNotFound")),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn missing_bus(error: &dbus_secret_service::Error) -> bool {
    matches!(error, dbus_secret_service::Error::Dbus(error) if matches!(error.name(),
        Some("org.freedesktop.DBus.Error.NoServer" | "org.freedesktop.DBus.Error.FileNotFound")))
}

#[cfg(target_os = "linux")]
pub(crate) fn remove(scope: &Scope) -> Result<Removal, String> {
    use dbus_secret_service::{EncryptionType, SecretService};
    // Follow the real backend selector, but still inventory an available desktop
    // keyring: selecting the file backend does not remove its older OS tokens.
    let file_active = super::file::active();
    let team_token_in_file = file_active && super::file::get_secret_result(crate::teams::TEAM_TOKEN_SERVER, crate::teams::TEAM_TOKEN_KEY)
        .ok().flatten().is_some();
    crate::hostenv::restore_session_bus_env();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })));
    let desktop_runtime = runtime.exists();
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none_or(|address| address.is_empty()) {
        return Ok(if desktop_runtime {
            skipped_keyring("No session bus found in the desktop runtime directory")
        } else {
            // A genuinely headless host has neither a session bus nor runtime data.
            no_keyring(team_token_in_file)
        });
    }
    let service = match SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 5) {
        Ok(service) => service,
        Err(error) if missing_bus(&error) && file_active && !desktop_runtime => return Ok(no_keyring(team_token_in_file)),
        Err(error) if unavailable(&error) => return Ok(skipped_keyring(&format!("Secret Service is unavailable: {error}"))),
        Err(error) => return Err(error.to_string()),
    };
    let entries = service
        .search_items(std::collections::HashMap::from([(
            "service",
            super::SERVICE,
        )]))
        .map_err(|error| error.to_string())?;
    let mut report = remove_owned(
        entries.unlocked.into_iter().chain(entries.locked),
        scope,
        |item| {
            item.get_attributes()
                .map_err(|error| error.to_string())
                .and_then(|attrs| {
                    Ok((attrs.get("service").cloned().ok_or("Missing service attribute")?,
                        attrs.get("username").cloned().or_else(|| attrs.get("account").cloned()).ok_or("Missing account attribute")?))
                })
        },
        |item| format!("secret-service:{}", item.path),
        |item| item.delete().map_err(|error| error.to_string()),
    );
    report.team_token_in_file = team_token_in_file;
    Ok(report)
}

#[cfg(target_os = "windows")]
pub(crate) fn remove(scope: &Scope) -> Result<Removal, String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredEnumerateW, CredFree, CREDENTIALW, CRED_TYPE_GENERIC,
    };
    unsafe fn string(mut ptr: *const u16) -> String {
        let mut value = Vec::new();
        if !ptr.is_null() {
            while unsafe { *ptr } != 0 {
                value.push(unsafe { *ptr });
                ptr = unsafe { ptr.add(1) };
            }
        }
        String::from_utf16_lossy(&value)
    }
    let mut count = 0;
    let mut entries: *mut *mut CREDENTIALW = std::ptr::null_mut();
    if unsafe { CredEnumerateW(std::ptr::null(), 0, &mut count, &mut entries) } == 0 {
        let error = unsafe { GetLastError() };
        return if error == ERROR_NOT_FOUND {
            Ok(Removal::default())
        } else {
            Err(format!("CredEnumerateW: {error}"))
        };
    }
    let result = remove_owned(
        unsafe { std::slice::from_raw_parts(entries, count as usize) }
            .iter()
            .copied(),
        scope,
        |entry| {
            let entry = unsafe { &**entry };
            let target = unsafe { string(entry.TargetName) };
            let service = if entry.Type == CRED_TYPE_GENERIC
                && target.ends_with(&format!(".{}", super::SERVICE))
            {
                super::SERVICE
            } else {
                ""
            };
            let account = target.strip_suffix(&format!(".{}", super::SERVICE)).unwrap_or("");
            Ok((service.into(), account.into()))
        },
        |entry| {
            format!("credential-manager:{}", unsafe {
                string((**entry).TargetName)
            })
        },
        |entry| {
            if unsafe { CredDeleteW((**entry).TargetName, CRED_TYPE_GENERIC, 0) } != 0 {
                Ok(())
            } else {
                Err(format!("CredDeleteW: {}", unsafe { GetLastError() }))
            }
        },
    );
    unsafe { CredFree(entries.cast()) };
    Ok(result)
}

#[cfg(target_os = "macos")]
pub(crate) fn remove(scope: &Scope) -> Result<Removal, String> {
    use core_foundation::{
        array::CFArray,
        base::{CFType, CFTypeRef, TCFType},
        boolean::CFBoolean,
        dictionary::CFDictionary,
        string::CFString,
    };
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};
    use security_framework_sys::item::*;
    #[link(name = "Security", kind = "framework")]
    extern "C" {
        static kSecUseDataProtectionKeychain: CFTypeRef;
        fn SecItemDelete(query: CFTypeRef) -> i32;
        fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> i32;
    }
    unsafe fn k(raw: CFTypeRef) -> CFType {
        unsafe { CFType::wrap_under_get_rule(raw) }
    }
    fn account(attributes: &CFDictionary) -> Option<String> {
        let value = attributes.find(unsafe { kSecAttrAccount.cast::<std::ffi::c_void>() })?;
        Some(unsafe { CFString::wrap_under_get_rule((*value).cast()) }.to_string())
    }
    let mut report = Removal::default();
    let mut legacy = Vec::new();
    // Legacy ACL-bearing entries require reference deletion. No secret data is read.
    let entries = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(super::SERVICE)
        .limit(Limit::All)
        .load_refs(true)
        .load_attributes(true)
        .load_data(false)
        .search();
    match entries {
        Ok(entries) => {
            for entry in entries {
                let SearchResult::Dict(attributes) = entry else {
                    report.leftovers.push(Leftover {
                        path: "keychain:conduit-mcp/legacy (unreadable item)".into(),
                        error: "Keychain did not return item attributes".into(),
                    });
                    continue;
                };
                let Some(account) = account(&attributes) else {
                    report.leftovers.push(Leftover { path: "keychain:conduit-mcp/legacy".into(), error: "Missing account attribute".into() });
                    continue;
                };
                let path = format!("keychain:conduit-mcp/legacy/{account}");
                if !scope.owns(&account) {
                    report.preserved.push(path);
                    continue;
                }
                let reference = attributes.find(unsafe { kSecValueRef.cast::<std::ffi::c_void>() });
                let Some(reference) = reference else {
                    report.leftovers.push(Leftover {
                        path,
                        error: "Keychain did not return an item reference".into(),
                    });
                    continue;
                };
                legacy.push((unsafe { k((*reference).cast()) }, account, path));
            }
        }
        Err(error) if error.code() == -25300 => {}
        Err(error) => report.leftovers.push(Leftover {
            path: "keychain:conduit-mcp/legacy (inventory unavailable)".into(),
            error: error.to_string(),
        }),
    }
    unsafe {
        let base = vec![
            (k(kSecClass.cast()), k(kSecClassGenericPassword.cast())),
            (
                k(kSecAttrService.cast()),
                CFString::new(super::SERVICE).as_CFType(),
            ),
            (
                k(kSecAttrAccessGroup.cast()),
                CFString::new(super::platform::SHARED_ACCESS_GROUP).as_CFType(),
            ),
            (
                k(kSecUseDataProtectionKeychain.cast()),
                CFBoolean::true_value().as_CFType(),
            ),
        ];
        let mut search = base.clone();
        search.extend([
            (
                k(kSecReturnAttributes.cast()),
                CFBoolean::true_value().as_CFType(),
            ),
            (k(kSecMatchLimit.cast()), k(kSecMatchLimitAll.cast())),
        ]);
        let query = CFDictionary::from_CFType_pairs(&search);
        let mut result = std::ptr::null();
        let status = SecItemCopyMatching(query.as_concrete_TypeRef().cast(), &mut result);
        if status == 0 && !result.is_null() {
            let entries = CFArray::<CFDictionary>::wrap_under_create_rule(result.cast());
            let accounts: Option<Vec<_>> = entries.iter().map(|attributes| account(&attributes)).collect();
            if accounts.is_none() {
                report.leftovers.push(Leftover { path: "keychain:conduit-mcp/data-protection".into(), error: "Missing account attribute; no credentials removed".into() });
            }
            let inventory_failed = !report.leftovers.is_empty();
            for account in accounts.unwrap_or_default() {
                let path = format!("keychain:conduit-mcp/data-protection/{account}");
                if !scope.owns(&account) {
                    report.preserved.push(path);
                    continue;
                }
                if inventory_failed { continue; }
                let mut delete = base.clone();
                delete.push((
                    k(kSecAttrAccount.cast()),
                    CFString::new(&account).as_CFType(),
                ));
                let query = CFDictionary::from_CFType_pairs(&delete);
                let status = SecItemDelete(query.as_concrete_TypeRef().cast());
                if status == 0 {
                    record_deleted(&account, path, &mut report);
                } else if status == -25300 {
                    report.removed.push(path);
                } else {
                    report.leftovers.push(Leftover {
                        path: format!("keychain:conduit-mcp/data-protection/{account}"),
                        error: format!("SecItemDelete: {status}"),
                    });
                }
            }
        } else if status != -25300 {
            report.leftovers.push(Leftover {
                path: "keychain:conduit-mcp/data-protection (inventory unavailable)".into(),
                error: format!("SecItemCopyMatching: {status}"),
            });
        }
    }
    if report.leftovers.is_empty() {
        for (reference, account, path) in legacy {
            let status = unsafe { security_framework_sys::keychain_item::SecKeychainItemDelete(reference.as_CFTypeRef().cast_mut().cast()) };
            if status == 0 { record_deleted(&account, path, &mut report); }
            else { report.leftovers.push(Leftover { path, error: format!("SecKeychainItemDelete: {status}") }); }
        }
    } else {
        report.preserved.extend(legacy.into_iter().map(|(_, _, path)| format!("{path} (retained after keychain cleanup failure)")));
    }
    Ok(report)
}

#[cfg(test)]
pub(crate) fn remove_fixture_accounts(scope: &Scope, accounts: &[&str]) -> Removal {
    remove_owned(accounts.iter().copied(), scope,
        |account| Ok((super::SERVICE.into(), (*account).into())),
        |account| (*account).into(), |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fake_keyring_keeps_other_apps_and_reports_each_failed_item() {
        let items = [
            ("other-app", "/vault/native"),
            ("conduit-mcp", "/vault/server"),
            ("conduit-mcp", "/vault/orphan-chunk"),
        ];
        let mut deleted = Vec::new();
        let leftovers = remove_owned(
            items,
            &Scope { all_installs: true, server_ids: Default::default() },
            |item| Ok((item.0.into(), item.1.into())),
            |item| item.1.into(),
            |item| {
                deleted.push(item.1);
                if item.1.ends_with("chunk") {
                    Err("locked".into())
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(deleted, ["/vault/server", "/vault/orphan-chunk"]);
        assert_eq!(leftovers.leftovers[0].path, "/vault/orphan-chunk");
        assert_eq!(leftovers.leftovers[0].error, "locked");
    }
    #[test]
    fn purge_scoped_keyring_preserves_other_install_master_and_chunks() {
        let items = ["mine::TOKEN", "mine::TOKEN::toolport-chunk::generation::0", "main::TOKEN", "__conduit_master_key__", "__conduit_team__::member_token"];
        let scope = Scope { all_installs: false, server_ids: std::collections::HashSet::from(["mine".into()]) };
        let report = remove_owned(items, &scope, |account| Ok(("conduit-mcp".into(), (*account).into())), |account| (*account).into(), |_| Ok(()));
        assert_eq!(report.removed, items[..2]);
        assert_eq!(report.preserved, items[2..]);
        assert!(report.leftovers.is_empty());
    }

    #[test]
    fn purge_keyring_inventory_failure_never_deletes_an_earlier_item() {
        let scope = Scope { all_installs: true, server_ids: Default::default() };
        let report = remove_owned(["readable", "unreadable"], &scope,
            |account| if *account == "unreadable" { Err("denied".into()) } else { Ok(("conduit-mcp".into(), (*account).into())) },
            |account| (*account).into(), |_| panic!("inventory must finish before deletion"));
        assert!(report.removed.is_empty());
        assert_eq!(report.leftovers[0].path, "unreadable");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn purge_missing_secret_service_is_distinct_from_locked_credentials() {
        assert!(unavailable(&dbus_secret_service::Error::Unavailable));
        assert!(!unavailable(&dbus_secret_service::Error::Locked));
        assert!(!unavailable(&dbus_secret_service::Error::Prompt));
    }

    #[test]
    fn purge_explicit_global_scope_removes_master_internal_and_deleted_server_accounts() {
        let items = ["__conduit_master_key__", "__toolport_internal__::task_key", "deleted-server::TOKEN", "mine::TOKEN::toolport-chunk::generation::0"];
        let scope = Scope { all_installs: true, server_ids: Default::default() };
        let report = remove_owned(items, &scope, |account| Ok(("conduit-mcp".into(), (*account).into())), |account| (*account).into(), |_| Ok(()));
        assert_eq!(report.removed, items);
        assert!(report.preserved.is_empty() && report.leftovers.is_empty());
        assert!(!report.team_token_removed);
    }

    #[test]
    fn purge_team_token_note_tracks_successful_deletion_only() {
        let team = "__conduit_team__::member_token";
        let scope = Scope { all_installs: true, server_ids: Default::default() };
        for delete_team in [false, true] {
            let report = remove_owned(["mine::TOKEN", team], &scope,
                |account| Ok(("conduit-mcp".into(), (*account).into())), |account| (*account).into(),
                |account| if *account == team && !delete_team { Err("locked".into()) } else { Ok(()) });
            assert_eq!(report.team_token_removed, delete_team);
        }
        let report = remove_owned([team], &Scope { all_installs: false, server_ids: Default::default() },
            |account| Ok(("conduit-mcp".into(), (*account).into())), |account| (*account).into(), |_| panic!("shared token must stay"));
        assert!(!report.team_token_removed);
    }

    #[test]
    fn purge_default_scope_preserves_shared_and_unknown_accounts_even_without_detected_installs() {
        let items = ["mine::TOKEN", "mine::TOKEN::toolport-chunk::generation::0", "work::TOKEN",
            "__conduit_master_key__", "__toolport_internal__::task_key", "__conduit_team__::member_token", "unknown"];
        let scope = Scope { all_installs: false, server_ids: std::collections::HashSet::from([
            "mine".into(), "__toolport_internal__".into(), "__conduit_team__".into()]) };
        let report = remove_owned(items, &scope, |account| Ok(("conduit-mcp".into(), (*account).into())),
            |account| (*account).into(), |_| Ok(()));
        assert_eq!(report.removed, items[..2]);
        assert_eq!(report.preserved, items[2..]);
        assert!(!report.team_token_removed);
    }

}
