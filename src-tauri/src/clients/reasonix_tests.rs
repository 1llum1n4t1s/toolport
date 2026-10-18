use super::*;

/// The shape `reasonix mcp add` leaves: the commented template, then plugins.
const CONFIG: &str = r#"# Reasonix configuration.
config_version = 12   # schema marker for diagnostics
default_model = "deepseek-flash"

[ui]
theme = "auto"   # auto|dark|light

[skills]
# disabled_skills = ["review"]   # hide noisy or unwanted skills

# External MCP servers. type: "stdio" (default, a subprocess) | "http" | "sse".

[[plugins]]
name    = "demo"
command = "python3"   # keep this note
args    = ["/opt/demo/server.py"]

[[plugins]]
name    = "web"
type    = "http"
url     = "https://example.test/mcp"
headers = { Authorization = "Bearer ${WEB_TOKEN}" }
"#;

fn stdio(name: &str) -> ServerEntry {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "transport": "stdio",
        "command": "/opt/toolport/toolport-gateway",
        "args": ["--stdio"],
        "env": [{"key": "TOOLPORT_CLIENT_ID", "value": "reasonix"}],
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
    sync(Some(text), &want, &Default::default()).unwrap()
}

fn without(text: &str, name: &str) -> String {
    let mut want = desired(text);
    want.remove(name);
    sync(Some(text), &want, &Default::default()).unwrap()
}

#[test]
fn reads_stdio_and_remote_plugins() {
    let listed = servers(CONFIG).unwrap();
    let names: Vec<_> = listed.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["demo", "web"]);
    assert_eq!(listed[0].transport, "stdio");
    assert_eq!(listed[0].args, ["/opt/demo/server.py"]);
    assert_eq!(listed[1].transport, "http");
    assert_eq!(listed[1].url.as_deref(), Some("https://example.test/mcp"));
    assert_eq!(listed[1].env_keys, ["Authorization"]);
}

#[test]
fn connect_appends_one_table_and_disconnect_restores_the_bytes() {
    let added = with(CONFIG, &stdio("toolport"));
    assert!(added.starts_with(CONFIG), "{added}");
    assert!(added.ends_with(
        "\n[[plugins]]\nname = \"toolport\"\ncommand = \"/opt/toolport/toolport-gateway\"\nargs = [\"--stdio\"]\nenv = { TOOLPORT_CLIENT_ID = \"reasonix\" }\n"
    ), "{added}");
    let removed = without(&added, "toolport");
    assert_eq!(removed, CONFIG);
}

#[test]
fn a_missing_or_empty_config_starts_with_the_one_table() {
    for start in [None, Some(""), Some("plugins = []\n")] {
        let mut want = serde_json::Map::new();
        want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
        let out = sync(start, &want, &Default::default()).unwrap();
        assert_eq!(servers(&out).unwrap().len(), 1, "{out}");
        assert!(out.contains("[[plugins]]\nname = \"toolport\""), "{out}");
    }
}

#[test]
fn a_changed_table_is_edited_in_place_and_keeps_its_comments() {
    let mut want = desired(CONFIG);
    want["demo"]["args"] = serde_json::json!(["/opt/demo/server.py", "--verbose"]);
    let out = sync(Some(CONFIG), &want, &Default::default()).unwrap();
    assert!(
        out.contains("command = \"python3\"   # keep this note"),
        "{out}"
    );
    assert!(out.find("name    = \"demo\"").unwrap() < out.find("name    = \"web\"").unwrap());
    assert_eq!(
        servers(&out).unwrap()[0].args,
        ["/opt/demo/server.py", "--verbose"]
    );
}

#[test]
fn removing_a_server_leaves_the_others_and_every_setting() {
    let out = without(CONFIG, "demo");
    assert!(!out.contains("python3"), "{out}");
    assert!(
        out.contains("headers = { Authorization = \"Bearer ${WEB_TOKEN}\" }"),
        "{out}"
    );
    assert!(out.contains("# disabled_skills = [\"review\"]"), "{out}");
    assert_eq!(
        normalized(&out).unwrap()["rest"],
        normalized(CONFIG).unwrap()["rest"]
    );
    let empty = without(&out, "web");
    assert!(!empty.contains("[[plugins]]"), "{empty}");
    assert!(
        empty.contains("default_model = \"deepseek-flash\""),
        "{empty}"
    );
}

fn texts(raw: &[(String, String, Json)]) -> Vec<String> {
    raw.iter().map(|(_, text, _)| text.clone()).collect()
}

fn recorded(text: &str) -> std::collections::BTreeMap<String, String> {
    raw_entries(text)
        .unwrap()
        .into_iter()
        .map(|(name, text, _)| (name, text))
        .collect()
}

