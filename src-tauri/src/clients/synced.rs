//! Client formats edited through a normalized view instead of a key in a map.
//!
//! DeepSeek Harness keeps servers as entries in YAML patch rows, Reasonix as
//! `[[plugins]]` tables, and Codewhale under whichever of two keys its JSON
//! uses. None fits the shared `{key: {name: server}}` shape the move, restore
//! and mutation code works on, so each exposes the same five
//! operations: a `{"mcpServers": {name: fields}, ...}` view of the file, a sync
//! that makes the file's servers equal a desired map while keeping everything
//! else, and raw per-server text for the move record so a server goes back
//! exactly as it was.

use std::collections::BTreeMap;

use serde_json::Value as Json;

use super::{codewhale, dsh, reasonix, Format, McpServer, ServerEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Synced {
    Dsh,
    Reasonix,
    Codewhale,
}

/// The server map inside every normalized view.
pub(super) const KEY: &str = "mcpServers";

impl Synced {
    pub(super) fn of(format: Format) -> Option<Self> {
        match format {
            Format::YamlDshPatch => Some(Synced::Dsh),
            Format::TomlReasonixPlugins => Some(Synced::Reasonix),
            Format::JsonCodewhaleServers => Some(Synced::Codewhale),
            _ => None,
        }
    }

    pub(super) fn normalized(self, text: &str) -> Result<Json, String> {
        match self {
            Synced::Dsh => dsh::normalized(text),
            Synced::Reasonix => reasonix::normalized(text),
            Synced::Codewhale => codewhale::normalized(text),
        }
    }

    /// The file's servers, name -> fields.
    pub(super) fn servers_map(self, text: &str) -> Result<serde_json::Map<String, Json>, String> {
        Ok(self.normalized(text)?[KEY]
            .as_object()
            .cloned()
            .unwrap_or_default())
    }

    pub(super) fn servers(self, text: &str) -> Result<Vec<McpServer>, String> {
        match self {
            Synced::Dsh => dsh::servers(text),
            Synced::Reasonix => reasonix::servers(text),
            Synced::Codewhale => codewhale::servers(text),
        }
    }

    pub(super) fn config_of(self, entry: &ServerEntry) -> Result<Json, String> {
        match self {
            Synced::Dsh => dsh::config_of(entry),
            Synced::Reasonix => reasonix::config_of(entry),
            Synced::Codewhale => codewhale::config_of(entry),
        }
    }

    /// Make the file's servers `desired`. A server added back whose recorded
    /// raw text in `raw` still matches is written verbatim.
    pub(super) fn sync(
        self,
        text: Option<&str>,
        desired: &serde_json::Map<String, Json>,
        raw: &BTreeMap<String, String>,
    ) -> Result<String, String> {
        match self {
            Synced::Dsh => dsh::sync(text, desired, raw),
            Synced::Reasonix => reasonix::sync(text, desired, raw),
            Synced::Codewhale => codewhale::sync(text, desired, raw),
        }
    }

    /// `(name, raw text, fields)` for every server the client starts.
    pub(super) fn raw_entries(self, text: &str) -> Result<Vec<(String, String, Json)>, String> {
        match self {
            Synced::Dsh => dsh::raw_entries(text),
            Synced::Reasonix => reasonix::raw_entries(text),
            Synced::Codewhale => codewhale::raw_entries(text),
        }
    }

    /// The fields of one recorded raw entry.
    pub(super) fn definition(self, raw: &str) -> Result<Json, String> {
        match self {
            Synced::Dsh => dsh::definition(raw),
            Synced::Reasonix => reasonix::definition(raw),
            Synced::Codewhale => codewhale::definition(raw),
        }
    }

    pub(super) fn append_raw(
        self,
        text: Option<&str>,
        entries: &[String],
    ) -> Result<String, String> {
        match self {
            Synced::Dsh => dsh::append_raw(text, entries),
            Synced::Reasonix => reasonix::append_raw(text, entries),
            Synced::Codewhale => codewhale::append_raw(text, entries),
        }
    }
}
