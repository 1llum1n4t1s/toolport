use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "toolport-nanobot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, text: &str) -> PathBuf {
        let path = self.0.join("config.json");
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
    serde_json::from_value(serde_json::json!({"name":"toolport","transport":"stdio","command":"/opt/toolport/toolport-gateway","args":["--stdio"],"env":[{"key":"TOOLPORT_CLIENT_ID","value":"nanobot"}]})).unwrap()
}
const CONFIG: &str = "{\n  \"agents\": {\"defaults\": {\"model\": \"mine\"}},\n  \"tools\": {\n    \"web\": {\"enabled\": true},\n    \"mcpServers\": {\"native\": {\"command\": \"node\", \"args\": [\"server.js\"], \"enabled\": false}}\n  }\n}\n";
#[test]
fn no_off_switch_and_native_server_fields_are_read() {
    let servers = parse(CONFIG).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "native");
    assert_eq!(servers[0].args, ["server.js"]);
}
#[test]
fn transport_inference_matches_nanobot() {
    for (definition, expected) in [
        (serde_json::json!({"command":"node"}), "stdio"),
        (serde_json::json!({"url":"https://a.test/sse/"}), "sse"),
        (serde_json::json!({"url":"https://a.test/sse?x=1"}), "http"),
        (
            serde_json::json!({"url":"https://a.test/sse", "type":"streamableHttp"}),
            "http",
        ),
        (
            serde_json::json!({"url":"https://a.test/mcp", "type":"sse"}),
            "sse",
        ),
    ] {
        assert_eq!(transport(&definition), Some(expected));
    }
}
#[test]
fn production_connect_disconnect_preserves_unrelated_text_and_returns_original_bytes() {
    let _lock = crate::registry::data_dir_test_lock();
    let dir = Dir::new();
    let _data = crate::registry::DataDirOverride::set(dir.0.join("data"));
    let path = dir.file(CONFIG);
    // Production Connect stages the writer in a strict JSON mutation and captures
    // provenance; Disconnect applies that snapshot before its fallback writer.
    mutation::run(CLIENT, &path, Format::JsonNanobotMcp, || {
        edit_gateway(&path, Some(&gateway()))
    })
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"agents\": {\"defaults\": {\"model\": \"mine\"}}"));
    let root: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(root["tools"]["web"]["enabled"], true);
    let gw = &root["tools"]["mcpServers"]["toolport"];
    assert_eq!(gw["env"]["TOOLPORT_CLIENT_ID"], "nanobot");
    assert!(gw.get("enabled").is_none());
    assert_eq!(parse(&text).unwrap().len(), 2);
    mutation::run(CLIENT, &path, Format::JsonNanobotMcp, || {
        mutation::disconnecting();
        assert!(restore::apply(CLIENT, Format::JsonNanobotMcp, &path)?);
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), CONFIG);
}
#[test]
fn missing_config_creates_nested_servers() {
    let dir = Dir::new();
    let path = dir.0.join("new.json");
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(root["tools"]["mcpServers"]["toolport"].is_object());
}
#[test]
fn remote_writes_explicit_nanobot_transport_and_headers() {
    for (kind, expected) in [("http", "streamableHttp"), ("sse", "sse")] {
        let entry:ServerEntry=serde_json::from_value(serde_json::json!({"name":"remote","transport":kind,"url":"https://a.test/sse","env":[{"key":"Authorization","value":"Bearer x"}]})).unwrap();
        let value = entry_value(&entry).unwrap();
        assert_eq!(value["type"], expected);
        assert_eq!(value["headers"]["Authorization"], "Bearer x");
        assert!(value.get("env").is_none());
    }
}
#[test]
fn malformed_or_ambiguous_configs_are_never_overwritten() {
    for text in [
        "[]",
        "{broken",
        "{\"tools\":null}",
        "{\"tools\":{\"mcpServers\":[]}}",
        "{\"tools\":{\"mcp_servers\":{}}}",
        "{\"tools\":{},\"tools\":{}}",
        "{ // comment\n \"tools\": {} }",
    ] {
        let dir = Dir::new();
        let path = dir.file(text);
        assert!(edit_gateway(&path, Some(&gateway())).is_err(), "{text}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }
}
#[test]
fn imports_refuse_settings_gateway_cannot_keep() {
    for (key, value) in [
        ("cwd", serde_json::json!("/tmp")),
        ("auth", serde_json::json!("oauth")),
        ("toolTimeout", serde_json::json!(90)),
        ("tool_timeout", serde_json::json!(90)),
        ("enabledTools", serde_json::json!(["echo"])),
        ("enabled_tools", serde_json::json!([])),
    ] {
        let mut definition = serde_json::json!({"command":"node"});
        definition[key] = value;
        assert!(importable("native", &definition).is_err(), "{key}");
    }
    assert!(importable(
        "native",
        &serde_json::json!({"command":"node","enabledTools":["*"],"enabled":false})
    )
    .is_ok());
}
#[test]
fn repair_keeps_gateway_timeouts_and_filters() {
    let dir = Dir::new();
    let path=dir.file(r#"{"tools":{"mcpServers":{"conduit":{"command":"conduit-gateway","toolTimeout":90,"enabledTools":["toolport_search_tools"]}}}}"#);
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let map = &root["tools"]["mcpServers"];
    assert!(map.get("conduit").is_none());
    assert_eq!(map["toolport"]["toolTimeout"], 90);
    assert_eq!(
        map["toolport"]["enabledTools"],
        serde_json::json!(["toolport_search_tools"])
    );
}
#[test]
fn moving_servers_keeps_other_tool_settings() {
    let dir = Dir::new();
    let path = dir.file(CONFIG);
    validate_import(&path, &["native".into()]).unwrap();
    assert!(validate_import(&path, &["gone".into()]).is_err());
    write_servers(&path, &[gateway()]).unwrap();
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(root["tools"]["web"]["enabled"], true);
    assert!(root["tools"]["mcpServers"].get("native").is_none());
}
#[test]
fn workspace_respects_custom_absolute_or_tilde_paths_and_refuses_invalid_config() {
    let dir = Dir::new();
    let home = &dir.0;
    assert_eq!(workspace(home), Some(home.join(".nanobot/workspace")));
    std::fs::create_dir_all(home.join(".nanobot")).unwrap();
    let config = home.join(".nanobot/config.json");
    for (raw, expected) in [
        ("~/my-work", Some(home.join("my-work"))),
        ("/srv/nanobot", Some(PathBuf::from("/srv/nanobot"))),
        ("relative", None),
    ] {
        std::fs::write(
            &config,
            serde_json::json!({"agents":{"defaults":{"workspace":raw}}}).to_string(),
        )
        .unwrap();
        assert_eq!(workspace(home), expected);
    }
    for invalid in [
        "invalid",
        r#"{"agents":null}"#,
        r#"{"agents":{"defaults":[]}}"#,
    ] {
        std::fs::write(&config, invalid).unwrap();
        assert!(workspace(home).is_none());
    }
}

#[test]
fn explicit_remote_type_overrides_stale_stdio_fields_in_inventory() {
    for (kind, expected) in [("sse", "sse"), ("streamableHttp", "http")] {
        let definition = serde_json::json!({"type":kind,"url":"https://example.test/mcp","command":"node","args":["old.js"],"env":{"STDIO_ONLY":"ignored"},"headers":{"Authorization":"Bearer x"},"requestInit":{"headers":{"PHANTOM_ONLY":"ignored"}}});
        let content = serde_json::json!({"tools":{"mcpServers":{"remote":definition}}}).to_string();
        let servers = parse(&content).unwrap();
        assert_eq!(servers[0].transport, expected);
        assert_eq!(servers[0].command, None);
        assert!(servers[0].args.is_empty());
        assert_eq!(servers[0].url.as_deref(), Some("https://example.test/mcp"));
        assert_eq!(servers[0].env_keys, ["Authorization"]);
        assert!(importable("remote", &definition).is_ok());
        let dir = Dir::new();
        let path = dir.file(&content);
        validate_import(&path, &["remote".into()]).unwrap();
    }
}

#[test]
fn explicit_transport_requires_its_active_address() {
    for definition in [
        serde_json::json!({"type":"stdio","url":"https://example.test/mcp"}),
        serde_json::json!({"type":"sse","command":"node"}),
    ] {
        assert!(normalized_server("broken", &definition).is_err());
    }
}
