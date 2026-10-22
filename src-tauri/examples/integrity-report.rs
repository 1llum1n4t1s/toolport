//! Offline integrity report for recorded tool lists, used by the ecosystem tracker.
//! Reads JSON lines on stdin, each `{"server": "...", "previous": [tools] | null,
//! "current": [tools]}`, and writes one JSON line per input with each current tool's
//! fingerprints and scan findings plus the drift events against `previous`. A line
//! that cannot be read or parsed gets an `{"error": ...}` line instead.
//! Touches no registry, pin store, keychain or network.
use conduit_lib::integrity;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn tools(value: &Value) -> Result<Vec<Value>, String> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "tool list must be an array".to_string())
}

fn report(line: &[u8]) -> Result<Value, String> {
    let line = std::str::from_utf8(line).map_err(|e| e.to_string())?;
    let input: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let server = input["server"].as_str().ok_or("missing server")?;
    let current = tools(&input["current"])?;
    let previous = match &input["previous"] {
        Value::Null => None,
        value => Some(tools(value)?),
    };
    let mut inspected: Vec<Value> = current.iter().map(integrity::inspect_definition).collect();
    inspected.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({
        "server": server,
        "tools": inspected,
        "events": integrity::compare_catalogs(server, previous.as_deref(), &current),
    }))
}

fn main() -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().split(b'\n') {
        let line = line?;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value = report(&line).unwrap_or_else(|error| json!({"error": error}));
        if let Err(e) = writeln!(out, "{value}") {
            // A closed pipe means the reader is done; anything else is a real failure.
            return if e.kind() == std::io::ErrorKind::BrokenPipe {
                Ok(())
            } else {
                Err(e)
            };
        }
    }
    match out.flush() {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e),
        _ => Ok(()),
    }
}
