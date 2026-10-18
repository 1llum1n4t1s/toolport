//! OpenClaw keeps MCP servers under `mcp.servers` in its gateway config,
//! `openclaw.json`, which it reads as JSON5 and which also holds the user's
//! channels, agents and models.
//!
//! Each server is `command`/`args`/`env`/`cwd` (stdio) or `url`/`headers` with a
//! `transport` of `streamable-http` or `sse`. OpenClaw reads only `transport`, and
//! a URL without one is SSE, so Toolport always writes it. A server with
//! `enabled: false` is kept but not started. Edits patch the `mcp` object in the
//! existing text, so comments, unquoted keys and every other setting stay. A
//! config that pulls `mcp` in with `$include` is refused: OpenClaw itself won't
//! write through those, and an entry written next to the include would override
//! it.

use super::*;
use serde_json::{Map, Value};

const CLIENT: &str = "OpenClaw";

/// Fields OpenClaw honors that a Toolport server can't carry, so importing a
/// server that sets one would change how it runs.
const NOT_IMPORTED: &[&str] = &[
    "cwd",
    "workingDirectory",
    "connectionTimeoutMs",
    "requestTimeoutMs",
    "supportsParallelToolCalls",
    "auth",
    "oauth",
    "sslVerify",
    "clientCert",
    "clientKey",
    "toolFilter",
    "codex",
];

/// Parse a config and refuse what Toolport can't edit safely: a root that isn't
/// an object, repeated keys and `$include`. The mutation guard runs this before
/// every write, Disconnect and restores included.
pub(super) fn root_from_content(content: &str) -> Result<Value, String> {
    let root = read_existing_json(content, true)?;
    if !root.is_object() {
        return Err("The OpenClaw config root must be an object; leaving it untouched.".into());
    }
    if !content.trim().is_empty() {
        let cst =
            jsonc_parser::cst::CstRootNode::parse(content, &jsonc_parser::ParseOptions::default())
                .map_err(|error| format!("Could not parse the OpenClaw config: {error}"))?;
        if let Some(object) = cst.object_value() {
            zcode::duplicate_keys(&object, "", CLIENT)?;
        }
    }
    refuse_includes(&root)?;
    Ok(root)
}

/// `$include` at the root, in `mcp`, in `mcp.servers` or in a server.
fn refuse_includes(root: &Value) -> Result<(), String> {
    let mcp = root.get("mcp");
    let servers = mcp.and_then(|mcp| mcp.get("servers"));
    let mut places = vec![(root, ""), (mcp.unwrap_or(&Value::Null), "mcp")];
    places.push((servers.unwrap_or(&Value::Null), "mcp.servers"));
    if let Some(Value::Object(map)) = servers {
        places.extend(map.values().map(|server| (server, "mcp.servers")));
    }
    match places
        .into_iter()
        .find(|(value, _)| value.get("$include").is_some())
    {
        Some((_, place)) => Err(format!(
            "This OpenClaw config uses $include{} for its MCP servers, which Toolport can't edit safely. Add the server with `openclaw mcp add` or edit the included file by hand.",
            if place.is_empty() {
                String::new()
            } else {
                format!(" in {place}")
            }
        )),
        None => Ok(()),
    }
}

fn read_root(path: &Path) -> Result<(Value, Option<String>), String> {
    if let Some(staged) = mutation::read(path) {
        return match staged {
            Some(content) => Ok((root_from_content(&content)?, Some(content))),
            None => Ok((serde_json::json!({}), None)),
        };
    }
    match std::fs::metadata(path) {
        Ok(_) => {
            let content = read_config_file(path)?;
            Ok((root_from_content(&content)?, Some(content)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((serde_json::json!({}), None))
        }
        Err(error) => Err(format!("Could not read the OpenClaw config: {error}")),
    }
}

fn server_map(root: &Value) -> Result<Map<String, Value>, String> {
    let Some(mcp) = root.get("mcp") else {
        return Ok(Map::new());
    };
    let mcp = mcp.as_object().ok_or("OpenClaw 'mcp' must be an object")?;
    match mcp.get("servers") {
        None => Ok(Map::new()),
        Some(servers) => servers
            .as_object()
            .cloned()
            .ok_or_else(|| "OpenClaw 'mcp.servers' must be an object".into()),
    }
}

fn disabled(definition: &Value) -> bool {
    definition.get("enabled") == Some(&Value::Bool(false))
}

/// The transport OpenClaw uses for a server: a command means stdio, otherwise
/// `transport` (or a CLI-style `type` its config migration turns into one),
/// SSE when neither is set.
fn transport(definition: &Value) -> Option<&'static str> {
    let text = |key: &str| {
        definition
            .get(key)
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase())
    };
    if text("command").is_some_and(|command| !command.is_empty()) {
        return Some("stdio");
    }
    text("url").filter(|url| !url.is_empty())?;
    let kind = text("transport").or_else(|| text("type"));
    match kind.as_deref() {
        None | Some("sse") => Some("sse"),
        Some("streamable-http" | "http") => Some("http"),
        Some(_) => None,
    }
}

