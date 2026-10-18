use super::*;

/// What `dsh` writes for a fresh profile patch layer.
const TEMPLATE: &str = "# Your patch layer for this dsh profile, applied after every bundle layer:
# a top-level YAML array of loader patch entries (id-targeted config
# overrides, disables, and insert lists; `!!js` expressions allowed).
[]
";

/// A hand-written home layer: comments, a provider override with `!!js`, a memory
/// server whose env is an expression, and a second server in the same row.
const BUSY: &str = r#"# machine-wide dsh settings
- id: llm-pi-ai
  config:
    providers:
      my-gateway:
        apiKeyEnv: GATEWAY_API_KEY   # keep the key out of this file
        baseURL: https://gateway.example/v1

# memory, from the mcp-memory guide
- insert:
    - id: memory-mcp-reference
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: reference_memory
        transport: stdio
        command: mcp-server-memory
        cwd: !!js process.cwd()
        env:
          MEMORY_FILE_PATH: !!js >-
            process.env.MEMORY_FILE_PATH?.trim() || '/tmp/memory.jsonl'
    # remote search
    - id: mcp-web
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: web
        transport: streamable-http
        url: http://localhost:3000/mcp
        headers:
          Authorization: !!js '`Bearer ${process.env.MCP_TOKEN}`'
"#;

fn stdio(name: &str) -> ServerEntry {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "transport": "stdio",
        "command": "/opt/toolport/toolport-gateway",
        "args": ["--stdio"],
        "env": [{"key": "TOOLPORT_CLIENT_ID", "value": "deepseek-harness"}],
    }))
    .unwrap()
}

fn desired(text: &str) -> serde_json::Map<String, Json> {
    normalized(text).unwrap()[KEY]
        .as_object()
        .cloned()
        .unwrap_or_default()
}

fn with(text: &str, entry: &ServerEntry) -> String {
    let mut want = desired(text);
    want.insert(entry.name.clone(), config_of(entry).unwrap());
    sync(Some(text), &want).unwrap()
}

fn without(text: &str, name: &str) -> String {
    let mut want = desired(text);
    want.remove(name);
    sync(Some(text), &want).unwrap()
}

#[test]
fn the_template_gets_one_insert_row_and_goes_back_to_empty() {
    let added = with(TEMPLATE, &stdio("toolport"));
    assert!(added.starts_with("# Your patch layer"), "{added}");
    assert!(!added.contains("[]"), "{added}");
    assert!(added.ends_with(
        "- insert:\n    - id: mcp-toolport\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: toolport\n        transport: stdio\n        command: /opt/toolport/toolport-gateway\n        args:\n        - --stdio\n        env:\n          TOOLPORT_CLIENT_ID: deepseek-harness\n"
    ), "{added}");
    let listed = servers(&added).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "toolport");
    assert_eq!(listed[0].transport, "stdio");
    assert_eq!(listed[0].args, ["--stdio"]);
    assert_eq!(listed[0].env_keys, ["TOOLPORT_CLIENT_ID"]);

    let removed = without(&added, "toolport");
    assert!(servers(&removed).unwrap().is_empty());
    assert!(removed.starts_with("# Your patch layer"), "{removed}");
    assert!(removed.trim_end().ends_with("[]"), "{removed}");
}

#[test]
fn an_empty_or_missing_file_starts_a_list() {
    for start in [None, Some(""), Some("\n# only a note\n")] {
        let mut want = serde_json::Map::new();
        want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
        let out = sync(start, &want).unwrap();
        assert_eq!(servers(&out).unwrap().len(), 1, "{out}");
        assert!(out.contains("- insert:\n    - id: mcp-toolport\n"), "{out}");
    }
}

#[test]
fn a_busy_file_keeps_every_byte_outside_the_gateway_row() {
    let listed = servers(BUSY).unwrap();
    let names: Vec<_> = listed.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["reference_memory", "web"]);
    assert_eq!(listed[0].env_keys, ["MEMORY_FILE_PATH"]);
    assert_eq!(listed[1].transport, "http");
    assert_eq!(listed[1].url.as_deref(), Some("http://localhost:3000/mcp"));
    assert_eq!(listed[1].env_keys, ["Authorization"]);

    let added = with(BUSY, &stdio("toolport"));
    assert!(added.starts_with(BUSY), "{added}");
    let removed = without(&added, "toolport");
    assert_eq!(removed, format!("{BUSY}\n"));
    assert_eq!(normalized(&removed).unwrap(), normalized(BUSY).unwrap());
}

#[test]
fn removing_one_server_from_a_shared_row_takes_only_its_lines() {
    let out = without(BUSY, "reference_memory");
    assert!(out.contains("# memory, from the mcp-memory guide\n- insert:\n    # remote search\n    - id: mcp-web\n"), "{out}");
    assert!(!out.contains("memory-mcp-reference"), "{out}");
    assert!(
        out.contains("apiKeyEnv: GATEWAY_API_KEY   # keep the key out of this file"),
        "{out}"
    );
    let names: Vec<_> = servers(&out).unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["web"]);

    // The last server leaves the whole row, never an empty `insert:`.
    let out = without(&out, "web");
    assert!(!out.contains("insert"), "{out}");
    assert!(out.contains("- id: llm-pi-ai"), "{out}");
}

#[test]
fn a_changed_server_keeps_its_id_and_moves_to_a_new_row() {
    let added = with(BUSY, &stdio("toolport"));
    let mut entry = stdio("toolport");
    entry.args = vec!["--stdio".into(), "--profile".into(), "Work".into()];
    let changed = with(&added, &entry);
    assert_eq!(changed.matches("id: mcp-toolport").count(), 1, "{changed}");
    let gateway = servers(&changed)
        .unwrap()
        .into_iter()
        .find(|s| s.name == "toolport")
        .unwrap();
    assert_eq!(gateway.args, ["--stdio", "--profile", "Work"]);
}

