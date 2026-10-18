use std::collections::BTreeMap;

use super::*;

/// What `codewhale mcp add` leaves: pretty JSON with global timeouts.
const CONFIG: &str = r#"{
  "timeouts": {
    "connect_timeout": 30,
    "execute_timeout": 1800,
    "read_timeout": 120
  },
  "servers": {
    "git": {
      "command": "uvx",
      "args": ["mcp-server-git"],
      "lazy": true
    },
    "docs": {
      "url": "https://example.test/mcp",
      "bearer_token_env_var": "DOCS_TOKEN"
    },
    "old": {
      "command": "old-server",
      "disabled": true
    }
  }
}
"#;

fn stdio(name: &str) -> ServerEntry {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "transport": "stdio",
        "command": "/opt/toolport/toolport-gateway",
        "args": ["--stdio"],
        "env": [{"key": "TOOLPORT_CLIENT_ID", "value": "codewhale"}],
    }))
    .unwrap()
}

fn desired(text: &str) -> Map<String, Json> {
    normalized(text).unwrap()[KEY]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn with(text: &str, entry: &ServerEntry) -> String {
    let mut want = desired(text);
    want.insert(entry.name.clone(), config_of(entry).unwrap());
    sync(Some(text), &want, &BTreeMap::new()).unwrap()
}

fn without(text: &str, name: &str) -> String {
    let mut want = desired(text);
    want.remove(name);
    sync(Some(text), &want, &BTreeMap::new()).unwrap()
}

#[test]
fn reads_started_servers_and_leaves_out_turned_off_ones() {
    let listed = servers(CONFIG).unwrap();
    let names: Vec<_> = listed.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["docs", "git"]);
    let docs = &listed[0];
    assert_eq!(docs.transport, "http");
    assert_eq!(docs.url.as_deref(), Some("https://example.test/mcp"));
    assert_eq!(listed[1].transport, "stdio");
    assert_eq!(listed[1].args, ["mcp-server-git"]);
    let off = r#"{"servers": {"a": {"command": "a", "enabled": false}}}"#;
    assert!(servers(off).unwrap().is_empty());
}

#[test]
fn connect_adds_one_entry_and_disconnect_restores_the_bytes() {
    let added = with(CONFIG, &stdio("toolport"));
    assert!(added.contains("\"toolport\""), "{added}");
    assert!(added.contains("\"lazy\": true"), "{added}");
    let gateway = &desired(&added)["toolport"];
    assert_eq!(gateway["command"], "/opt/toolport/toolport-gateway");
    assert_eq!(gateway["args"], serde_json::json!(["--stdio"]));
    assert_eq!(gateway["env"]["TOOLPORT_CLIENT_ID"], "codewhale");
    assert_eq!(without(&added, "toolport"), CONFIG);
}

#[test]
fn the_legacy_key_is_kept_and_both_keys_are_refused() {
    let legacy = "{\n  \"mcpServers\": {\n    \"git\": { \"command\": \"uvx\" }\n  }\n}\n";
    let added = with(legacy, &stdio("toolport"));
    let root: Json = serde_json::from_str(&added).unwrap();
    assert!(root.get("servers").is_none(), "{added}");
    assert!(root["mcpServers"].get("toolport").is_some(), "{added}");
    assert_eq!(without(&added, "toolport"), legacy);

    let both = r#"{"servers": {}, "mcpServers": {}}"#;
    assert!(normalized(both).unwrap_err().contains("both"));
}

#[test]
fn a_missing_or_empty_file_starts_with_servers() {
    for start in [None, Some(""), Some("{}")] {
        let mut want = Map::new();
        want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
        let out = sync(start, &want, &BTreeMap::new()).unwrap();
        let root: Json = serde_json::from_str(&out).unwrap();
        assert!(root["servers"].get("toolport").is_some(), "{out}");
        assert_eq!(servers(&out).unwrap().len(), 1);
    }
}

#[test]
fn turned_off_servers_stay_until_a_name_is_reused() {
    let out = without(CONFIG, "git");
    assert!(out.contains("\"old\""), "{out}");
    assert!(!out.contains("mcp-server-git"), "{out}");
    assert_eq!(
        normalized(&out).unwrap()["off"],
        normalized(CONFIG).unwrap()["off"]
    );
    // Connecting under a turned-off name replaces it.
    let reused = with(CONFIG, &stdio("old"));
    assert!(!reused.contains("old-server"), "{reused}");
    assert!(desired(&reused).contains_key("old"));
}

#[test]
fn servers_move_out_and_back() {
    let raw = raw_entries(CONFIG).unwrap();
    assert_eq!(raw.len(), 2);
    let (name, text, value) = &raw[1];
    assert_eq!(name, "git");
    assert_eq!(&definition(text).unwrap(), value);
    let emptied = without(&without(CONFIG, "git"), "docs");
    assert!(desired(&emptied).is_empty());
    let back = append_raw(
        Some(&emptied),
        &raw.iter()
            .map(|(_, text, _)| text.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(desired(&back), desired(CONFIG));
    assert_eq!(
        normalized(&back).unwrap()["rest"],
        normalized(CONFIG).unwrap()["rest"]
    );
}

#[test]
fn remote_servers_use_url_headers_and_sse_transport() {
    for (transport, sse) in [("http", false), ("sse", true)] {
        let entry: ServerEntry = serde_json::from_value(serde_json::json!({
            "name": "remote", "transport": transport, "url": "https://example.test/mcp",
            "env": [{"key": "Authorization", "value": "Bearer x"}],
        }))
        .unwrap();
        let config = config_of(&entry).unwrap();
        assert_eq!(config["url"], "https://example.test/mcp");
        assert_eq!(config["headers"]["Authorization"], "Bearer x");
        assert!(config.get("type").is_none());
        assert_eq!(config.get("transport").is_some(), sse, "{config}");
    }
}

#[test]
fn files_codewhale_cannot_read_are_refused() {
    assert!(normalized("[]").unwrap_err().contains("JSON object"));
    assert!(normalized("{ // note\n}")
        .unwrap_err()
        .contains("Could not parse"));
    assert!(normalized(r#"{"servers": []}"#)
        .unwrap_err()
        .contains("must be an object"));
}
