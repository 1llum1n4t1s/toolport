//! Preserve YAML scalar style and explicit tags before applying nanobot's
//! PyYAML 1.1 string-field contract. No YAML mutation or package code execution.
use std::collections::HashMap;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use unsafe_libyaml as unsafe_yaml;

#[derive(Clone)]
struct Scalar {
    value: String,
    tag: Option<String>,
    plain: bool,
}
enum Event {
    Scalar(Scalar, Option<String>),
    Alias(String),
    Map(Option<String>),
    MapEnd,
    Seq(Option<String>),
    SeqEnd,
    StreamStart,
    StreamEnd,
    DocStart,
    DocEnd,
}
struct Reader<'a> {
    raw: Box<MaybeUninit<unsafe_yaml::yaml_parser_t>>,
    input: PhantomData<&'a [u8]>,
    remaining: usize,
}
impl<'a> Reader<'a> {
    fn new(input: &'a str) -> Option<Self> {
        let mut raw = Box::<unsafe_yaml::yaml_parser_t>::new_uninit();
        // SAFETY: Box pins the parser's address. The input outlives Reader; only
        // successfully initialized parsers reach Drop.
        unsafe {
            if unsafe_yaml::yaml_parser_initialize(raw.as_mut_ptr()).fail {
                return None;
            }
            unsafe_yaml::yaml_parser_set_input_string(
                raw.as_mut_ptr(),
                input.as_ptr(),
                input.len() as u64,
            );
        }
        Some(Self {
            raw,
            input: PhantomData,
            remaining: super::MAX_ENTRIES,
        })
    }
    fn next(&mut self) -> Option<Event> {
        self.remaining = self.remaining.checked_sub(1)?;
        let mut event = MaybeUninit::<unsafe_yaml::yaml_event_t>::uninit();
        // SAFETY: Reader keeps its input borrowed for the parser's lifetime. Union
        // fields are read only for their matching event type; all borrowed event
        // data is copied before yaml_event_delete, even on conversion errors.
        unsafe {
            if unsafe_yaml::yaml_parser_parse(self.raw.as_mut_ptr(), event.as_mut_ptr()).fail {
                return None;
            }
            let event = event.assume_init_mut();
            let convert = || -> Option<Event> {
                let text = |ptr: *const u8| {
                    if ptr.is_null() {
                        Some(None)
                    } else {
                        Some(Some(CStr::from_ptr(ptr.cast()).to_str().ok()?.to_string()))
                    }
                };
                Some(match event.type_ {
                    unsafe_yaml::YAML_STREAM_START_EVENT => Event::StreamStart,
                    unsafe_yaml::YAML_STREAM_END_EVENT => Event::StreamEnd,
                    unsafe_yaml::YAML_DOCUMENT_START_EVENT => Event::DocStart,
                    unsafe_yaml::YAML_DOCUMENT_END_EVENT => Event::DocEnd,
                    unsafe_yaml::YAML_MAPPING_START_EVENT => {
                        let data = event.data.mapping_start;
                        if !valid_tag(text(data.tag)?.as_deref(), "map") {
                            return None;
                        }
                        Event::Map(text(data.anchor)?)
                    }
                    unsafe_yaml::YAML_MAPPING_END_EVENT => Event::MapEnd,
                    unsafe_yaml::YAML_SEQUENCE_START_EVENT => {
                        let data = event.data.sequence_start;
                        if !valid_tag(text(data.tag)?.as_deref(), "seq") {
                            return None;
                        }
                        Event::Seq(text(data.anchor)?)
                    }
                    unsafe_yaml::YAML_SEQUENCE_END_EVENT => Event::SeqEnd,
                    unsafe_yaml::YAML_ALIAS_EVENT => Event::Alias(text(event.data.alias.anchor)??),
                    unsafe_yaml::YAML_SCALAR_EVENT => {
                        let data = event.data.scalar;
                        let tag = text(data.tag)?;
                        // Explicit non-string/custom scalar tags cannot satisfy
                        // these string fields. Conservatively skip the metadata
                        // rather than partially interpreting unknown constructors.
                        if !valid_tag(tag.as_deref(), "str")
                            && tag.as_deref() != Some("tag:yaml.org,2002:merge")
                        {
                            return None;
                        }
                        let value = std::str::from_utf8(std::slice::from_raw_parts(
                            data.value,
                            data.length as usize,
                        ))
                        .ok()?
                        .to_string();
                        Event::Scalar(
                            Scalar {
                                value,
                                tag,
                                plain: data.style == unsafe_yaml::YAML_PLAIN_SCALAR_STYLE,
                            },
                            text(data.anchor)?,
                        )
                    }
                    _ => return None,
                })
            };
            let result = convert();
            unsafe_yaml::yaml_event_delete(event);
            result
        }
    }
}
impl Drop for Reader<'_> {
    fn drop(&mut self) {
        // SAFETY: Reader is created only after successful initialization, and
        // each reader exclusively owns and deletes one parser.
        unsafe { unsafe_yaml::yaml_parser_delete(self.raw.as_mut_ptr()) }
    }
}
fn valid_tag(tag: Option<&str>, expected: &str) -> bool {
    tag.is_none_or(|tag| tag == "!" || tag == format!("tag:yaml.org,2002:{expected}"))
}
enum Node {
    Pending,
    Scalar(Scalar),
    Map(Vec<(usize, usize)>),
    Seq(Vec<usize>),
}
struct Document<'a> {
    reader: Reader<'a>,
    nodes: Vec<Node>,
    anchors: HashMap<String, usize>,
}
impl Document<'_> {
    fn node(&mut self, event: Event, depth: usize) -> Option<usize> {
        if depth > 64 {
            return None;
        }
        if let Event::Alias(name) = &event {
            return self.anchors.get(name).copied();
        }
        let anchor = match &event {
            Event::Scalar(_, anchor) | Event::Map(anchor) | Event::Seq(anchor) => anchor.as_ref(),
            _ => None,
        };
        if anchor.is_some_and(|anchor| self.anchors.contains_key(anchor)) {
            return None;
        }
        let index = self.nodes.len();
        self.nodes.push(Node::Pending);
        let (anchor, node) = match event {
            Event::Scalar(value, anchor) => {
                if !valid_constructor(&value) {
                    return None;
                }
                (anchor, Node::Scalar(value))
            }
            Event::Map(anchor) => {
                if let Some(anchor) = &anchor {
                    self.anchors.insert(anchor.clone(), index);
                }
                let mut pairs = Vec::new();
                loop {
                    let key = match self.reader.next()? {
                        Event::MapEnd => break,
                        event => self.node(event, depth + 1)?,
                    };
                    let event = self.reader.next()?;
                    let value = self.node(event, depth + 1)?;
                    if matches!(&self.nodes[value], Node::Scalar(scalar) if invalid_value(scalar)) {
                        return None;
                    }
                    pairs.push((key, value));
                }
                for &(key, value) in &pairs {
                    let Node::Scalar(key) = &self.nodes[key] else {
                        return None;
                    };
                    if merge_key(key)
                        && !match &self.nodes[value] {
                            Node::Map(_) | Node::Pending => true,
                            Node::Seq(maps) => maps
                                .iter()
                                .all(|&node| matches!(self.nodes[node], Node::Map(_))),
                            _ => false,
                        }
                    {
                        return None;
                    }
                }
                (anchor, Node::Map(pairs))
            }
            Event::Seq(anchor) => {
                if let Some(anchor) = &anchor {
                    self.anchors.insert(anchor.clone(), index);
                }
                let mut values = Vec::new();
                loop {
                    let event = self.reader.next()?;
                    if matches!(event, Event::SeqEnd) {
                        break;
                    }
                    let value = self.node(event, depth + 1)?;
                    if matches!(&self.nodes[value], Node::Scalar(scalar) if invalid_value(scalar)) {
                        return None;
                    }
                    values.push(value);
                }
                (anchor, Node::Seq(values))
            }
            _ => return None,
        };
        if let Some(anchor) = anchor {
            self.anchors.insert(anchor, index);
        }
        self.nodes[index] = node;
        Some(index)
    }
    fn field(&self, root: usize, name: &str, depth: usize, work: &mut usize) -> Option<usize> {
        *work = work.checked_sub(1)?;
        if depth > 64 {
            return None;
        }
        let Node::Map(pairs) = self.nodes.get(root)? else {
            return None;
        };
        // Explicit keys win over merges regardless of their position; duplicate
        // explicit keys have PyYAML's last-key-wins behavior.
        for &(key, value) in pairs.iter().rev() {
            *work = work.checked_sub(1)?;
            if let Node::Scalar(key) = &self.nodes[key] {
                if key.value == name && string(key).is_some() {
                    return Some(value);
                }
            }
        }
        for &(key, value) in pairs.iter().rev() {
            *work = work.checked_sub(1)?;
            let Node::Scalar(key) = &self.nodes[key] else {
                continue;
            };
            if !merge_key(key) {
                continue;
            }
            if let Node::Seq(maps) = &self.nodes[value] {
                for &map in maps {
                    *work = work.checked_sub(1)?;
                    if let Some(value) = self.field(map, name, depth + 1, work) {
                        return Some(value);
                    }
                }
            } else if let Some(value) = self.field(value, name, depth + 1, work) {
                return Some(value);
            }
        }
        None
    }
}
// Python's resolver regexes allow one final newline at their `$` anchor.
fn implicit_value(scalar: &Scalar) -> Option<&str> {
    if !matches!(scalar.tag.as_deref(), None | Some("!"))
        || (!scalar.plain && scalar.tag.as_deref() != Some("!"))
    {
        return None;
    }
    Some(scalar.value.strip_suffix('\n').unwrap_or(&scalar.value))
}