#[test]
fn expressions_survive_being_moved_out_and_back() {
    let raw = raw_entries(BUSY).unwrap();
    assert_eq!(raw.len(), 2);
    let (name, text, _) = &raw[0];
    assert_eq!(name, "reference_memory");
    assert!(
        text.starts_with("id: memory-mcp-reference\nname: '@deepseek-ai/dsh-mcp-client'\n"),
        "{text}"
    );
    assert!(text.contains("cwd: !!js process.cwd()"), "{text}");

    let emptied = without(&without(BUSY, "reference_memory"), "web");
    let restored = append_raw(
        Some(&emptied),
        &raw.iter()
            .map(|(_, text, _)| text.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        restored.contains("        cwd: !!js process.cwd()\n"),
        "{restored}"
    );
    assert!(restored.contains("    # remote search\n") || !restored.contains("# remote search"));
    assert_eq!(
        normalized(&restored).unwrap()[KEY],
        normalized(BUSY).unwrap()[KEY]
    );
}

#[test]
fn tagged_values_compare_and_write_back_as_the_same_tag() {
    let doc = parse(BUSY).unwrap();
    let memory = &doc.servers()[0];
    let json = to_json(&memory.config);
    let text = entry_text("mcp-x", None, &json).unwrap();
    let back: Yaml = serde_yaml::from_str(&text).unwrap();
    assert_eq!(back["config"], memory.config, "{text}");
}

#[test]
fn a_disabled_server_is_not_listed_and_is_left_alone() {
    let text = format!("{BUSY}\n- id: mcp-web\n  disabled: true\n");
    let names: Vec<_> = servers(&text)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["reference_memory"]);
    let added = with(&text, &stdio("toolport"));
    assert!(added.starts_with(&text), "{added}");
    // Re-adding a disabled name gets a fresh id rather than reviving the old row.
    let readded = with(
        &text,
        &serde_json::from_value::<ServerEntry>(serde_json::json!({
            "name": "web", "transport": "http", "url": "http://localhost:4000/mcp",
        }))
        .unwrap(),
    );
    assert!(readded.contains("id: mcp-web-2"), "{readded}");
}

#[test]
fn layouts_the_scan_cannot_split_are_read_but_never_edited() {
    let flow = "[{insert: [{id: mcp-a, name: '@deepseek-ai/dsh-mcp-client', config: {serverName: a, transport: stdio, command: a}}]}]\n";
    assert_eq!(servers(flow).unwrap().len(), 1);
    let mut want = desired(flow);
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    assert!(sync(Some(flow), &want)
        .unwrap_err()
        .contains("edit it by hand"));
    // Nothing to change is not an edit.
    assert_eq!(sync(Some(flow), &desired(flow)).unwrap(), flow);

    let inline = "- insert: [{id: mcp-a, name: '@deepseek-ai/dsh-mcp-client', config: {serverName: a, transport: stdio, command: a}}]\n";
    let mut want = desired(inline);
    want.remove("a");
    assert!(sync(Some(inline), &want)
        .unwrap_err()
        .contains("edit it by hand"));
    // Appending next to a row it cannot split is still exact.
    let mut want = desired(inline);
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    let out = sync(Some(inline), &want).unwrap();
    assert!(out.starts_with(inline), "{out}");
}

#[test]
fn a_file_that_is_not_a_list_is_refused() {
    let err = normalized("mcp:\n  servers: {}\n").unwrap_err();
    assert!(err.contains("top-level list"), "{err}");
    let err = normalized("- insert: [\n").unwrap_err();
    assert!(err.contains("Could not parse"), "{err}");
}

#[test]
fn names_and_transports_dsh_cannot_load_are_refused() {
    let mut entry = stdio("my server");
    assert!(config_of(&entry).unwrap_err().contains("letters, digits"));
    entry.name = "x".repeat(33);
    assert!(config_of(&entry).is_err());
    let sse: ServerEntry = serde_json::from_value(serde_json::json!({
        "name": "events", "transport": "sse", "url": "http://localhost:1/sse",
    }))
    .unwrap();
    assert!(config_of(&sse).unwrap_err().contains("not SSE"));
    let http: ServerEntry = serde_json::from_value(serde_json::json!({
        "name": "web", "transport": "http", "url": "http://localhost:1/mcp",
        "env": [{"key": "Authorization", "value": "Bearer x"}],
    }))
    .unwrap();
    let config = config_of(&http).unwrap();
    assert_eq!(config["transport"], "streamable-http");
    assert_eq!(config["headers"]["Authorization"], "Bearer x");
    assert!(config.get("env").is_none());
}

#[test]
fn crlf_files_keep_their_line_endings() {
    let text = BUSY.replace('\n', "\r\n");
    assert_eq!(servers(&text).unwrap().len(), 2);
    let out = without(&text, "web");
    assert!(out.contains("\r\n"), "{out}");
    assert!(!out.contains("mcp-web"), "{out}");
    assert_eq!(servers(&out).unwrap().len(), 1);
    let added = with(&text, &stdio("toolport"));
    assert!(added.starts_with(&text), "{added}");
    assert!(
        !added.replace("\r\n", "").contains('\n'),
        "every line Toolport adds ends in CRLF: {added:?}"
    );
}

#[test]
fn a_disabled_server_counts_as_other_settings() {
    let text = format!("{BUSY}\n- id: mcp-web\n  disabled: true\n");
    let rows = normalized(&text).unwrap()["rows"].to_string();
    assert!(rows.contains("http://localhost:3000/mcp"), "{rows}");
    assert!(!rows.contains("mcp-server-memory"), "{rows}");
}
