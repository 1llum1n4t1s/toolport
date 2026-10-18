//! Reasonix keeps MCP servers as `[[plugins]]` tables in its global
//! `config.toml` (`<Reasonix home>/config.toml`).
//!
//! Each table names its server with `name` and uses Claude Code's fields:
//! `type` (`stdio` by default, `http` or `sse`), `command`/`args`/`env` or
//! `url`/`headers`, plus Reasonix's own policy keys. The same file holds the
//! user's whole Reasonix configuration, mostly comments in the template Reasonix
//! writes, so edits go through `toml_edit`: a table Toolport changes keeps its
//! place, a new one is appended, and everything else stays as it was. Every edit
//! is checked by reading the result back.
//!
//! The rest of the client code sees the file through [`normalized`]:
//! `{"mcpServers": {name: table}, "rest": {...}}`.

use serde_json::Value as Json;
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use super::{McpServer, ServerEntry};

pub(super) const KEY: &str = "mcpServers";

const UNTOUCHED: &str = "leaving the Reasonix config untouched";

fn document(text: &str) -> Result<DocumentMut, String> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|e| format!("Could not parse the Reasonix config ({e}); {UNTOUCHED}."))
}

/// The `[[plugins]]` tables, or `None` when the key holds something else.
fn plugins(doc: &DocumentMut) -> Result<Option<&ArrayOfTables>, String> {
    match doc.get("plugins") {
        None => Ok(None),
        Some(Item::ArrayOfTables(list)) => Ok(Some(list)),
        Some(Item::Value(Value::Array(list))) if list.is_empty() => Ok(None),
        Some(Item::Value(Value::Array(_))) => Err(format!(
            "This Reasonix config lists plugins inline; edit it by hand, {UNTOUCHED}."
        )),
        Some(_) => Err(format!(
            "'plugins' in the Reasonix config must be a list of [[plugins]] tables; {UNTOUCHED}."
        )),
    }
}

fn name_of(table: &Table) -> Option<&str> {
    table.get("name").and_then(Item::as_str)
}

fn to_json(table: &Table) -> Json {
    let mut doc = DocumentMut::new();
    for (key, item) in table.iter() {
        doc.insert(key, item.clone());
    }
    toml::from_str::<toml::Value>(&doc.to_string())
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Json::Null)
}

/// A table's fields other than `name`, as the shared code compares them.
fn config_json(table: &Table) -> Json {
    let mut value = to_json(table);
    if let Some(map) = value.as_object_mut() {
        map.remove("name");
    }
    value
}

