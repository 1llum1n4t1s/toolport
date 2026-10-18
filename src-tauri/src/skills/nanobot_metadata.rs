//! Preserve YAML scalar style and explicit tags before applying nanobot's
//! PyYAML 1.1 string-field contract. No YAML mutation or package code execution.
use std::collections::HashMap;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use unsafe_libyaml as yaml;

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
    raw: Box<MaybeUninit<yaml::yaml_parser_t>>,
    input: PhantomData<&'a [u8]>,
    remaining: usize,
}
impl<'a> Reader<'a> {
    fn new(input: &'a str) -> Option<Self> {
        let mut raw = Box::<yaml::yaml_parser_t>::new_uninit();
        // SAFETY: Box pins the parser's address. The input outlives Reader; only
        // successfully initialized parsers reach Drop.
        unsafe {
            if yaml::yaml_parser_initialize(raw.as_mut_ptr()).fail {
                return None;
            }
            yaml::yaml_parser_set_input_string(
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
        let mut event = MaybeUninit::<yaml::yaml_event_t>::uninit();
        // SAFETY: The parser owns its input and internal allocations. Union
        // fields are read only for their matching event type; all borrowed event
        // data is copied before yaml_event_delete, even on conversion errors.
        unsafe {
            if yaml::yaml_parser_parse(self.raw.as_mut_ptr(), event.as_mut_ptr()).fail {
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
                    yaml::YAML_STREAM_START_EVENT => Event::StreamStart,
                    yaml::YAML_STREAM_END_EVENT => Event::StreamEnd,
                    yaml::YAML_DOCUMENT_START_EVENT => Event::DocStart,
                    yaml::YAML_DOCUMENT_END_EVENT => Event::DocEnd,
                    yaml::YAML_MAPPING_START_EVENT => {
                        let data = event.data.mapping_start;
                        if !valid_tag(text(data.tag)?.as_deref(), "map") {
                            return None;
                        }
                        Event::Map(text(data.anchor)?)
                    }
                    yaml::YAML_MAPPING_END_EVENT => Event::MapEnd,
                    yaml::YAML_SEQUENCE_START_EVENT => {
                        let data = event.data.sequence_start;
                        if !valid_tag(text(data.tag)?.as_deref(), "seq") {
                            return None;
                        }
                        Event::Seq(text(data.anchor)?)
                    }
                    yaml::YAML_SEQUENCE_END_EVENT => Event::SeqEnd,
                    yaml::YAML_ALIAS_EVENT => Event::Alias(text(event.data.alias.anchor)??),
                    yaml::YAML_SCALAR_EVENT => {
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
                                plain: data.style == yaml::YAML_PLAIN_SCALAR_STYLE,
                            },
                            text(data.anchor)?,
                        )
                    }
                    _ => return None,
                })
            };
            let result = convert();
            yaml::yaml_event_delete(event);
            result
        }
    }
}
impl Drop for Reader<'_> {
    fn drop(&mut self) {
        // SAFETY: Reader is created only after successful initialization, and
        // each reader exclusively owns and deletes one parser.
        unsafe { yaml::yaml_parser_delete(self.raw.as_mut_ptr()) }
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
            Event::Scalar(value, anchor) => (anchor, Node::Scalar(value)),
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
                    values.push(self.node(event, depth + 1)?);
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
    fn field(&self, root: usize, name: &str, depth: usize) -> Option<usize> {
        if depth > 64 {
            return None;
        }
        let Node::Map(pairs) = self.nodes.get(root)? else {
            return None;
        };
        // Explicit keys win over merges regardless of their position; duplicate
        // explicit keys have PyYAML's last-key-wins behavior.
        for &(key, value) in pairs.iter().rev() {
            if let Node::Scalar(key) = &self.nodes[key] {
                if string(key) == Some(name) {
                    return Some(value);
                }
            }
        }
        for &(key, value) in pairs.iter().rev() {
            let Node::Scalar(key) = &self.nodes[key] else {
                continue;
            };
            if !merge_key(key) {
                continue;
            }
            if let Node::Seq(maps) = &self.nodes[value] {
                for &map in maps {
                    if let Some(value) = self.field(map, name, depth + 1) {
                        return Some(value);
                    }
                }
            } else if let Some(value) = self.field(value, name, depth + 1) {
                return Some(value);
            }
        }
        None
    }
}
fn merge_key(key: &Scalar) -> bool {
    key.value == "<<"
        && ((key.plain && key.tag.is_none())
            || key.tag.as_deref() == Some("tag:yaml.org,2002:merge"))
}

fn string(scalar: &Scalar) -> Option<&str> {
    if scalar.tag.is_some() || !scalar.plain {
        return Some(&scalar.value);
    }
    static TYPES: std::sync::OnceLock<regex::RegexSet> = std::sync::OnceLock::new();
    let types = TYPES.get_or_init(|| regex::RegexSet::new([
        r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$",
        r"^(?:~|null|Null|NULL|<<|=|)$",
        r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$",
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
        r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}(?:[Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?)$",
    ]).expect("constant PyYAML scalar resolvers"));
    (!types.is_match(&scalar.value)).then_some(scalar.value.as_str())
}

pub(super) fn fields(yaml: &str) -> Option<(String, String)> {
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
    let Node::Scalar(name) = &document.nodes[document.field(root, "name", 0)?] else {
        return None;
    };
    let Node::Scalar(description) = &document.nodes[document.field(root, "description", 0)?] else {
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
}