#[test]
fn tables_move_out_and_back_with_their_comments() {
    let raw = raw_entries(CONFIG).unwrap();
    assert_eq!(raw.len(), 2);
    assert!(raw[0].1.contains("# keep this note"), "{}", raw[0].1);
    // Leading comments stay while connected and return to their table on restore.
    assert!(raw[0].1.contains("External MCP servers"), "{}", raw[0].1);
    assert_eq!(definition(&raw[0].1).unwrap(), desired(CONFIG)["demo"]);
    let emptied = without(&without(CONFIG, "demo"), "web");
    assert_eq!(emptied.matches("# External MCP servers").count(), 1);
    let restored = append_raw(Some(&emptied), &texts(&raw)).unwrap();
    assert_eq!(restored, CONFIG);
}

#[test]
fn a_server_put_back_uses_its_recorded_table() {
    let moved = without(CONFIG, "demo");
    let back = sync(Some(&moved), &desired(CONFIG), &recorded(CONFIG)).unwrap();
    assert_eq!(back, CONFIG);
    let appended = append_raw(Some(&moved), &texts(&raw_entries(CONFIG).unwrap())).unwrap();
    assert_eq!(appended, CONFIG);
}

#[test]
fn a_table_with_subtables_goes_back_in_one_piece() {
    let text = "default_model = \"x\"\n\n[[plugins]]\nname = \"a\"\ncommand = \"a\"\n\n[plugins.env]\nA = \"1\"\n\n[[plugins]]\nname = \"b\"\ncommand = \"b\"\n\n[plugins.env]\nB = \"2\"\n\n[skills]\ndisabled_skills = []\n";
    let raw = raw_entries(text).unwrap();
    let moved = without(text, "a");
    assert_eq!(
        sync(Some(&moved), &desired(text), &recorded(text)).unwrap(),
        text
    );
    assert_eq!(append_raw(Some(&moved), &texts(&raw[..1])).unwrap(), text);
    // The last plugin goes back exactly where it was.
    let moved = without(text, "b");
    assert_eq!(append_raw(Some(&moved), &texts(&raw[1..])).unwrap(), text);
    // Into a file without plugins, at the end.
    let empty = "[skills]\ndisabled_skills = []\n";
    let into = append_raw(Some(empty), &texts(&raw)).unwrap();
    assert_eq!(desired(&into), desired(text));
    assert_eq!(
        normalized(&into).unwrap()["rest"],
        normalized(empty).unwrap()["rest"]
    );
    assert_eq!(into.parse::<DocumentMut>().unwrap().to_string(), into);
}

#[test]
fn a_put_back_table_next_to_a_new_one_stays_after_the_settings() {
    // Every plugin goes, one server is new and one comes back from its record.
    let mut want = serde_json::Map::new();
    want.insert("demo".into(), desired(CONFIG)["demo"].clone());
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    let raw = recorded(CONFIG);
    let out = sync(Some(&without(CONFIG, "web")), &want, &raw).unwrap();
    let out = sync(Some(&out), &want, &raw).unwrap();
    let emptied = without(&without(CONFIG, "demo"), "web");
    let both = sync(Some(&emptied), &want, &raw).unwrap();
    for text in [&out, &both] {
        let skills = text.find("[skills]").unwrap();
        let demo = text.find("name    = \"demo\"").unwrap();
        let gateway = text.find("name = \"toolport\"").unwrap();
        assert!(skills < demo && skills < gateway, "{text}");
    }
    assert_eq!(desired(&both), want);
}

#[test]
fn duplicates_use_last_command_and_merge_policy_without_blocking_connect() {
    let text = "[[plugins]]\nname = 'dup'\ncommand = '/first/server'\ndisabled_tools = ['a', 'b']\n[[plugins]]\nname = 'dup'\ncommand = '/last/server'\ndisabled_tools = ['b', 'c']\n";
    assert_eq!(
        servers(text).unwrap()[0].command.as_deref(),
        Some("/last/server")
    );
    assert_eq!(
        desired(text)["dup"]["disabled_tools"],
        serde_json::json!(["a", "b", "c"])
    );
    let connected = with(text, &stdio("toolport"));
    assert!(connected.starts_with(text), "{connected}");
    assert_eq!(without(&connected, "toolport"), text);
    let raw = raw_entries(text).unwrap();
    assert_eq!(raw[0].1.matches("[[plugins]]").count(), 2);
    assert_eq!(raw[0].2, desired(text)["dup"]);
    assert_eq!(definition(&raw[0].1).unwrap(), raw[0].2);
    // Only editing or moving the duplicated name needs manual resolution.
    let mut want = desired(text);
    want.remove("dup");
    assert!(sync(Some(text), &want, &Default::default())
        .unwrap_err()
        .contains("duplicate"));
    let mut want = desired(text);
    want["dup"]["command"] = "changed".into();
    assert!(sync(Some(text), &want, &Default::default()).is_err());
    let identical = "[[plugins]]\nname = 'same'\ncommand = 'same'\n[[plugins]]\nname = 'same'\ncommand = 'same'\n";
    assert!(with(identical, &stdio("toolport")).starts_with(identical));
}

