//! Read-only activation checks for nanobot-ai 0.3.5 Agent Plugin packages.
//! Unlike upstream discovery, never migrate or remove activation markers.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;

pub(super) const LABEL: &str = "nanobot package";
const SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;

fn contained(path: &Path, parent: &Path, directory: bool) -> Option<PathBuf> {
    let resolved = canon(path).ok()?;
    (resolved.starts_with(parent)
        && if directory {
            resolved.is_dir()
        } else {
            resolved.is_file()
        })
    .then_some(resolved)
}

fn children(path: &Path) -> Option<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(path).ok()? {
        paths.push(entry.ok()?.path());
        if paths.len() > MAX_ENTRIES {
            return None;
        }
    }
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Some(paths)
}

fn valid_name(name: &str, plugin: bool) -> bool {
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().next().is_some_and(alnum)
        && name.bytes().last().is_some_and(alnum)
        && name
            .bytes()
            .all(|c| alnum(c) || c == b'-' || (plugin && c == b'.'))
        && !name.contains("--")
        && (!plugin || !name.contains(".."))
}

fn posix(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    #[cfg(windows)]
    return Some(text.replace('\\', "/"));
    #[cfg(not(windows))]
    Some(text.to_string())
}

/// Python's sorted(Path.rglob('*')) orders path components, rather than whole
/// path strings. Windows Path comparison additionally folds case.
fn order(path: &Path) -> Option<Vec<String>> {
    path.components()
        .map(|part| {
            let text = part.as_os_str().to_str()?;
            #[cfg(windows)]
            return Some(text.to_lowercase());
            #[cfg(not(windows))]
            Some(text.to_string())
        })
        .collect()
}

pub(super) fn fingerprint(root: &Path) -> Option<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for path in children(&dir)? {
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(path.clone());
            }
            paths.push((order(path.strip_prefix(root).ok()?)?, path));
            if paths.len() > MAX_ENTRIES {
                return None;
            }
        }
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    for (_, path) in paths {
        digest.update(posix(path.strip_prefix(root).ok()?)?.as_bytes());
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.file_type().is_symlink() {
            digest.update(b"\0link\0");
            digest.update(posix(&std::fs::read_link(path).ok()?)?.as_bytes());
        } else if metadata.is_file() {
            if metadata.len() > MAX_PACKAGE_BYTES.saturating_sub(bytes) {
                return None;
            }
            digest.update(b"\0file\0");
            let mut file = std::fs::File::open(path).ok()?;
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).ok()?;
                if count == 0 {
                    break;
                }
                bytes = bytes.checked_add(count as u64)?;
                if bytes > MAX_PACKAGE_BYTES {
                    return None;
                }
                digest.update(&buffer[..count]);
            }
        } else if metadata.is_dir() {
            digest.update(b"\0dir\0");
        } else {
            return None;
        }
        digest.update(b"\0");
    }
    Some(format!("{:x}", digest.finalize()))
}

/// Python json.dumps(..., ensure_ascii=True), including UTF-16 surrogate pairs.
fn ascii_json(text: &str) -> Option<String> {
    let encoded = serde_json::to_string(text).ok()?;
    let mut out = String::new();
    for ch in encoded.chars() {
        if ch <= '~' {
            out.push(ch);
        } else {
            for part in ch.encode_utf16(&mut [0; 2]) {
                out.push_str(&format!("\\u{:04x}", *part));
            }
        }
    }
    Some(out)
}

pub(super) fn activation(root: &Path) -> Option<String> {
    Some(format!(
        "{{\"fingerprint\":\"{}\",\"root\":{}}}",
        fingerprint(root)?,
        ascii_json(root.to_str()?)?
    ))
}

pub(super) fn marker(home: &Path, workspace: &Path, name: &str) -> Option<PathBuf> {
    let config = crate::clients::nanobot::home_dir(home).join("config.json");
    let mut parent = if config.exists() {
        canon(&config).ok()?.parent()?.to_path_buf()
    } else {
        canon(config.parent()?).ok()?
    };
    let hash = crate::registry::sha256_hex(workspace.to_str()?);
    for segment in ["plugin-data", &hash[..12], name] {
        parent = contained(&parent.join(segment), &parent, true)?;
    }
    Some(parent.join("enabled"))
}