fn merge_key(key: &Scalar) -> bool {
    key.tag.as_deref() == Some("tag:yaml.org,2002:merge") || implicit_value(key) == Some("<<")
}

fn string(scalar: &Scalar) -> Option<&str> {
    match scalar.tag.as_deref() {
        Some("tag:yaml.org,2002:str") => return Some(&scalar.value),
        None | Some("!") => {}
        _ => return None,
    }
    implicit_value(scalar)
        .is_none_or(|value| !implicit_types().is_match(value))
        .then_some(scalar.value.as_str())
}

fn implicit_types() -> &'static regex::RegexSet {
    static TYPES: std::sync::OnceLock<regex::RegexSet> = std::sync::OnceLock::new();
    TYPES.get_or_init(|| regex::RegexSet::new([
        r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$",
        r"^(?:~|null|Null|NULL|<<|=|)$",
        r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$",
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
        r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}(?:[Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?)$",
    ]).expect("constant PyYAML scalar resolvers"))
}

fn invalid_value(scalar: &Scalar) -> bool {
    // PyYAML rewrites the value tag to a string only in mapping keys.
    merge_key(scalar) || implicit_value(scalar) == Some("=")
}

/// PyYAML constructs every value, even fields outside the identity contract.
fn valid_constructor(scalar: &Scalar) -> bool {
    let Some(value) = implicit_value(scalar) else {
        return true;
    };
    // PyYAML resolves these as bools but its boolean constructor rejects the
    // trailing newline. Integer constructors instead accept that whitespace.
    if value != scalar.value.as_str() && implicit_types().matches(value).matched(0) {
        return false;
    }
    let unsigned = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    for prefix in ["0b", "0x"] {
        if unsigned
            .strip_prefix(prefix)
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch == '_'))
        {
            return false; // The resolver accepts these, but int("", base) fails.
        }
    }
    static TIMESTAMP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = TIMESTAMP.get_or_init(|| regex::Regex::new(
        r"^(?P<y>[0-9]{4})-(?P<m>[0-9]{1,2})-(?P<d>[0-9]{1,2})(?:(?:[Tt]|[ \t]+)(?P<h>[0-9]{1,2}):(?P<min>[0-9]{2}):(?P<s>[0-9]{2})(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+](?P<zh>[0-9]{1,2})(?::(?P<zm>[0-9]{2}))?))?)?$"
    ).expect("constant PyYAML timestamp constructor"));
    let Some(parts) = pattern.captures(value) else {
        return true;
    };
    // A short month/day without a time is not implicitly a timestamp.
    if parts.name("h").is_none() && (parts["m"].len() != 2 || parts["d"].len() != 2) {
        return true;
    }
    let number = |key| {
        parts
            .name(key)
            .map(|m| m.as_str().parse::<u32>().expect("ASCII digits"))
            .unwrap_or(0)
    };
    let year = number("y");
    let month = number("m");
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0
        && (1..=days).contains(&number("d"))
        && number("h") < 24
        && number("min") < 60
        && number("s") < 60
        && number("zh") * 60 + number("zm") < 24 * 60
}