#[test]
fn raw_restore_keeps_neighbors_comments_and_unrelated_edits() {
    let text = "[ui]\ntheme = 'dark'\n\n# server A\n[[plugins]]\nname = 'a'\ncommand = 'a'\n\n# skill policy\n[skills]\ndisabled_skills = []\n";
    let raw = raw_entries(text).unwrap();
    let moved = without(text, "a").replace("'dark'", "'light'");
    let restored = append_raw(Some(&moved), &texts(&raw)).unwrap();
    assert_eq!(restored, text.replace("'dark'", "'light'"));
}

#[test]
fn connect_and_fallback_disconnect_keep_crlf_and_mixed_endings() {
    for text in [
        "[ui]\r\ntheme = 'dark'\r\n",
        "[ui]\r\ntheme = 'dark'\r\n\n[skills]\ndisabled_skills = []\n",
    ] {
        let connected = with(text, &stdio("toolport"));
        assert!(connected.starts_with(text), "{connected:?}");
        let edited = connected.replace("'dark'", "'light'");
        assert_eq!(
            without(&edited, "toolport"),
            text.replace("'dark'", "'light'")
        );
    }
}

#[test]
fn inline_or_odd_plugin_lists_are_refused() {
    let inline = "plugins = [{ name = \"a\", command = \"a\" }]\n";
    let mut want = serde_json::Map::new();
    want.insert("toolport".into(), config_of(&stdio("toolport")).unwrap());
    assert!(sync(Some(inline), &want, &Default::default())
        .unwrap_err()
        .contains("edit it by hand"));
    let odd = "plugins = \"x\"\n";
    assert!(normalized(odd).unwrap_err().contains("[[plugins]] tables"));
    let broken = "[[plugins]\n";
    assert!(normalized(broken).unwrap_err().contains("Could not parse"));
}

#[test]
fn remote_servers_use_type_url_and_headers() {
    for (transport, kind) in [("http", "http"), ("sse", "sse")] {
        let entry: ServerEntry = serde_json::from_value(serde_json::json!({
            "name": "remote", "transport": transport, "url": "https://example.test/mcp",
            "env": [{"key": "Authorization", "value": "Bearer x"}],
        }))
        .unwrap();
        let config = config_of(&entry).unwrap();
        assert_eq!(config["type"], kind);
        assert_eq!(config["headers"]["Authorization"], "Bearer x");
        assert!(config.get("env").is_none());
    }
}

#[test]
fn removing_a_plugin_does_not_take_its_neighbors_line_endings() {
    let text = "[[plugins]]\r\nname = 'a'\r\ncommand = 'same'\r\n\n[[plugins]]\nname = 'b'\ncommand = 'same'\n";
    let out = without(text, "a");
    assert!(
        out.ends_with("[[plugins]]\nname = 'b'\ncommand = 'same'\n"),
        "{out:?}"
    );
}

#[test]
fn crlf_raw_restore_retains_comments_and_section_order() {
    let text = "[ui]\r\ntheme = 'dark'\r\n\r\n# server A\r\n[[plugins]]\r\nname = 'a'\r\ncommand = 'a'\r\n\r\n[skills]\r\ndisabled_skills = []\r\n";
    let raw = raw_entries(text).unwrap();
    let moved = without(text, "a").replace("'dark'", "'light'");
    assert_eq!(
        append_raw(Some(&moved), &texts(&raw)).unwrap(),
        text.replace("'dark'", "'light'")
    );
}

#[test]
fn mixed_ending_raw_plugin_restores_its_own_newlines() {
    let text = "[ui]\r\ntheme = 'dark'\r\n\n[[plugins]]\nname = 'a'\ncommand = 'a'\n\r\n[skills]\r\ndisabled_skills = []\r\n";
    let raw = raw_entries(text).unwrap();
    let moved = without(text, "a").replace("'dark'", "'light'");
    assert_eq!(
        append_raw(Some(&moved), &texts(&raw)).unwrap(),
        text.replace("'dark'", "'light'")
    );
}

#[test]
fn raw_restore_uses_nested_and_named_array_neighbors() {
    for text in [
        "[ui]\ntheme = 'dark'\n[ui.colors]\naccent = 'blue'\n\n[[plugins]]\nname = 'a'\ncommand = 'a'\n",
        "[[plugins]]\nname = 'a'\ncommand = 'a'\n\n[[providers]]\nname = 'local'\nurl = 'http://localhost'\n",
    ] {
        let moved = without(text, "a");
        let raw = raw_entries(text).unwrap();
        assert_eq!(append_raw(Some(&moved), &texts(&raw)).unwrap(), text);
    }
}
