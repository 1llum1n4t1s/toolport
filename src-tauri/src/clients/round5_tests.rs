use super::*;

fn exercise_manual_v1_profile(data: &crate::registry::DataDirTestEnv, folder: &Path) {
    let path = folder.join(".claude.json");
    let original = serde_json::json!({"mcpServers": {
        "my-toolport":{"command":"/usr/bin/toolport-gateway", "env": {
            "TOOLPORT_ATTRIBUTION_ID":"claude-code", "TOOLPORT_PROFILE":"work"}},
        "other-gateway":{"command":"/usr/bin/conduit-gateway"},
        "native":{"command":"native"}
    }});
    std::fs::write(&path, original.to_string()).unwrap();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    let id = add_client_location("claude-code", folder.to_str().unwrap()).unwrap();
    assert!(locations::saved().unwrap().iter().any(|row| row.id == id && row.manual));
    let mut rows = detect_clients();
    apply_entry_states(&mut rows, &HashMap::new());
    let row = rows.iter().find(|row| row.id == id).unwrap();
    assert!(row.gateway_installed);
    assert_eq!(row.entry_state, GatewayEntryState::Managed);
    let gateway = data.dir.join("toolport-gateway");
    std::fs::write(&gateway, "fixture").unwrap();
    let outcome = repoint_stale_gateways_in(gateway.to_str().unwrap(), rows, &HashMap::new());
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert!(outcome.repointed.iter().any(|(client, _)| client == &id));
    let value = parse_json_value(&read_config_file(&path).unwrap()).unwrap();
    assert_eq!(value["mcpServers"].as_object().unwrap().len(), 2);
    assert_eq!(value["mcpServers"]["toolport"]["command"], gateway.to_str().unwrap());
    assert_eq!(value["mcpServers"]["toolport"]["env"][crate::brand::CLIENT_ID], id);
    assert_eq!(value["mcpServers"]["toolport"]["env"]["TOOLPORT_PROFILE"], "work");
    // Connect also deduplicates a v1 alias introduced after adoption.
    let mut duplicate = value.clone();
    duplicate["mcpServers"]["my-toolport"] = original["mcpServers"]["my-toolport"].clone();
    std::fs::write(&path, duplicate.to_string()).unwrap();
    let entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport",
        "transport":"stdio", "command":gateway})).unwrap();
    install_or_remove(&id, Some(&entry)).unwrap();
    assert_eq!(read_client(&find_def(&id).unwrap()).servers.len(), 2);
    uninstall_gateway(&id).unwrap();
    let value = parse_json_value(&read_config_file(&path).unwrap()).unwrap();
    assert_eq!(value["mcpServers"].as_object().unwrap().len(), 1);
    assert_eq!(value["mcpServers"]["native"], original["mcpServers"]["native"]);
}

#[test]
fn round5_readding_ordinary_v1_profiles_keeps_ownership_and_dedupe() {
    for folder_name in [".claude", ".claude-work"] {
        let data = crate::registry::DataDirTestEnv::new("round5-v1-readd");
        let _home = TestHome::set(&data.dir);
        let folder = data.dir.join(folder_name);
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join(".claude.json"), "{}").unwrap();
        detect_clients_for_display_in(machine_defs(true), crate::registry::Registry::default(), None).unwrap();
        assert!(locations::saved().unwrap().iter().any(|row| !row.manual));
        exercise_manual_v1_profile(&data, &folder);
    }
}

#[cfg(unix)]
#[test]
fn round5_manual_linked_v1_profile_keeps_ownership_and_dedupe() {
    let data = crate::registry::DataDirTestEnv::new("round5-v1-linked");
    let home = data.dir.join("home");
    std::fs::create_dir(&home).unwrap();
    let _home = TestHome::set(&home);
    let external = data.dir.join("external-profile");
    std::fs::create_dir(&external).unwrap();
    let folder = home.join(".claude-work");
    std::os::unix::fs::symlink(&external, &folder).unwrap();
    exercise_manual_v1_profile(&data, &folder);
}