pub(super) fn fields(yaml: &str) -> Option<(String, String)> {
    let mut work = super::MAX_ENTRIES;
    fields_with_budget(yaml, &mut work)
}

fn fields_with_budget(yaml: &str, work: &mut usize) -> Option<(String, String)> {
    let mut document = Document {
        reader: Reader::new(yaml)?,
        nodes: Vec::new(),
        anchors: HashMap::new(),
    };
    if !matches!(document.reader.next()?, Event::StreamStart)
        || !matches!(document.reader.next()?, Event::DocStart)
    {
        return None;
    }
    let event = document.reader.next()?;
    let root = document.node(event, 0)?;
    if !matches!(document.reader.next()?, Event::DocEnd)
        || !matches!(document.reader.next()?, Event::StreamEnd)
    {
        return None;
    }
    let Node::Scalar(name) = &document.nodes[document.field(root, "name", 0, work)?] else {
        return None;
    };
    let Node::Scalar(description) =
        &document.nodes[document.field(root, "description", 0, work)?]
    else {
        return None;
    };
    Some((string(name)?.to_string(), string(description)?.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pyyaml_implicit_types_are_not_strings_but_quoted_and_tagged_scalars_are() {
        for value in [
            "yes",
            "NO",
            "on",
            "Off",
            "true",
            "null",
            "~",
            "012",
            "12:34",
            "1_000",
            "1:20.5",
            "2026-10-18",
            ".nan",
            ".Inf",
            "=",
        ] {
            assert!(
                fields(&format!("name: valid\ndescription: {value}\n")).is_none(),
                "{value}"
            );
            assert_eq!(
                fields(&format!("name: valid\ndescription: '{value}'\n")),
                Some(("valid".into(), value.into())),
                "{value}"
            );
            assert_eq!(
                fields(&format!("name: valid\ndescription: !!str {value}\n")),
                Some(("valid".into(), value.into())),
                "{value}"
            );
        }
        assert!(fields("name: yes\ndescription: ordinary\n").is_none());
        assert!(fields("name: valid\ndescription: ! yes\n").is_none());
        assert!(fields("name: valid\ndescription: !!merge <<\n").is_none());
        assert!(fields("name: valid\ndescription: ! 'yes'\n").is_none());
        assert_eq!(
            fields("name: 'yes'\ndescription: ordinary\n"),
            Some(("yes".into(), "ordinary".into()))
        );
        assert_eq!(
            fields("name: valid\ndescription: 0o12\n"),
            Some(("valid".into(), "0o12".into()))
        );
        assert_eq!(
            fields("name: >-\n  yes\ndescription: |-\n  2026-10-18\n"),
            Some(("yes".into(), "2026-10-18".into()))
        );
    }
    #[test]
    fn aliases_merge_precedence_and_duplicate_fields_match_pyyaml() {
        for yaml in [
            "base: &base {name: valid, description: 'yes'}\n<<: *base\n",
            "base: &base {name: valid, description: 'yes'}\n! <<: *base\n",
            "base: &base {name: valid, description: 'yes'}\n! '<<': *base\n",
            "base: &base {name: valid, description: 'yes'}\n!!merge '<<': *base\n",
            "<<: [{name: valid, description: 'yes'}, {name: other, description: 'no'}]\n",
            "base: &base {name: other, description: 'yes'}\nname: valid\n<<: *base\n",
            "name: first\nname: valid\nvalue: &description 'yes'\ndescription: *description\n",
        ] {
            assert_eq!(fields(yaml), Some(("valid".into(), "yes".into())), "{yaml}");
        }
        assert!(
            fields("name: valid\nbase: &description yes\ndescription: *description\n").is_none()
        );
        assert!(
            fields("name: valid\n<<: [{description: []}, {description: ordinary}]\n").is_none()
        );
        assert!(fields("name: valid\ndescription: ordinary\n<<: 7\n").is_none());
    }
    #[test]
    fn malformed_unsupported_and_excessively_nested_metadata_is_skipped() {
        for yaml in [
            "[bad",
            "name: valid\ndescription: !custom ordinary",
            "name: valid\ndescription: *missing",
            "a: &x first\nb: &x second\nname: valid\ndescription: ordinary",
            "---\nname: valid\ndescription: ordinary\n---\nother: document",
        ] {
            assert!(fields(yaml).is_none(), "{yaml}");
        }
        let nested = format!(
            "name: valid\ndescription: ordinary\nother: {}0{}",
            "[".repeat(66),
            "]".repeat(66)
        );
        assert!(fields(&nested).is_none());
    }
    #[test]
    fn alias_dag_missing_fields_share_a_finite_lookup_budget() {
        let mut yaml = "base: &n0 {other: ordinary}\n".to_string();
        for level in 1..=30 {
            yaml.push_str(&format!(
                "node{level}: &n{level} {{<<: [*n{}, *n{}]}}\n",
                level - 1,
                level - 1
            ));
        }
        yaml.push_str("name: valid\n<<: *n30\n");
        let mut work = super::super::MAX_ENTRIES;
        assert!(fields_with_budget(&yaml, &mut work).is_none());
        assert_eq!(
            work, 0,
            "the alias DAG must exhaust the shared bound instead of expanding exponentially"
        );
        assert!(fields("name: valid\nbase: &base {<<: *base}\n<<: *base\n").is_none());
    }
}

#[cfg(test)]
mod constructor_tests {
    use super::*;
    #[test]
    fn invalid_constructors_in_unrelated_values_skip_metadata() {
        for other in [
            "op: =",
            "op: <<",
            "when: 2026-13-45",
            "nested: [=]",
            "nested: {op: ! '<<'}",
        ] {
            assert!(
                fields(&format!("name: valid\ndescription: ordinary\n{other}\n")).is_none(),
                "{other}"
            );
        }
        assert!(fields("name: valid\ndescription: ! '1000'\n").is_none());
        assert!(fields("name: valid\ndescription: ordinary\n= : ordinary\n").is_some());
    }
}

#[cfg(test)]
mod standard_tag_tests {
    use super::*;
    #[test]
    fn standard_constructors_outside_the_supported_subset_are_skipped_conservatively() {
        for value in [
            "!!int 3",
            "!!float 1",
            "!!binary aGk=",
            "!!timestamp 2026-10-18",
            "!!set {a, b}",
            "!!omap [{a: b}]",
            "!!pairs [{a: b}]",
        ] {
            assert!(
                fields(&format!(
                    "name: valid\ndescription: ordinary\nother: {value}\n"
                ))
                .is_none(),
                "{value}"
            );
        }
    }
    #[test]
    fn implicit_timestamp_constructor_matches_python_calendar_and_clock_bounds() {
        for value in [
            "2026-13-45",
            "2026-02-29",
            "0000-01-01",
            "2024-02-29T24:00:00",
            "2024-02-29T12:60:00",
            "2024-02-29T12:00:60",
            "2024-02-29T12:00:00+24:00",
        ] {
            assert!(
                fields(&format!(
                    "name: valid\ndescription: ordinary\nother: {value}\n"
                ))
                .is_none(),
                "{value}"
            );
        }
        for value in [
            "2024-02-29",
            "2026-1-1",
            "2024-2-29 1:00:00Z",
            "2024-02-29T23:59:59.123456789+01:00",
        ] {
            assert!(
                fields(&format!(
                    "name: valid\ndescription: ordinary\nother: {value}\n"
                ))
                .is_some(),
                "{value}"
            );
        }
    }
}

#[cfg(test)]
mod resolver_newline_tests {
    use super::*;
    #[test]
    fn nonspecific_tags_preserve_python_resolver_and_constructor_newlines() {
        for value in [r#"! "yes\n""#, r#"! "=\n""#, r#"! "<<\n""#, "0x_", "-0b__"] {
            assert!(
                fields(&format!(
                    "name: valid\ndescription: ordinary\nother: {value}\n"
                ))
                .is_none(),
                "{value}"
            );
        }
        for value in [r#"! "1000\n""#, "0_", "'0x_'", "!!str 0b_"] {
            assert!(
                fields(&format!(
                    "name: valid\ndescription: ordinary\nother: {value}\n"
                ))
                .is_some(),
                "{value}"
            );
        }
        assert!(fields(
            r#"name: valid
description: ! "1000\n"
"#
        )
        .is_none());
        for key in [r#"! "<<\n""#, "!!merge anything"] {
            assert_eq!(
                fields(&format!(
                    "base: &base {{name: valid, description: ordinary}}\n{key}: *base\n"
                )),
                Some(("valid".into(), "ordinary".into()))
            );
        }
    }
}
