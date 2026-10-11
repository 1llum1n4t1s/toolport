use super::*;

#[test]
fn round4_saved_automatic_v1_profiles_keep_binary_ownership_and_dedupe() {
    for folder_name in [".claude", ".claude-work"] {
        let data = crate::registry::DataDirTestEnv::new("round4-v1-auto");
        let _home = TestHome::set(&data.dir);
        let folder = data.dir.join(folder_name);
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join(".claude.json");
        let original = serde_json::json!({"mcpServers": {
            "my-toolport":{"command":"/usr/bin/toolport-gateway", "env": {
                "TOOLPORT_ATTRIBUTION_ID":"claude-code", "TOOLPORT_PROFILE":"work"}},
            "other-gateway":{"command":"/usr/bin/conduit-gateway"},
            "native":{"command":"native"}
        }});
        std::fs::write(&path, original.to_string()).unwrap();
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        let id = format!("claude-code@{}", folder_name.trim_start_matches('.'));
        // Opening Clients with no current gateway still saves the ordinary row.
        detect_clients_for_display_in(machine_defs(true), crate::registry::load().unwrap(), None).unwrap();
        assert!(locations::saved().unwrap().iter().any(|row| row.id == id));
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
}

#[test]
fn round4_backup_named_profiles_receive_startup_hook_and_guard_cleanup() {
    let data = crate::registry::DataDirTestEnv::new("round4-settings");
    let _home = TestHome::set(&data.dir);
    let original = serde_json::json!({"permissions":{"allow":["Read"]}, "hooks": {
        "PreToolUse":[{"hooks":[
            {"command":"toolport-gateway --toolport-hook PreToolUse"},
            {"command":"toolport-gateway --toolport-guard claude-code"},
            {"command":"native-hook"}
        ]}]
    }});
    let paths: Vec<_> = [".claude-old", ".claude-bak", ".claude-backup-2026"].iter()
        .map(|name| data.dir.join(name).join("settings.json")).collect();
    for path in &paths {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, original.to_string()).unwrap();
    }
    crate::hooks::cleanup_on_startup();
    crate::guard_cleanup::run_once_with_cursor(None);
    for path in &paths {
        let value = parse_json_value(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"], serde_json::json!([{"command":"native-hook"}]));
        assert_eq!(value["permissions"], original["permissions"]);
    }
}

#[test]
fn round4_gateway_publication_failure_still_returns_client_rows() {
    let data = crate::registry::DataDirTestEnv::new("round4-publication");
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join(".claude-work");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join(".claude.json");
    let raw = r#"{"mcpServers":{"my-toolport":{"command":"/usr/bin/toolport-gateway"}}}"#;
    std::fs::write(&path, raw).unwrap();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    let rows = detect_clients_for_display_with_gateway(|| Err("Cannot publish gateway fixture".into())).unwrap();
    assert!(rows.iter().any(|row| row.id == "claude-code@claude-work" && row.config_exists && row.error.is_none()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), raw);
    assert!(crate::registry::load().unwrap().client_managed_entries.is_empty());
}

#[test]
fn round4_activity_resolves_each_missing_location_once_per_pass() {
    let entries = vec![serde_json::json!({"client":"adapter:claude-code@removed"}); 10_000];
    let mut calls = 0;
    let names = CallerNames::for_entries_with_adapter(&entries, || None, |_| {
        calls += 1;
        None
    });
    assert_eq!(calls, 1);
    assert_eq!(names.trusted_name(Some("adapter:claude-code@removed"), None), "[private]");
}

#[cfg(windows)]
#[test]
fn round4_windows_missing_main_codex_config_does_not_create_location() {
    let data = crate::registry::DataDirTestEnv::new("round4-codex-main");
    let _env = env_test_lock();
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join(".codex");
    std::fs::create_dir(&folder).unwrap();
    let _override = EnvRestore::set("CODEX_HOME", &folder);
    assert!(!folder.join("config.toml").exists());
    assert_eq!(add_client_location("codex", folder.to_str().unwrap()).unwrap(), "codex");
    assert!(locations::saved().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn round4_windows_manual_location_displays_ordinary_path() {
    let data = crate::registry::DataDirTestEnv::new("round4-windows-display");
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join("work-profile");
    std::fs::create_dir(&folder).unwrap();
    let id = add_client_location("codex", folder.to_str().unwrap()).unwrap();
    let row = read_client(&find_def(&id).unwrap());
    assert!(!row.config_path.starts_with(r"\\?\"), "{}", row.config_path);
    assert_eq!(add_client_location("codex", folder.to_str().unwrap()).unwrap(), id);
}
