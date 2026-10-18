use super::*;

/// An `mcp.json` the way `openhands mcp add` writes it: every unset fastmcp
/// setting is a `null`. `events` and `off` were written by hand.
const CONFIG: &str = r#"{
  "mcpServers": {
    "docs": {
      "url": "https://mcp.example.com/mcp",
      "transport": "http",
      "headers": {
        "Authorization": "Bearer x"
      },
      "auth": null,
      "sse_read_timeout": null,
      "timeout": null,
      "description": null,
      "icon": null,
      "authentication": null,
      "enabled": true
    },
    "events": {
      "url": "https://old.example.com/sse?key=1"
    },
    "local": {
      "command": "node",
      "args": [
        "./server.js"
      ],
      "env": {
        "A": "1"
      },
      "transport": "stdio",
      "type": null,
      "cwd": null,
      "timeout": null,
      "keep_alive": null,
      "description": null,
      "icon": null,
      "authentication": null,
      "enabled": true
    },
    "off": {
      "command": "off-server",
      "args": [],
      "enabled": false
    }
  }
}
"#;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("toolport-openhands-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn file(&self, text: &str) -> PathBuf {
        let path = self.0.join("mcp.json");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn gateway() -> ServerEntry {
    serde_json::from_value(serde_json::json!({
        "name": "toolport",
        "transport": "stdio",
        "command": "/opt/toolport/toolport-gateway",
        "args": ["--stdio"],
        "env": [{"key": "TOOLPORT_CLIENT_ID", "value": "openhands"}],
    }))
    .unwrap()
}

fn remote(transport: &str) -> ServerEntry {
    serde_json::from_value(serde_json::json!({
        "name": "remote",
        "transport": transport,
        "url": "https://example.test/mcp",
        "env": [{"key": "Authorization", "value": "Bearer x"}],
    }))
    .unwrap()
}

#[test]
fn reads_started_servers_with_fastmcps_transport_rules() {
    let listed = parse(CONFIG).unwrap();
    let kinds: Vec<_> = listed
        .iter()
        .map(|s| (s.name.as_str(), s.transport.as_str()))
        .collect();
    // An `/sse` path without `transport` is SSE; a turned-off server isn't listed.
    assert_eq!(
        kinds,
        [("docs", "http"), ("events", "sse"), ("local", "stdio")]
    );
    assert_eq!(listed[0].env_keys, ["Authorization"]);
    assert_eq!(listed[2].args, ["./server.js"]);
    assert_eq!(listed[2].env_keys, ["A"]);
}

#[test]
fn guesses_sse_only_for_an_sse_path_segment() {
    for (url, sse) in [
        ("https://a.test/sse", true),
        ("https://a.test/sse/", true),
        ("https://a.test/v1/sse?x=1", true),
        ("https://a.test/ssevents", false),
        ("https://a.test/mcp", false),
        ("https://sse.test", false),
        ("https://a.test/mcp?path=/sse", false),
    ] {
        assert_eq!(sse_path(url), sse, "{url}");
    }
}

#[test]
fn connect_keeps_the_file_and_disconnect_gives_back_the_bytes() {
    let dir = Dir::new("connect");
    let path = dir.file(CONFIG);
    edit_gateway(&path, Some(&gateway())).unwrap();
    let added = std::fs::read_to_string(&path).unwrap();
    assert!(
        added.contains(r#""url": "https://old.example.com/sse?key=1""#),
        "{added}"
    );
    let root: Value = serde_json::from_str(&added).unwrap();
    let entry = &root["mcpServers"]["toolport"];
    assert_eq!(entry["command"], "/opt/toolport/toolport-gateway");
    assert_eq!(entry["env"]["TOOLPORT_CLIENT_ID"], "openhands");
    assert!(entry.get("type").is_none(), "{entry}");
    assert_eq!(parse(&added).unwrap().len(), 4);
    edit_gateway(&path, None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG);
}

#[test]
fn a_missing_config_starts_with_mcp_servers() {
    let dir = Dir::new("missing");
    let path = dir.0.join("mcp.json");
    edit_gateway(&path, Some(&gateway())).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    assert!(root["mcpServers"]["toolport"].is_object(), "{text}");
}

#[test]
fn remote_servers_always_name_their_transport() {
    let http = entry_value(&remote("http")).unwrap();
    assert_eq!(http["transport"], "http");
    assert_eq!(http["headers"]["Authorization"], "Bearer x");
    assert!(http.get("type").is_none() && http.get("env").is_none());
    assert_eq!(entry_value(&remote("sse")).unwrap()["transport"], "sse");
    let mut local = gateway();
    local.cwd = Some("/srv/tools".into());
    assert_eq!(entry_value(&local).unwrap()["cwd"], "/srv/tools");
}

#[test]
fn moving_servers_keeps_the_turned_off_ones() {
    let dir = Dir::new("move");
    let path = dir.file(CONFIG);
    write_servers(&path, &[gateway()]).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let names: Vec<_> = root["mcpServers"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(names, ["off", "toolport"]);
}

#[test]
fn a_turned_off_gateway_stays_off_and_keeps_its_settings_when_repaired() {
    let dir = Dir::new("repair");
    let path = dir.file(
        r#"{"mcpServers":{"toolport":{"command":"toolport-gateway","enabled":false,"timeout":9000}}}"#,
    );
    let listed = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(listed.len(), 1, "a turned-off gateway still counts");
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let entry = &root["mcpServers"]["toolport"];
    assert_eq!(entry["enabled"], false);
    assert_eq!(entry["timeout"], 9000);
    assert_eq!(entry["command"], "/opt/toolport/toolport-gateway");
    edit_gateway(&path, None).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(root["mcpServers"].as_object().unwrap().is_empty());
}

#[test]
fn import_refuses_settings_toolport_cannot_keep() {
    let dir = Dir::new("import");
    let path = dir.file(
        r#"{"mcpServers":{
          "plain":{"command":"uvx","args":["x"]},
          "slow":{"command":"uvx","timeout":60000},
          "filtered":{"url":"https://a.test/mcp","tools":{"x":{"enabled":false}}},
          "oauth":{"url":"https://a.test/mcp","auth":"oauth"},
          "off":{"command":"uvx","enabled":false}
        }}"#,
    );
    validate_import(&path, &["plain".into()], true).unwrap();
    for name in ["slow", "filtered", "oauth"] {
        let error = validate_import(&path, &[name.into()], false).unwrap_err();
        assert!(error.contains("can't keep"), "{name}: {error}");
    }
    validate_import(&path, &["off".into()], false).unwrap();
    let error = validate_import(&path, &["off".into()], true).unwrap_err();
    assert!(error.contains("turned-off"), "{error}");
    let error = validate_import(&path, &["gone".into()], false).unwrap_err();
    assert!(error.contains("changed since detection"), "{error}");
}

#[test]
fn servers_added_by_the_cli_can_be_imported_and_moved() {
    let dir = Dir::new("cli");
    let path = dir.file(CONFIG);
    validate_import(&path, &["docs".into(), "local".into()], true).unwrap();
}

#[test]
fn falsy_enabled_values_turn_a_server_off_like_python() {
    let dir = Dir::new("falsy");
    let path = dir.file(
        r#"{"mcpServers":{
          "on":{"command":"a"},
          "yes":{"command":"a","enabled":"false"},
          "one":{"command":"a","enabled":1},
          "zero":{"command":"a","enabled":0},
          "null":{"command":"a","enabled":null},
          "empty":{"command":"a","enabled":""},
          "list":{"command":"a","enabled":[]},
          "map":{"command":"a","enabled":{}},
          "false":{"command":"a","enabled":false}
        }}"#,
    );
    let listed = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let names: Vec<_> = listed.iter().map(|server| server.name.as_str()).collect();
    assert_eq!(names, ["on", "one", "yes"]);
    let error = validate_import(&path, &["zero".into()], true).unwrap_err();
    assert!(error.contains("turned-off"), "{error}");
}

