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

/// `CONFIG` with the demo table moved after the web one.
const DEMO_LAST: &str = r#"# Reasonix configuration.
config_version = 12   # schema marker for diagnostics
default_model = "deepseek-flash"

[ui]
theme = "auto"   # auto|dark|light

[skills]
# disabled_skills = ["review"]   # hide noisy or unwanted skills

# External MCP servers. type: "stdio" (default, a subprocess) | "http" | "sse".

[[plugins]]
name    = "web"
type    = "http"
url     = "https://example.test/mcp"
headers = { Authorization = "Bearer ${WEB_TOKEN}" }

[[plugins]]
name    = "demo"
command = "python3"   # keep this note
args    = ["/opt/demo/server.py"]
"#;

#[test]
fn tables_move_out_and_back_with_their_comments() {
    let raw = raw_entries(CONFIG).unwrap();
    assert_eq!(raw.len(), 2);
    assert!(raw[0].1.contains("# keep this note"), "{}", raw[0].1);
    // The comment above the table stays in the file, so the record leaves it out.
    assert!(!raw[0].1.contains("External MCP servers"), "{}", raw[0].1);
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
    assert_eq!(back, DEMO_LAST);
    let appended = append_raw(Some(&moved), &texts(&raw_entries(CONFIG).unwrap())).unwrap();
    assert_eq!(appended, DEMO_LAST);
}

#[test]
fn a_table_with_subtables_goes_back_in_one_piece() {
    let text = "default_model = \"x\"\n\n[[plugins]]\nname = \"a\"\ncommand = \"a\"\n\n[plugins.env]\nA = \"1\"\n\n[[plugins]]\nname = \"b\"\ncommand = \"b\"\n\n[plugins.env]\nB = \"2\"\n\n[skills]\ndisabled_skills = []\n";
    let a_last = "default_model = \"x\"\n\n[[plugins]]\nname = \"b\"\ncommand = \"b\"\n\n[plugins.env]\nB = \"2\"\n\n[[plugins]]\nname = \"a\"\ncommand = \"a\"\n\n[plugins.env]\nA = \"1\"\n\n[skills]\ndisabled_skills = []\n";
    let raw = raw_entries(text).unwrap();
    let moved = without(text, "a");
    assert_eq!(
        sync(Some(&moved), &desired(text), &recorded(text)).unwrap(),
        a_last
    );
    assert_eq!(append_raw(Some(&moved), &texts(&raw[..1])).unwrap(), a_last);
    // The last plugin goes back exactly where it was.
    let moved = without(text, "b");
    assert_eq!(append_raw(Some(&moved), &texts(&raw[1..])).unwrap(), text);
    // Into a file without plugins, at the end.
    let empty = "[skills]\ndisabled_skills = []\n";
    let into = append_raw(Some(empty), &texts(&raw)).unwrap();
    assert_eq!(desired(&into), desired(text));
    assert!(into.starts_with(empty), "{into}");
    assert_eq!(into.parse::<DocumentMut>().unwrap().to_string(), into);
}

#[test]
fn a_shadowed_table_moves_with_the_one_reasonix_uses() {
    let text = format!("{CONFIG}\n[[plugins]]\nname = \"demo\"\ncommand = \"other\"\n");
    let raw = raw_entries(&text).unwrap();
    assert_eq!(raw.len(), 2);
    assert!(raw[0].1.contains("command = \"other\""), "{}", raw[0].1);
    assert_eq!(raw[0].2, desired(&text)["demo"]);
    let emptied = without(&without(&text, "demo"), "web");
    let back = append_raw(Some(&emptied), &texts(&raw)).unwrap();
    assert!(back.contains("command = \"other\""), "{back}");
    assert_eq!(desired(&back), desired(&text));
    let moved = without(&text, "demo");
    let synced = sync(Some(&moved), &desired(&text), &recorded(&text)).unwrap();
    assert_eq!(synced.matches("name = \"demo\"").count(), 1, "{synced}");
    assert!(synced.contains("command = \"other\""), "{synced}");
    assert_eq!(desired(&synced), desired(&text));
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