#[cfg(unix)]
fn exercise_bad_profile_link(bad_link: &str) {
    use std::os::unix::fs::PermissionsExt;
    let data = crate::registry::DataDirTestEnv::new("round5-settings");
    let _home = TestHome::set(&data.dir);
    let bad = data.dir.join(".claude-bad");
    let blocked = data.dir.join("blocked");
    if bad_link == "loop" {
        std::os::unix::fs::symlink(&bad, &bad).unwrap();
    } else {
        std::fs::create_dir_all(blocked.join("profile")).unwrap();
        std::os::unix::fs::symlink(blocked.join("profile"), &bad).unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    // Prove this fixture exercises a real metadata failure, even with root runners.
    let error = std::fs::metadata(&bad).unwrap_err();
    if bad_link == "unreadable" {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }
    let good = data.dir.join(".claude-work/settings.json");
    std::fs::create_dir_all(good.parent().unwrap()).unwrap();
    let original = serde_json::json!({"permissions":{"allow":["Read"]}, "hooks": {
        "PreToolUse":[{"hooks":[
            {"command":"toolport-gateway --toolport-hook PreToolUse"},
            {"command":"toolport-gateway --toolport-guard claude-code"},
            {"command":"native-hook"}
        ]}]
    }});
    std::fs::write(&good, original.to_string()).unwrap();
    assert!(try_claude_code_config_paths().is_err(), "pruning must stay fail closed");
    crate::hooks::cleanup_on_startup();
    crate::guard_cleanup::run_once_with_cursor(None);
    if bad_link == "unreadable" {
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let value = parse_json_value(&std::fs::read_to_string(&good).unwrap()).unwrap();
    assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"], serde_json::json!([{"command":"native-hook"}]), "{bad_link}");
    assert_eq!(value["permissions"], original["permissions"]);
}

#[cfg(unix)]
#[test]
fn round5_profile_link_loop_does_not_skip_good_settings_cleanup() {
    exercise_bad_profile_link("loop");
}

#[cfg(unix)]
#[test]
fn round5_unreadable_profile_link_does_not_skip_good_settings_cleanup() {
    exercise_bad_profile_link("unreadable");
}

#[test]
fn round5_bulk_disconnect_matches_native_recovery_paths() {
    let data = crate::registry::DataDirTestEnv::new("round5-windows-recovery");
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join("work-profile");
    std::fs::create_dir(&folder).unwrap();
    let id = add_client_location("claude-code", folder.to_str().unwrap()).unwrap();
    let native = std::fs::canonicalize(&folder).unwrap().join(".claude.json");
    let store = data.dir.join("client-locations.json");
    let mut saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
    saved[0]["configPath"] = serde_json::json!(native);
    std::fs::write(&store, saved.to_string()).unwrap();
    let original = serde_json::json!({"mcpServers":{"native":{"command":"native"}}}).to_string();
    std::fs::write(&native, &original).unwrap();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    let entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport",
        "transport":"stdio", "command":"/new/toolport-gateway", "env":[{"key":"TOOLPORT_CLIENT_ID", "value":id}]})).unwrap();
    install_or_remove(&id, Some(&entry)).unwrap();
    #[cfg(windows)]
    assert!(!read_client(&find_def(&id).unwrap()).config_path.starts_with(r"\\?\"));
    let results = disconnect_all(true).unwrap();
    assert_eq!(results.iter().filter(|result| result.client_id == id).count(), 1);
    // The newly integrated global purge path must use the same native snapshot
    // even when it cannot load the registry.
    let registry_path = data.dir.join("registry.json");
    std::fs::write(&registry_path, "broken").unwrap();
    let results = disconnect::for_purge(true).unwrap();
    assert_eq!(results.iter().filter(|result| result.client_id == id).count(), 1);
    assert!(results.iter().all(|result| result.error.is_none()));
    assert_eq!(std::fs::read_to_string(&registry_path).unwrap(), "broken");
    assert_eq!(std::fs::read_to_string(&native).unwrap(), original);
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    install_or_remove(&id, Some(&entry)).unwrap();
    let results = disconnect_all(false).unwrap();
    assert_eq!(results.iter().filter(|result| result.client_id == id).count(), 1);
    assert!(results.iter().all(|result| result.error.is_none()));
    assert!(restore::released(&id, &native).unwrap());
    assert_eq!(std::fs::read_to_string(&native).unwrap(), original);
    // A native edit after release must not cause another client restoration.
    let edited = serde_json::json!({"mcpServers":{"toolport":{"command":"/old/toolport-gateway",
        "env":{"TOOLPORT_CLIENT_ID":id}}, "native":{"command":"native"}}}).to_string();
    std::fs::write(&native, &edited).unwrap();
    assert!(read_client(&find_def(&id).unwrap()).gateway_installed);
    let results = disconnect_all(true).unwrap();
    assert!(!results.iter().any(|result| result.client_id == id));
    let results = disconnect_all(false).unwrap();
    assert!(!results.iter().any(|result| result.client_id == id));
    assert_eq!(std::fs::read_to_string(&native).unwrap(), edited);
}

#[cfg(unix)]
#[test]
fn round5_bulk_disconnect_deduplicates_equivalent_recovery_paths() {
    let data = crate::registry::DataDirTestEnv::new("round5-recovery-alias");
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join("work-profile");
    std::fs::create_dir(&folder).unwrap();
    let alias = data.dir.join("profile-alias");
    std::os::unix::fs::symlink(&folder, &alias).unwrap();
    let id = add_client_location("claude-code", folder.to_str().unwrap()).unwrap();
    let path = folder.join(".claude.json");
    std::fs::write(&path, r#"{"mcpServers":{"native":{"command":"native"}}}"#).unwrap();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    let store = data.dir.join("client-locations.json");
    let mut saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
    saved[0]["configPath"] = serde_json::json!(alias.join(".claude.json"));
    std::fs::write(&store, saved.to_string()).unwrap();
    let entry: ServerEntry = serde_json::from_value(serde_json::json!({"id":"toolport", "name":"toolport",
        "transport":"stdio", "command":"/new/toolport-gateway", "env":[{"key":"TOOLPORT_CLIENT_ID", "value":id}]})).unwrap();
    mutation::run(&id, &path, Format::JsonMcpServers, || edit_format(Format::JsonMcpServers, &path, Some(&entry), false)).unwrap();
    let results = disconnect_all(true).unwrap();
    assert_eq!(results.iter().filter(|result| result.client_id == id).count(), 1);
}