/// Everything in the file except the plugin tables.
fn rest(text: &str) -> Result<Json, String> {
    if text.trim().is_empty() {
        return Ok(Json::Object(serde_json::Map::new()));
    }
    let mut value: toml::Value = toml::from_str(text)
        .map_err(|e| format!("Could not parse the Reasonix config ({e}); {UNTOUCHED}."))?;
    if let Some(table) = value.as_table_mut() {
        table.remove("plugins");
    }
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn servers_map(doc: &DocumentMut) -> Result<serde_json::Map<String, Json>, String> {
    let mut out = serde_json::Map::new();
    for table in plugins(doc)?.into_iter().flat_map(ArrayOfTables::iter) {
        if let Some(name) = name_of(table) {
            // Reasonix keeps the first table of a name.
            out.entry(name.to_string())
                .or_insert_with(|| config_json(table));
        }
    }
    Ok(out)
}

/// `{"mcpServers": {name: fields}, "rest": everything else}`.
pub(super) fn normalized(text: &str) -> Result<Json, String> {
    let doc = document(text)?;
    let servers = servers_map(&doc)?;
    let mut root = serde_json::Map::new();
    if !servers.is_empty() {
        root.insert(KEY.into(), Json::Object(servers));
    }
    let rest = rest(text)?;
    if rest.as_object().is_some_and(|map| !map.is_empty()) {
        root.insert("rest".into(), rest);
    }
    Ok(Json::Object(root))
}

fn strings(value: Option<&Json>) -> Vec<String> {
    value
        .and_then(Json::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| match item {
                    Json::String(s) => Some(s.clone()),
                    Json::Number(n) => Some(n.to_string()),
                    Json::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The servers in the file, for import and the client list.
pub(super) fn servers(text: &str) -> Result<Vec<McpServer>, String> {
    let doc = document(text)?;
    Ok(servers_map(&doc)?
        .into_iter()
        .map(|(name, config)| {
            let text = |key: &str| config.get(key).and_then(Json::as_str).map(str::to_string);
            let transport = match text("type").as_deref() {
                None | Some("stdio") | Some("") => "stdio",
                Some("http") | Some("streamable-http") => "http",
                Some("sse") => "sse",
                Some(_) => "unknown",
            };
            let mut env_keys: Vec<String> = ["env", "headers"]
                .iter()
                .filter_map(|key| config.get(*key).and_then(Json::as_object))
                .flat_map(|map| map.keys().cloned())
                .collect();
            env_keys.sort_unstable();
            env_keys.dedup();
            McpServer {
                name,
                transport: transport.into(),
                command: text("command"),
                args: strings(config.get("args")),
                env_keys,
                url: text("url"),
            }
        })
        .collect())
}

/// A Toolport server as a Reasonix plugin table's fields (without `name`).
pub(super) fn config_of(entry: &ServerEntry) -> Result<Json, String> {
    if entry.name.trim().is_empty() {
        return Err(format!("A Reasonix plugin needs a name; {UNTOUCHED}."));
    }
    let values: serde_json::Map<String, Json> = entry
        .env
        .iter()
        .filter_map(|e| {
            e.value
                .as_ref()
                .map(|v| (e.key.clone(), Json::String(v.clone())))
        })
        .collect();
    let mut config = serde_json::Map::new();
    if let Some(command) = &entry.command {
        config.insert("command".into(), Json::String(command.clone()));
        if !entry.args.is_empty() {
            config.insert(
                "args".into(),
                Json::Array(entry.args.iter().cloned().map(Json::String).collect()),
            );
        }
        if !values.is_empty() {
            config.insert("env".into(), Json::Object(values));
        }
    } else if let Some(url) = &entry.url {
        let kind = if entry.transport.eq_ignore_ascii_case("sse") {
            "sse"
        } else {
            "http"
        };
        config.insert("type".into(), kind.into());
        config.insert("url".into(), Json::String(url.clone()));
        if !values.is_empty() {
            config.insert("headers".into(), Json::Object(values));
        }
    } else {
        return Err(format!(
            "'{}' has no command or URL; {UNTOUCHED}.",
            entry.name
        ));
    }
    Ok(Json::Object(config))
}

fn edit_value(value: &Json) -> Option<Value> {
    Some(match value {
        Json::String(s) => Value::from(s.as_str()),
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::from(i),
            None => Value::from(n.as_f64()?),
        },
        Json::Array(list) => {
            let mut array = toml_edit::Array::new();
            for item in list {
                array.push(edit_value(item)?);
            }
            Value::Array(array)
        }
        Json::Object(map) => {
            let mut inline = InlineTable::new();
            for (key, item) in map {
                inline.insert(key, edit_value(item)?);
            }
            Value::InlineTable(inline)
        }
        Json::Null => return None,
    })
}

/// The documented keys first, the way Reasonix's own `mcp add` writes them.
const ORDER: [&str; 7] = ["type", "command", "args", "env", "url", "headers", "load"];

fn new_table(name: &str, config: &Json) -> Result<Table, String> {
    let mut table = Table::new();
    table.insert("name", Item::Value(Value::from(name)));
    fill(&mut table, config)?;
    Ok(table)
}

/// Set `table`'s fields to `config`, keeping the place and comments of keys that
/// stay. `name` is never touched.
fn fill(table: &mut Table, config: &Json) -> Result<(), String> {
    let map = config
        .as_object()
        .ok_or_else(|| format!("A Reasonix plugin must be a table; {UNTOUCHED}."))?;
    let stale: Vec<String> = table
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| key != "name" && !map.contains_key(key))
        .collect();
    for key in stale {
        table.remove(&key);
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|key| ORDER.iter().position(|k| k == key).unwrap_or(ORDER.len()));
    for key in keys {
        if key == "name" {
            continue;
        }
        let value = &map[key];
        let current = table.get(key).map(|item| {
            let mut one = Table::new();
            one.insert(key, item.clone());
            config_json(&one).get(key).cloned()
        });
        if current.flatten().as_ref() == Some(value) {
            continue;
        }
        let item = edit_value(value)
            .ok_or_else(|| format!("Reasonix can't store a null '{key}'; {UNTOUCHED}."))?;
        table.insert(key, Item::Value(item));
    }
    Ok(())
}

/// Make the file's plugins exactly `desired` (name -> fields). Tables already
/// matching stay byte for byte; a changed one is edited in place.
pub(super) fn sync(
    text: Option<&str>,
    desired: &serde_json::Map<String, Json>,
    raw: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    let text = text.unwrap_or("");
    let mut doc = document(text)?;
    let before_rest = rest(text)?;
    plugins(&doc)?;
    if servers_map(&doc)? == *desired {
        return Ok(text.to_string());
    }
    if !matches!(doc.get("plugins"), Some(Item::ArrayOfTables(_))) {
        doc.remove("plugins");
        doc.insert("plugins", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let list = doc["plugins"]
        .as_array_of_tables_mut()
        .expect("set to an array of tables above");
    let mut seen = std::collections::BTreeSet::new();
    let mut drop = Vec::new();
    for (i, table) in list.iter_mut().enumerate() {
        let Some(name) = name_of(table).map(str::to_string) else {
            continue;
        };
        match desired.get(&name) {
            // Reasonix ignores a later table with a taken name; so does Toolport.
            Some(_) if !seen.insert(name.clone()) => {}
            Some(want) => {
                if config_json(table) != *want {
                    fill(table, want)?;
                }
            }
            None => drop.push(i),
        }
    }
    // Comment lines above a removed table's header belong to whatever came
    // before it (often the end of another section), so they stay.
    let mut notes = Vec::new();
    for i in drop.into_iter().rev() {
        let table = list.get(i).expect("index from this list");
        let note = table
            .decor()
            .prefix()
            .and_then(|prefix| prefix.as_str())
            .filter(|prefix| prefix.contains('#'))
            .map(|prefix| prefix.trim_end_matches(['\n', '\r', ' ', '\t']).to_string() + "\n");
        if let (Some(note), Some(at)) = (note, table.position()) {
            notes.push((at, note));
        }
        list.remove(i);
    }
    for (name, config) in desired {
        if seen.contains(name) {
            continue;
        }
        let recorded = raw
            .get(name)
            .filter(|text| definition(text).ok().as_ref() == Some(config))
            .and_then(|text| document(text).ok())
            .and_then(|one| plugins(&one).ok().flatten().and_then(|l| l.get(0)).cloned());
        match recorded {
            Some(table) => list.push(table),
            None => list.push(new_table(name, config)?),
        }
    }
    if list.is_empty() {
        doc.remove("plugins");
    }
    // Latest first, so notes from adjacent removed tables keep their order.
    notes.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (at, note) in notes {
        let mut next = None;
        next_position(doc.as_table(), at, &mut next);
        match next.and_then(|position| table_at(doc.as_table_mut(), position)) {
            Some(table) => {
                let own = table
                    .decor()
                    .prefix()
                    .and_then(|prefix| prefix.as_str())
                    .unwrap_or("")
                    .to_string();
                table.decor_mut().set_prefix(note + &own);
            }
            None => {
                let trailing = doc.trailing().as_str().unwrap_or("").to_string();
                doc.set_trailing(note + &trailing);
            }
        }
    }
    let out = doc.to_string();
    verify(&out, &before_rest, desired)?;
    Ok(out)
}

/// The smallest table header position in the document after `after`.
fn next_position(table: &Table, after: usize, best: &mut Option<usize>) {
    let consider = |t: &Table, best: &mut Option<usize>| {
        if let Some(at) = t.position().filter(|&at| at > after) {
            if best.is_none_or(|b| at < b) {
                *best = Some(at);
            }
        }
        next_position(t, after, best);
    };
    for (_, item) in table.iter() {
        match item {
            Item::Table(t) => consider(t, best),
            Item::ArrayOfTables(list) => list.iter().for_each(|t| consider(t, best)),
            _ => {}
        }
    }
}

fn table_at(table: &mut Table, position: usize) -> Option<&mut Table> {
    for (_, item) in table.iter_mut() {
        let tables: Vec<&mut Table> = match item {
            Item::Table(t) => vec![t],
            Item::ArrayOfTables(list) => list.iter_mut().collect(),
            _ => continue,
        };
        for t in tables {
            if t.position() == Some(position) {
                return Some(t);
            }
            if let Some(found) = table_at(t, position) {
                return Some(found);
            }
        }
    }
    None
}

/// Each plugin table as a one-table document, for the move record, so it goes
/// back with its comments when Disconnect restores it.
pub(super) fn raw_entries(text: &str) -> Result<Vec<(String, String, Json)>, String> {
    let doc = document(text)?;
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for table in plugins(&doc)?.into_iter().flat_map(ArrayOfTables::iter) {
        let Some(name) = name_of(table) else {
            continue;
        };
        if !seen.insert(name.to_string()) {
            continue;
        }
        let mut one = DocumentMut::new();
        let mut list = ArrayOfTables::new();
        list.push(table.clone());
        one.insert("plugins", Item::ArrayOfTables(list));
        out.push((name.to_string(), one.to_string(), config_json(table)));
    }
    Ok(out)
}

/// The fields of a recorded one-table document.
pub(super) fn definition(raw: &str) -> Result<Json, String> {
    let doc = document(raw)?;
    plugins(&doc)?
        .and_then(|list| list.get(0))
        .map(config_json)
        .ok_or_else(|| "Could not read imported settings".to_string())
}

/// Append recorded tables back into the file.
pub(super) fn append_raw(text: Option<&str>, entries: &[String]) -> Result<String, String> {
    let text = text.unwrap_or("");
    let mut doc = document(text)?;
    let before_rest = rest(text)?;
    plugins(&doc)?;
    let mut desired = servers_map(&doc)?;
    if !matches!(doc.get("plugins"), Some(Item::ArrayOfTables(_))) {
        doc.remove("plugins");
        doc.insert("plugins", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let list = doc["plugins"]
        .as_array_of_tables_mut()
        .expect("set to an array of tables above");
    for raw in entries {
        let one = document(raw)?;
        let Some(table) = plugins(&one)?.and_then(|l| l.get(0)).cloned() else {
            continue;
        };
        let Some(name) = name_of(&table).map(str::to_string) else {
            continue;
        };
        if desired.contains_key(&name) {
            continue;
        }
        desired.insert(name, config_json(&table));
        list.push(table);
    }
    let out = doc.to_string();
    verify(&out, &before_rest, &desired)?;
    Ok(out)
}

fn verify(
    out: &str,
    before_rest: &Json,
    desired: &serde_json::Map<String, Json>,
) -> Result<(), String> {
    let doc = document(out).map_err(|e| {
        format!("Toolport produced an unreadable Reasonix config ({e}); {UNTOUCHED}.")
    })?;
    if servers_map(&doc)? != *desired || rest(out)? != *before_rest {
        return Err(format!(
            "Toolport could not apply this change exactly; {UNTOUCHED}."
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "reasonix_tests.rs"]
mod tests;