#[test]
fn root_level_servers_are_listed_and_connect_moves_them_under_mcp_servers() {
    let dir = Dir::new("root");
    let path = dir.file(r#"{"git":{"command":"uvx","args":["mcp-server-git"]}}"#);
    let text = std::fs::read_to_string(&path).unwrap();
    let names: Vec<_> = parse(&text).unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["git"]);
    validate_import(&path, &["git".into()], true).unwrap();
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let keys: Vec<_> = root.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["mcpServers"]);
    assert_eq!(root["mcpServers"]["git"]["command"], "uvx");
    assert!(root["mcpServers"]["toolport"].is_object());
    edit_gateway(&path, None).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(root["mcpServers"]["git"]["command"], "uvx");
    assert!(root["mcpServers"].get("toolport").is_none());
}

#[test]
fn refuses_files_it_cannot_edit_safely() {
    assert!(parse("[]").unwrap_err().contains("must be an object"));
    assert!(parse(r#"{"mcpServers":[]}"#)
        .unwrap_err()
        .contains("must be an object"));
    let error = parse(r#"{"mcpServers":{"a":{"command":"x"}},"mcpServers":{}}"#).unwrap_err();
    assert!(error.contains("duplicate"), "{error}");
    let error =
        parse(r#"{"mcpServers":{"a":{"url":"https://a.test","transport":"ws"}}}"#).unwrap_err();
    assert!(error.contains("supported transport"), "{error}");
}

#[test]
fn remote_type_is_ignored_and_url_paths_remain_case_sensitive() {
    // FastMCP 4.1.1 RemoteMCPServer accepts `type` as an ignored extra, while
    // infer_transport_type_from_url checks the original case-sensitive path.
    let listed = parse(
        r#"{"mcpServers":{
        "type-extra":{"url":"https://a.test/mcp","type":"sse"},
        "upper-path":{"url":"https://a.test/SSE"},
        "lower-path":{"url":"https://a.test/sse"},
        "explicit":{"url":"https://a.test/sse","transport":"http","type":"sse"}
    }}"#,
    )
    .unwrap();
    let kinds: Vec<_> = listed
        .iter()
        .map(|server| (server.name.as_str(), server.transport.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("explicit", "http"),
            ("lower-path", "sse"),
            ("type-extra", "http"),
            ("upper-path", "http")
        ]
    );
}

#[test]
fn public_disconnect_keeps_converted_servers_after_native_edits() {
    let data = crate::registry::DataDirTestEnv::new("openhands-native-disconnect");
    let _env_lock = env_test_lock();
    let _client_env = isolated_client_env(&data.dir);
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join(".openhands");
    std::fs::create_dir_all(&folder).unwrap();
    let _persistence = EnvRestore::set("OPENHANDS_PERSISTENCE_DIR", &folder);
    let path = folder.join("mcp.json");
    let original = "{\n \"git\": {\"command\": \"uvx\", \"args\": [\"mcp-server-git\"]},\n \"old\": {\"command\": \"old\"}\n}\n";
    for edit in ["unchanged", "add", "edit", "remove", "move-and-add"] {
        crate::registry::save(&crate::registry::Registry::default()).unwrap();
        std::fs::write(&path, original).unwrap();
        install_or_remove("openhands", Some(&gateway())).unwrap();
        if edit == "move-and-add" {
            mutation::run("openhands", &path, Format::JsonOpenHandsMcpServers, || {
                moved::record_selected(
                    "openhands",
                    Format::JsonOpenHandsMcpServers,
                    &path,
                    &["git".into()],
                )?;
                moved::remove_selected(Format::JsonOpenHandsMcpServers, &path, &["git".into()])
            })
            .unwrap();
        }
        let mut connected: Value = serde_json::from_str(&read_config_file(&path).unwrap()).unwrap();
        assert!(connected.get("git").is_none());
        if edit != "move-and-add" {
            assert_eq!(connected["mcpServers"]["git"]["command"], "uvx");
        }
        match edit {
            "add" | "move-and-add" => {
                connected["mcpServers"]["remote"] =
                    serde_json::json!({"url":"https://example.test/mcp", "transport":"http"})
            }
            "edit" => connected["mcpServers"]["git"]["args"] = serde_json::json!(["edited"]),
            "remove" => {
                connected["mcpServers"]
                    .as_object_mut()
                    .unwrap()
                    .remove("git");
            }
            _ => {}
        }
        if edit != "unchanged" {
            std::fs::write(&path, connected.to_string()).unwrap();
        }
        uninstall_gateway("openhands").unwrap();
        let text = read_config_file(&path).unwrap();
        if edit == "unchanged" {
            assert_eq!(text, original);
        } else {
            if edit == "move-and-add" {
                connected["mcpServers"]["git"] =
                    serde_json::json!({"command":"uvx", "args":["mcp-server-git"]});
            }
            connected["mcpServers"]
                .as_object_mut()
                .unwrap()
                .remove("toolport");
            assert_eq!(
                serde_json::from_str::<Value>(&text).unwrap(),
                connected,
                "{edit}"
            );
            assert_eq!(
                parse(&text).unwrap().len(),
                if matches!(edit, "add" | "move-and-add") {
                    3
                } else if edit == "remove" {
                    1
                } else {
                    2
                }
            );
        }
    }
}

#[test]
fn public_disconnect_preserves_mixed_root_and_mcp_servers() {
    let data = crate::registry::DataDirTestEnv::new("openhands-mixed-disconnect");
    let _env_lock = env_test_lock();
    let _client_env = isolated_client_env(&data.dir);
    let _home = TestHome::set(&data.dir);
    let folder = data.dir.join(".openhands");
    std::fs::create_dir_all(&folder).unwrap();
    let _persistence = EnvRestore::set("OPENHANDS_PERSISTENCE_DIR", &folder);
    let path = folder.join("mcp.json");
    let original = serde_json::json!({"ignored": {"command":"root-extra"}, "mcpServers":{"git":{"command":"uvx"}}});
    std::fs::write(&path, original.to_string()).unwrap();
    crate::registry::save(&crate::registry::Registry::default()).unwrap();
    install_or_remove("openhands", Some(&gateway())).unwrap();
    let mut connected: Value = serde_json::from_str(&read_config_file(&path).unwrap()).unwrap();
    connected["mcpServers"]["remote"] = serde_json::json!({"url":"https://example.test/mcp"});
    std::fs::write(&path, connected.to_string()).unwrap();
    uninstall_gateway("openhands").unwrap();
    connected["mcpServers"]
        .as_object_mut()
        .unwrap()
        .remove("toolport");
    let text = read_config_file(&path).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), connected);
    assert_eq!(
        parse(&text)
            .unwrap()
            .iter()
            .map(|server| server.name.as_str())
            .collect::<Vec<_>>(),
        ["git", "remote"]
    );
}
