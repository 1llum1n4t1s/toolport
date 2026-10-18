use super::*;

/// A hand-written JSON5 config, the way OpenClaw's docs show it.
const CONFIG: &str = r#"{
  // Gateway
  gateway: { port: 18789 },
  mcp: {
    servers: {
      docs: {
        url: 'https://mcp.example.com/mcp',
        transport: 'streamable-http',
        toolFilter: { include: ['search'] },
      },
      legacy: { url: 'https://old.example.com/sse' },
      local: { command: 'node', args: ['./server.js'], env: { A: '1' } },
      off: { command: 'off-server', enabled: false },
    },
  },
}
"#;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("toolport-openclaw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn file(&self, text: &str) -> PathBuf {
        let path = self.0.join("openclaw.json");
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
        "env": [{"key": "TOOLPORT_CLIENT_ID", "value": "openclaw"}],
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
fn reads_started_servers_with_openclaws_transport_rules() {
    let listed = parse(CONFIG).unwrap();
    let kinds: Vec<_> = listed
        .iter()
        .map(|s| (s.name.as_str(), s.transport.as_str()))
        .collect();
    // A URL without `transport` is SSE; a turned-off server isn't listed.
    assert_eq!(
        kinds,
        [("docs", "http"), ("legacy", "sse"), ("local", "stdio")]
    );
    assert_eq!(listed[2].args, ["./server.js"]);
    assert_eq!(listed[2].env_keys, ["A"]);
}

#[test]
fn connect_keeps_the_json5_text_and_disconnect_gives_back_the_bytes() {
    let dir = Dir::new("connect");
    let path = dir.file(CONFIG);
    edit_gateway(&path, Some(&gateway())).unwrap();
    let added = std::fs::read_to_string(&path).unwrap();
    assert!(added.contains("// Gateway"), "{added}");
    assert!(
        added.contains("legacy: { url: 'https://old.example.com/sse' },"),
        "{added}"
    );
    let root = parse_json_value(&added).unwrap();
    let entry = &root["mcp"]["servers"]["toolport"];
    assert_eq!(entry["command"], "/opt/toolport/toolport-gateway");
    assert_eq!(entry["env"]["TOOLPORT_CLIENT_ID"], "openclaw");
    assert!(entry.get("type").is_none(), "{entry}");
    assert_eq!(parse(&added).unwrap().len(), 4);
    edit_gateway(&path, None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CONFIG);
}

#[test]
fn a_missing_config_starts_with_mcp_servers() {
    let dir = Dir::new("missing");
    let path = dir.0.join("openclaw.json");
    edit_gateway(&path, Some(&gateway())).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    assert!(root["mcp"]["servers"]["toolport"].is_object(), "{text}");
}

#[test]
fn remote_servers_always_name_their_transport() {
    let http = entry_value(&remote("http")).unwrap();
    assert_eq!(http["transport"], "streamable-http");
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
    let root = parse_json_value(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let names: Vec<_> = root["mcp"]["servers"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(names, ["off", "toolport"]);
    assert_eq!(root["gateway"]["port"], 18789);
}

#[test]
fn a_turned_off_gateway_stays_off_when_repaired() {
    let dir = Dir::new("repair");
    let path = dir
        .file(r#"{"mcp":{"servers":{"toolport":{"command":"toolport-gateway","enabled":false}}}}"#);
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root = parse_json_value(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let entry = &root["mcp"]["servers"]["toolport"];
    assert_eq!(entry["enabled"], false);
    assert_eq!(entry["command"], "/opt/toolport/toolport-gateway");
    // A turned-off gateway still counts as connected, so Disconnect finds it.
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(parse(&text).unwrap()[0].name, "toolport");
    edit_gateway(&path, None).unwrap();
    assert!(parse(&std::fs::read_to_string(&path).unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn a_repaired_gateway_keeps_the_openclaw_settings_it_was_given() {
    let dir = Dir::new("settings");
    let path = dir.file(r#"{"mcp":{"servers":{"toolport":{"command":"old/toolport-gateway","toolFilter":{"include":["a*"]},"requestTimeoutMs":90000}}}}"#);
    edit_gateway(&path, Some(&gateway())).unwrap();
    let root = parse_json_value(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let entry = &root["mcp"]["servers"]["toolport"];
    assert_eq!(entry["command"], "/opt/toolport/toolport-gateway");
    assert_eq!(entry["toolFilter"]["include"][0], "a*");
    assert_eq!(entry["requestTimeoutMs"], 90000);
}

#[test]
fn imports_refuse_settings_toolport_would_drop() {
    let dir = Dir::new("import");
    let path = dir.file(CONFIG);
    validate_import(&path, &["local".into(), "legacy".into()], true).unwrap();
    let error = validate_import(&path, &["docs".into()], false).unwrap_err();
    assert!(error.contains("toolFilter"), "{error}");
    let error = validate_import(&path, &["off".into()], true).unwrap_err();
    assert!(error.contains("turned-off"), "{error}");
    validate_import(&path, &["off".into()], false).unwrap();
    let included =
        dir.file(r#"{"mcp":{"servers":{"a":{"command":"a","env":{"$include":"./env.json5"}}}}}"#);
    let error = validate_import(&included, &["a".into()], false).unwrap_err();
    assert!(error.contains("$include"), "{error}");
}

#[test]
fn includes_and_duplicate_keys_are_refused() {
    for text in [
        r#"{"$include": "./base.json5"}"#,
        r#"{"mcp": {"$include": "./mcp.json5"}}"#,
        r#"{"mcp": {"servers": {"$include": ["./a.json5"]}}}"#,
        r#"{"mcp": {"servers": {"a": {"$include": "./a.json5"}}}}"#,
    ] {
        assert!(parse(text).unwrap_err().contains("$include"), "{text}");
        let dir = Dir::new("include");
        let path = dir.file(text);
        assert!(edit_gateway(&path, Some(&gateway())).is_err(), "{text}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    let duplicate = r#"{"mcp": {"servers": {}}, "mcp": {"servers": {}}}"#;
    assert!(parse(duplicate).unwrap_err().contains("duplicate"));
}

#[test]
fn dirs_follow_openclaws_environment() {
    let home = Dir::new("dirs");
    let user = home.0.clone();
    let none = |_: &str| None;
    let plain = dirs(&user, &none);
    assert_eq!(plain.config, user.join(".openclaw/openclaw.json"));
    assert_eq!(plain.workspace, user.join(".openclaw/workspace"));
    assert!(plain.default_state);

    // The legacy state dir is used when it is the only one.
    std::fs::create_dir_all(user.join(".clawdbot")).unwrap();
    assert_eq!(
        dirs(&user, &none).config,
        user.join(".clawdbot/openclaw.json")
    );
    std::fs::create_dir_all(user.join(".openclaw")).unwrap();
    assert_eq!(dirs(&user, &none).state, user.join(".openclaw"));

    let set = |pairs: Vec<(&'static str, String)>| {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        }
    };
    let moved = dirs(&user, &set(vec![("OPENCLAW_HOME", "~/oc".into())]));
    assert_eq!(moved.config, user.join("oc/.openclaw/openclaw.json"));
    let srv = user.join("srv");
    let state = dirs(
        &user,
        &set(vec![
            ("OPENCLAW_HOME", srv.display().to_string()),
            ("OPENCLAW_STATE_DIR", "~/sd".into()),
        ]),
    );
    assert_eq!(state.state, srv.join("sd"));
    assert_eq!(state.workspace, srv.join("sd").join("workspace"));
    assert!(!state.default_state);
    let file = dirs(
        &user,
        &set(vec![("OPENCLAW_CONFIG_PATH", "~/x/c.json".into())]),
    );
    assert_eq!(file.config, user.join("x/c.json"));
    // OpenClaw resolves a relative value against its own cwd, so it's flagged.
    let relative = dirs(&user, &set(vec![("OPENCLAW_STATE_DIR", "sd".into())]));
    assert!(relative.relative);
    assert!(!plain.relative);

    // The workspace follows the config, and the env var wins over it.
    std::fs::write(
        user.join(".openclaw/openclaw.json"),
        "{ agents: { defaults: { workspace: '~/work' } } }",
    )
    .unwrap();
    assert_eq!(dirs(&user, &none).workspace, user.join("work"));
    let elsewhere = user.join("w");
    let env = dirs(
        &user,
        &set(vec![(
            "OPENCLAW_WORKSPACE_DIR",
            elsewhere.display().to_string(),
        )]),
    );
    assert_eq!(env.workspace, user.join("work"));
    std::fs::write(user.join(".openclaw/openclaw.json"), "{}").unwrap();
    let env = dirs(
        &user,
        &set(vec![(
            "OPENCLAW_WORKSPACE_DIR",
            elsewhere.display().to_string(),
        )]),
    );
    assert_eq!(env.workspace, elsewhere);
    assert!(env.workspace_known);
}

#[test]
fn workspace_selection_matches_the_default_agents_list() {
    let fixture = Dir::new("workspace-selection");
    let home = fixture.0.clone();
    let state = home.join(".openclaw");
    let env_dir = home.join("env");
    let env_path = env_dir.to_str().unwrap();
    for (root, raw_env, expected) in [
        (
            serde_json::json!({"agents":{"defaults":{"workspace":"~//configured"}}}),
            Some(env_path),
            Some(home.join("configured")),
        ),
        (
            serde_json::json!({"agents":{"defaults":{"workspace":"~/configured"}}}),
            Some(env_path),
            Some(home.join("configured")),
        ),
        (
            serde_json::json!({"agents":{"defaults":{"workspace":"relative"}}}),
            Some(env_path),
            None,
        ),
        (
            serde_json::json!({"agents":{"entries":{"solo":{"workspace":"~/solo"}}}}),
            Some(env_path),
            Some(home.join("solo")),
        ),
        (
            serde_json::json!({"agents":{"list":[{"id":"solo","workspace":"~/legacy"}]}}),
            Some(env_path),
            Some(home.join("legacy")),
        ),
        (
            serde_json::json!({"agents":{"entries":{"one":{},"two":{}}}}),
            None,
            None,
        ),
        (serde_json::json!({"agents":{"entries":{}}}), None, None),
        (serde_json::json!({"agents":{"list":[]}}), None, None),
        (
            serde_json::json!({"agents":{"entries":{"one":{"workspace":"~/one"},"two":{"workspace":"~/two","default":true}}}}),
            None,
            Some(home.join("two")),
        ),
        (
            serde_json::json!({"agents":{"ownership":"explicit","defaults":{"workspace":"~/all","systemAgent":{"agentId":"two"}},"entries":{"one":{},"two":{}}}}),
            Some(env_path),
            Some(home.join("all/two")),
        ),
        (serde_json::json!({}), Some("~/env"), None),
        (serde_json::json!({}), Some("relative"), None),
        (
            serde_json::json!({}),
            Some(env_path),
            Some(PathBuf::from(env_path)),
        ),
        (serde_json::json!({"$include":"elsewhere.json"}), None, None),
    ] {
        let env = |key: &str| {
            (key == "OPENCLAW_WORKSPACE_DIR")
                .then(|| raw_env.map(str::to_string))
                .flatten()
        };
        assert_eq!(
            default_workspace(&home, &state, Some(&root), &env),
            expected,
            "{root} {raw_env:?}"
        );
    }
}

#[test]
fn automatic_legacy_state_does_not_move_the_default_workspace() {
    let fixture = Dir::new("legacy-workspace");
    let home = &fixture.0;
    let legacy = home.join(".clawdbot");
    let root = serde_json::json!({});
    assert_eq!(
        default_workspace(home, &legacy, Some(&root), &|_| None),
        Some(home.join(".openclaw/workspace"))
    );
    let state = |key: &str| (key == "OPENCLAW_STATE_DIR").then(|| legacy.display().to_string());
    assert_eq!(
        default_workspace(home, &legacy, Some(&root), &state),
        Some(legacy.join("workspace"))
    );
}
