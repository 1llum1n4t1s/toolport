use super::*;

/// An `mcp.json` the way `openhands mcp add` writes it.
const CONFIG: &str = r#"{
  "mcpServers": {
    "docs": {
      "url": "https://mcp.example.com/mcp",
      "transport": "http",
      "headers": {
        "Authorization": "Bearer x"
      },
      "enabled": true
    },
    "events": {
      "url": "https://old.example.com/sse?key=1"
    },
    "local": {
      "command": "node",
      "args": ["./server.js"],
      "env": {
        "A": "1"
      },
      "transport": "stdio"
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
