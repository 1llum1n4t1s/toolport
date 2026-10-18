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
    sync(Some(text), &want, &BTreeMap::new()).unwrap()
}

fn without(text: &str, name: &str) -> String {
    let mut want = desired(text);
    want.remove(name);
    sync(Some(text), &want, &BTreeMap::new()).unwrap()
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
        let out = sync(start, &want, &BTreeMap::new()).unwrap();
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
fn a_changed_server_keeps_its_id_and_its_place() {
    let added = with(BUSY, &stdio("toolport"));
    let mut entry = stdio("toolport");
    entry.args = vec!["--stdio".into(), "--profile".into(), "Work".into()];
    let changed = with(&added, &entry);
    assert_eq!(changed.matches("id: mcp-toolport").count(), 1, "{changed}");
    assert_eq!(changed.matches("- insert:").count(), 2, "{changed}");
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
    assert_eq!(
        json["cwd"],
        serde_json::json!({"!tag": "!!js", "value": "process.cwd()"})
    );
    let text = entry_text("mcp-x", None, &json).unwrap();
    assert!(text.contains("cwd: !!js process.cwd()"), "{text}");
    assert!(text.contains("MEMORY_FILE_PATH: !!js "), "{text}");
    let back: Yaml = yaml_from(&text).unwrap();
    assert_eq!(back["config"], memory.config, "{text}");
    // `!!` inside a string is text, not a tag.
    let shout = "- insert:\n    - id: mcp-a\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: a, transport: stdio, command: echo, args: ['hi!!js', 'x: !!js y']}\n";
    assert_eq!(servers(shout).unwrap()[0].args, ["hi!!js", "x: !!js y"]);
}

#[test]
fn a_server_put_back_uses_its_recorded_entry() {
    let raw: BTreeMap<String, String> = raw_entries(BUSY)
        .unwrap()
        .into_iter()
        .map(|(name, text, _)| (name, text))
        .collect();
    let moved = without(BUSY, "reference_memory");
    let back = sync(Some(&moved), &desired(BUSY), &raw).unwrap();
    assert!(back.contains("    - id: memory-mcp-reference\n"), "{back}");
    assert!(back.contains("        cwd: !!js process.cwd()\n"), "{back}");
    assert_eq!(desired(&back), desired(BUSY));

    // Without a record it is written fresh, expressions still tagged.
    let fresh = sync(Some(&moved), &desired(BUSY), &BTreeMap::new()).unwrap();
    assert!(fresh.contains("id: mcp-reference_memory"), "{fresh}");
    assert!(fresh.contains("cwd: !!js process.cwd()"), "{fresh}");
    assert_eq!(desired(&fresh), desired(BUSY));
}

#[test]
fn rows_apply_in_order_and_servers_they_change_are_left_alone() {
    let enabled = "- insert:\n    - id: mcp-a\n      name: '@deepseek-ai/dsh-mcp-client'\n      disabled: true\n      config: {serverName: a, transport: stdio, command: a}\n- id: mcp-a\n  disabled: false\n";
    assert_eq!(servers(enabled).unwrap().len(), 1);
    // Another layer's row before the insert does not apply to it.
    let before = format!(
        "- id: mcp-web\n  disabled: true\n{}",
        &BUSY[BUSY.find("- insert").unwrap()..]
    );
    assert_eq!(servers(&before).unwrap().len(), 2);

    let config = format!(
        "{BUSY}\n- id: mcp-web\n  config: {{serverName: web, transport: stdio, command: other}}\n"
    );
    let web = servers(&config)
        .unwrap()
        .into_iter()
        .find(|s| s.name == "web")
        .unwrap();
    assert_eq!(web.command.as_deref(), Some("other"));
    let mut want = desired(&config);
    want.remove("web");
    assert!(sync(Some(&config), &want, &BTreeMap::new())
        .unwrap_err()
        .contains("later row"));
    assert!(raw_entries(&config)
        .unwrap()
        .iter()
        .all(|(name, _, _)| name != "web"));
    let added = with(&config, &stdio("toolport"));
    assert!(added.starts_with(&config), "{added}");
}

#[test]
fn a_new_id_avoids_every_id_in_the_file() {
    let text = "- id: mcp-toolport\n  disabled: true\n";
    let out = with(text, &stdio("toolport"));
    assert!(out.contains("id: mcp-toolport-2"), "{out}");
    assert_eq!(servers(&out).unwrap().len(), 1);
}

#[test]
fn two_servers_with_one_name_are_not_merged() {
    let text = format!("{BUSY}- insert:\n    - id: mcp-web-copy\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {{serverName: web, transport: stdio, command: web}}\n");
    let mut want = desired(&text);
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    assert!(sync(Some(&text), &want, &BTreeMap::new())
        .unwrap_err()
        .contains("Two DeepSeek Harness servers"));
    assert_eq!(
        sync(Some(&text), &desired(&text), &BTreeMap::new()).unwrap(),
        text
    );
}

#[test]
fn odd_but_valid_files_still_edit_cleanly() {
    // The empty list never lands on a comment line.
    let text = "- insert:\n    - id: mcp-toolport\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: toolport, transport: stdio, command: t}\n# end";
    let out = without(text, "toolport");
    assert_eq!(out, "# end\n[]\n");
    assert!(normalized(&out).unwrap().get(KEY).is_none());

    // A shallower comment with multibyte text inside an entry.
    let note = "- insert:\n    - id: mcp-a\n# 记忆服务\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: a, transport: stdio, command: a}\n";
    assert_eq!(servers(note).unwrap().len(), 1);
    assert!(raw_entries(note).unwrap()[0].1.contains("# 记忆服务"));

    // A byte order mark stays and does not block edits.
    let bom = format!("\u{feff}{BUSY}");
    let out = without(&bom, "web");
    assert!(out.starts_with('\u{feff}'), "{out}");
    assert_eq!(servers(&out).unwrap().len(), 1);

    // An item that opens with a bare dash moves out as a plain mapping.
    let bare = "- insert:\n    -\n      id: mcp-a\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: a, transport: stdio, command: a}\n";
    let raw = raw_entries(bare).unwrap();
    assert!(raw[0].1.starts_with("id: mcp-a\nname:"), "{}", raw[0].1);
    let back = append_raw(Some(&without(bare, "a")), &[raw[0].1.clone()]).unwrap();
    assert_eq!(desired(&back), desired(bare));
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
    assert!(sync(Some(flow), &want, &BTreeMap::new())
        .unwrap_err()
        .contains("edit it by hand"));
    // Nothing to change is not an edit.
    assert_eq!(
        sync(Some(flow), &desired(flow), &BTreeMap::new()).unwrap(),
        flow
    );

    let inline = "- insert: [{id: mcp-a, name: '@deepseek-ai/dsh-mcp-client', config: {serverName: a, transport: stdio, command: a}}]\n";
    let mut want = desired(inline);
    want.remove("a");
    assert!(sync(Some(inline), &want, &BTreeMap::new())
        .unwrap_err()
        .contains("edit it by hand"));
    // Appending next to a row it cannot split is still exact.
    let mut want = desired(inline);
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    let out = sync(Some(inline), &want, &BTreeMap::new()).unwrap();
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

const LOCAL_GROUP: &str = "- insert:\n    - id: tools\n      group: true\n      config: []\n- id: tools\n  insert:\n    - id: local-server\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: local\n        transport: stdio\n        command: /local/server\n";

#[test]
fn locally_resolved_groups_can_connect_and_edit_their_server() {
    assert_eq!(servers(LOCAL_GROUP).unwrap()[0].name, "local");
    let connected = with(LOCAL_GROUP, &stdio("toolport"));
    assert!(connected.starts_with(LOCAL_GROUP), "{connected}");
    assert_eq!(
        without(&connected, "toolport").trim_end(),
        LOCAL_GROUP.trim_end()
    );
    let mut moved = desired(LOCAL_GROUP);
    moved.remove("local");
    assert!(sync(Some(LOCAL_GROUP), &moved, &Default::default())
        .unwrap_err()
        .contains("activation context"));
    assert!(raw_entries(LOCAL_GROUP).unwrap().is_empty());
    let mut want = desired(LOCAL_GROUP);
    want["local"]["command"] = "/updated/server".into();
    let edited = sync(Some(LOCAL_GROUP), &want, &Default::default()).unwrap();
    assert!(edited.contains("- id: tools\n  insert:"));
    assert_eq!(
        servers(&edited).unwrap()[0].command.as_deref(),
        Some("/updated/server")
    );
}

#[test]
fn skipped_and_disabled_group_entries_do_not_block_connect() {
    for text in [
        LOCAL_GROUP.replace("- id: tools\n  insert:", "- id: missing\n  insert:"),
        LOCAL_GROUP.replace("group: true", "group: false"),
        LOCAL_GROUP.replace("group: true", "group: true\n      disabled: true"),
        format!("{LOCAL_GROUP}- id: tools\n  disabled: true\n"),
        format!("{LOCAL_GROUP}- id: tools\n  config: []\n"),
        LOCAL_GROUP.replace(
            "- id: tools\n  insert:",
            "- id: !!js process.env.GROUP_ID\n  insert:",
        ),
    ] {
        assert!(servers(&text).unwrap().is_empty());
        let connected = with(&text, &stdio("toolport"));
        assert!(connected.starts_with(&text), "{connected}");
        assert_eq!(without(&connected, "toolport").trim_end(), text.trim_end());
    }
    let unrelated = "- insert:\n    - id: tools\n      group: true\n      config: []\n- id: tools\n  insert: []\n- id: profile-group\n  insert:\n    - id: other\n      name: other-plugin\n";
    assert!(with(unrelated, &stdio("toolport")).starts_with(unrelated));
}

#[test]
fn group_context_is_kept_when_editing_unrelated_servers() {
    // A server nested inside a group config has no independent line span.
    let text = "- insert:\n    - id: tools\n      group: true\n      config:\n        - id: nested\n          name: '@deepseek-ai/dsh-mcp-client'\n          config: {serverName: nested, transport: stdio, command: nested}\n";
    assert_eq!(servers(text).unwrap()[0].name, "nested");
    assert!(with(text, &stdio("toolport")).starts_with(text));
    let disabled = format!("{text}- id: tools\n  disabled: true\n");
    assert!(servers(&disabled).unwrap().is_empty());
}

#[test]
fn group_replacement_children_are_not_new_patch_targets() {
    let child = "    - id: new-server\n      name: '@deepseek-ai/dsh-mcp-client'\n      config: {serverName: new, transport: stdio, command: /new/server}\n";
    let replaced = format!("- insert:\n    - id: tools\n      group: true\n      config: []\n- id: tools\n  config:\n{child}- id: new-server\n  config: {{serverName: new, transport: stdio, command: /wrong/server}}\n");
    assert_eq!(
        servers(&replaced).unwrap()[0].command.as_deref(),
        Some("/new/server")
    );
    assert!(with(&replaced, &stdio("toolport")).starts_with(&replaced));
    for text in [
        format!("- insert:\n    - id: tools\n      group: false\n      config:\n{}- id: tools\n  group: true\n", child.lines().map(|line| format!("    {line}\n")).collect::<String>()),
        format!("- insert:\n    - id: tools\n      group: true\n      config: []\n- id: tools\n  group: false\n  config:\n{child}- id: tools\n  group: true\n"),
    ] {
        assert_eq!(servers(&text).unwrap()[0].command.as_deref(), Some("/new/server"));
    }
}