fn valid_skill(path: &Path, name: &str) -> bool {
    if !valid_name(name, false) {
        return false;
    }
    let Some(text) = read_text(path) else {
        return false;
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return false;
    }
    let Some((metadata_name, description)) = super::nanobot_metadata::fields(&yaml) else {
        return false;
    };
    metadata_name == name && (1..=1024).contains(&description.trim().chars().count())
}

pub(super) fn discover(home: &Path) -> Vec<Found> {
    let Some(workspace) =
        crate::clients::nanobot::workspace(home).and_then(|path| canon(&path).ok())
    else {
        return Vec::new();
    };
    let Some(plugins) = contained(&workspace.join("plugins"), &workspace, true) else {
        return Vec::new();
    };
    let mut installed = Vec::new();
    let mut counts = HashMap::new();
    for candidate in children(&plugins)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_ENTRIES)
    {
        let Some(root) = contained(&candidate, &plugins, true) else {
            continue;
        };
        let Some(manifest) = contained(&root.join("plugin.json"), &root, false)
            .and_then(|path| read_text(&path))
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        else {
            continue;
        };
        if manifest.get("$schema").and_then(|v| v.as_str()) != Some(SCHEMA) {
            continue;
        }
        let Some(name) = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|name| valid_name(name, true))
        else {
            continue;
        };
        *counts.entry(name.to_string()).or_insert(0) += 1;
        installed.push((name.to_string(), root));
    }
    let mut found = Vec::new();
    for (name, root) in installed {
        if counts[&name] != 1 {
            continue;
        }
        let Some(current) = marker(home, &workspace, &name).and_then(|path| read_text(&path))
        else {
            continue;
        };
        let Some(expected) = activation(&root) else {
            continue;
        };
        if current != expected && root.to_str() != Some(current.as_str()) {
            continue;
        }
        let Some(skills) = contained(&root.join("skills"), &root, true) else {
            continue;
        };
        for candidate in children(&skills)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_ENTRIES)
        {
            let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(skill_root) = contained(&candidate, &skills, true) else {
                continue;
            };
            let Some(real) = contained(&skill_root.join("SKILL.md"), &root, false) else {
                continue;
            };
            if !valid_skill(&real, name) {
                continue;
            }
            found.push(Found {
                link: candidate.join("SKILL.md"),
                real: Some(real),
                root_label: LABEL,
                root_path: skills.clone(),
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_matches_released_python_for_hidden_unicode_and_component_order() {
        let root = std::env::temp_dir().join(format!("nanobot-fingerprint-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        for (name, text) in [
            (".hidden", "hidden\n"),
            ("a/child", "child\n"),
            ("a-b", "sibling\n"),
            ("unicode-🍜", "utf8\n"),
        ] {
            std::fs::write(root.join(name), text).unwrap();
        }
        assert_eq!(
            fingerprint(&root).as_deref(),
            Some("bd1ae3bc88479a92ea853396bd6229489e3157128d2baf8029c55bf94c492db4")
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("a/child", root.join("link")).unwrap();
            assert_eq!(
                fingerprint(&root).as_deref(),
                Some("0bb740ddeeee8ef8f2373afbe89fce9297e9e96ed6ece9caa0f8f90c6d395c44")
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activation_ascii_escaping_matches_python_json_dumps() {
        assert_eq!(
            ascii_json("/tmp/é-🍜\u{7f}").unwrap(),
            r#""/tmp/\u00e9-\ud83c\udf5c\u007f""#
        );
    }
    #[test]
    fn complete_package_over_the_byte_limit_is_not_fingerprinted() {
        let root =
            std::env::temp_dir().join(format!("nanobot-large-package-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join("large")).unwrap();
        file.set_len(MAX_PACKAGE_BYTES + 1).unwrap();
        drop(file);
        assert!(fingerprint(&root).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