/// The definition in the shape the shared parsers read, with `type` set.
fn normalized_server(name: &str, definition: &Value) -> Result<Value, String> {
    let mut value = definition
        .as_object()
        .cloned()
        .ok_or_else(|| format!("Malformed OpenClaw server '{name}': expected an object"))?;
    let kind = transport(definition).ok_or_else(|| {
        format!("Malformed OpenClaw server '{name}': it needs a command or a URL with a supported transport")
    })?;
    value.remove("transport");
    value.insert("type".into(), Value::String(kind.into()));
    Ok(Value::Object(value))
}

/// The servers OpenClaw starts, plus a Toolport entry the user turned off, so
/// it still counts as connected and Disconnect removes it.
fn inventory(servers: &Map<String, Value>) -> Result<Vec<McpServer>, String> {
    let mut inventory = servers
        .iter()
        .filter(|(name, definition)| !disabled(definition) || is_gateway(name, definition))
        .map(|(name, definition)| {
            normalized_server(name, definition).map(|value| json_server(name, &value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    inventory.sort_by_key(|server| server.name.to_lowercase());
    Ok(inventory)
}

pub(super) fn parse(content: &str) -> Result<Vec<McpServer>, String> {
    inventory(&server_map(&root_from_content(content)?)?)
}

fn is_gateway(name: &str, definition: &Value) -> bool {
    gateway_edit_matches(name, definition.get("command").and_then(Value::as_str))
}

fn has_include(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("$include") || map.values().any(has_include),
        Value::Array(list) => list.iter().any(has_include),
        _ => false,
    }
}

fn importable(name: &str, definition: &Value, reject_disabled: bool) -> Result<(), String> {
    normalized_server(name, definition)?;
    if has_include(definition) {
        return Err(format!(
            "Cannot import OpenClaw server '{name}': part of it comes from another file through $include. Keep this server in OpenClaw."
        ));
    }
    if let Some(field) = NOT_IMPORTED
        .iter()
        .find(|field| definition.get(**field).is_some())
    {
        return Err(format!(
            "Cannot import OpenClaw server '{name}': Toolport can't keep its '{field}' setting. Keep this server in OpenClaw, or set up the same behavior in Toolport by hand first."
        ));
    }
    if reject_disabled && disabled(definition) {
        return Err(format!(
            "Cannot move turned-off OpenClaw server '{name}': this path can't keep it turned off. Turn it on in OpenClaw first, or keep it there."
        ));
    }
    Ok(())
}

pub(super) fn validate_import(
    path: &Path,
    names: &[String],
    migration: bool,
) -> Result<(), String> {
    let (root, _) = read_root(path)?;
    let servers = server_map(&root)?;
    for name in names {
        let definition = servers.get(name).ok_or_else(|| {
            format!("OpenClaw server '{name}' changed since detection; refresh the client list and retry.")
        })?;
        importable(name, definition, migration)?;
    }
    Ok(())
}

/// A Toolport server as an OpenClaw entry: the shared JSON shape, with
/// `transport` in place of the `type` hint for a URL and `cwd` for a command.
pub(super) fn entry_value(entry: &ServerEntry) -> Result<Value, String> {
    if entry.command.is_none() && entry.url.is_none() {
        return Err(format!(
            "'{}' has no command or URL; leaving the OpenClaw config untouched.",
            entry.name
        ));
    }
    let mut value = entry_to_json(entry);
    let map = value
        .as_object_mut()
        .expect("entry_to_json builds an object");
    if entry.command.is_some() {
        if let Some(cwd) = &entry.cwd {
            map.insert("cwd".into(), Value::String(cwd.clone()));
        }
    } else {
        map.remove("type");
        let kind = if entry.transport.eq_ignore_ascii_case("sse") {
            "sse"
        } else {
            "streamable-http"
        };
        map.insert("transport".into(), Value::String(kind.into()));
    }
    Ok(value)
}

/// OpenClaw settings a user may add to the Toolport entry, kept when Toolport
/// rewrites it.
const GATEWAY_SETTINGS: &[&str] = &[
    "connectionTimeoutMs",
    "requestTimeoutMs",
    "supportsParallelToolCalls",
    "toolFilter",
    "codex",
];

/// A gateway the user turned off stays off when Toolport repairs it, and keeps
/// the OpenClaw settings they gave it.
fn retain_gateway_state(servers: &Map<String, Value>, value: &mut Value) {
    for (name, definition) in servers {
        if !is_gateway(name, definition) {
            continue;
        }
        if disabled(definition) {
            value["enabled"] = Value::Bool(false);
        }
        for key in GATEWAY_SETTINGS {
            if let (Some(setting), Some(map)) = (definition.get(*key), value.as_object_mut()) {
                map.entry(*key).or_insert_with(|| setting.clone());
            }
        }
    }
}

fn write_map(
    path: &Path,
    mut root: Value,
    original: Option<&str>,
    servers: Map<String, Value>,
) -> Result<(), String> {
    if root.get("mcp").is_none() {
        root["mcp"] = serde_json::json!({});
    }
    root["mcp"]["servers"] = Value::Object(servers);
    atomic_write_json_config(path, original, &root, "mcp")
}

pub(super) fn edit_gateway(path: &Path, entry: Option<&ServerEntry>) -> Result<(), String> {
    let (root, original) = read_root(path)?;
    let mut servers = server_map(&root)?;
    let mut value = entry.map(entry_value).transpose()?;
    if let Some(value) = &mut value {
        retain_gateway_state(&servers, value);
    }
    let count = servers.len();
    servers.retain(|name, definition| {
        !gateway_edit_matches(name, definition.get("command").and_then(Value::as_str))
    });
    if let Some(value) = value {
        servers.insert(GATEWAY_ENTRY_NAME.into(), value);
    } else if servers.len() == count {
        return Ok(());
    }
    write_map(path, root, original.as_deref(), servers)
}

pub(super) fn write_servers(path: &Path, entries: &[ServerEntry]) -> Result<(), String> {
    let (root, original) = read_root(path)?;
    let previous = server_map(&root)?;
    // Turned-off servers aren't listed, so they can't be in `entries`; keep them.
    let mut servers: Map<String, Value> = previous
        .iter()
        .filter(|(name, definition)| {
            disabled(definition)
                && !entries.iter().any(|entry| &entry.name == *name)
                && !is_gateway(name, definition)
        })
        .map(|(name, definition)| (name.clone(), definition.clone()))
        .collect();
    for entry in entries {
        let mut value = entry_value(entry)?;
        if is_gateway_server(entry) {
            retain_gateway_state(&previous, &mut value);
        }
        servers.insert(entry.name.clone(), value);
    }
    write_map(path, root, original.as_deref(), servers)
}

/// Where OpenClaw keeps its files, resolved as it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dirs {
    /// `OPENCLAW_HOME`, else the user's home.
    pub home: PathBuf,
    /// `OPENCLAW_STATE_DIR`, else `<home>/.openclaw`, or the legacy
    /// `<home>/.clawdbot` when only that one exists.
    pub state: PathBuf,
    /// `OPENCLAW_CONFIG_PATH`, else `<state>/openclaw.json`.
    pub config: PathBuf,
    /// The default agent's workspace: `OPENCLAW_WORKSPACE_DIR`, else
    /// `agents.defaults.workspace` in the config, else `<state>/workspace`.
    pub workspace: PathBuf,
    /// Whether the state dir is the default one; OpenClaw reads personal
    /// `~/.agents/skills` only then.
    pub default_state: bool,
    /// A path variable is relative. OpenClaw resolves it against its own
    /// working directory, which Toolport can't know, so the config is unknown.
    pub relative: bool,
}

/// `env` returns a variable trimmed, or `None` when it is unset or blank.
pub(crate) fn dirs(user_home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Dirs {
    let expand_from = |base: &Path, raw: &str| -> Option<PathBuf> {
        let rest = if raw == "~" {
            Some("")
        } else {
            raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\"))
        };
        match rest {
            Some(rest) => Some(base.join(rest.trim_start_matches(['/', '\\']))),
            None => {
                let path = PathBuf::from(raw);
                path.is_absolute().then_some(path)
            }
        }
    };
    let relative = [
        "OPENCLAW_HOME",
        "OPENCLAW_STATE_DIR",
        "OPENCLAW_CONFIG_PATH",
        "OPENCLAW_WORKSPACE_DIR",
    ]
    .iter()
    .filter_map(|key| env(key))
    .any(|raw| expand_from(user_home, &raw).is_none());
    let home = env("OPENCLAW_HOME")
        .and_then(|raw| expand_from(user_home, &raw))
        .unwrap_or_else(|| user_home.to_path_buf());
    let expand = |raw: String| expand_from(&home, &raw);
    let primary = home.join(".openclaw");
    let (state, default_state) = match env("OPENCLAW_STATE_DIR").and_then(expand) {
        Some(dir) => {
            let default = dir == primary;
            (dir, default)
        }
        None => {
            let legacy = home.join(".clawdbot");
            let dir = if !primary.exists() && legacy.exists() {
                legacy
            } else {
                primary
            };
            (dir, true)
        }
    };
    let config = env("OPENCLAW_CONFIG_PATH")
        .and_then(expand)
        .unwrap_or_else(|| state.join("openclaw.json"));
    let configured = || {
        let text = std::fs::read_to_string(&config).ok()?;
        let root = parse_json_value(&text).ok()?;
        let raw = root
            .get("agents")?
            .get("defaults")?
            .get("workspace")?
            .as_str()?
            .trim()
            .to_string();
        expand(raw)
    };
    let workspace = env("OPENCLAW_WORKSPACE_DIR")
        .and_then(expand)
        .or_else(configured)
        .unwrap_or_else(|| state.join("workspace"));
    Dirs {
        home,
        state,
        config,
        workspace,
        default_state,
        relative,
    }
}

#[cfg(test)]
#[path = "openclaw_tests.rs"]
mod tests;
