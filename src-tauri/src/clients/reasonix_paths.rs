//! Reasonix state relocation shared by team-rule writes and skill inventory.
use std::path::{Path, PathBuf};

pub(crate) fn state_root(
    home: &Path,
    reasonix: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> PathBuf {
    let configured = || {
        let text = std::fs::read_to_string(reasonix.join("config.toml")).ok()?;
        let config: toml::Value = toml::from_str(&text).ok()?;
        config
            .get("storage")?
            .get("state")?
            .as_str()
            .map(str::to_string)
    };
    env("REASONIX_STATE_HOME")
        .filter(|s| !s.trim().is_empty())
        .or_else(configured)
        .and_then(|raw| expand(home, &raw, env))
        .unwrap_or_else(|| reasonix.to_path_buf())
}

fn expand(home: &Path, raw: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let pattern =
        regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(:-([^}]*))?\}").expect("valid pattern");
    let expanded = pattern.replace_all(raw.trim(), |c: &regex::Captures<'_>| {
        env(&c[1])
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| c.get(3).map_or("", |m| m.as_str()).to_string())
    });
    let path = match expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
    {
        Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
        None if expanded == "~" => home.to_path_buf(),
        None => PathBuf::from(expanded.as_ref()),
    };
    // Relative paths depend on Reasonix's launch directory, like REASONIX_HOME.
    (!expanded.is_empty() && path.is_absolute()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_state_expands_paths_and_environment_takes_precedence() {
        let root =
            std::env::temp_dir().join(format!("toolport-reasonix-roots-{}", std::process::id()));
        let home = root.as_path();
        let reasonix = home.join(".reasonix");
        std::fs::create_dir_all(&reasonix).unwrap();
        std::fs::write(
            reasonix.join("config.toml"),
            "[storage]\nstate = '~/relocated'\n",
        )
        .unwrap();
        assert_eq!(
            state_root(home, &reasonix, &|_| None),
            home.join("relocated")
        );
        let override_path = home.join("override");
        assert_eq!(
            state_root(home, &reasonix, &|key| (key == "REASONIX_STATE_HOME")
                .then(|| override_path.to_string_lossy().into_owned())),
            override_path
        );
        std::fs::write(
            reasonix.join("config.toml"),
            "[storage]\nstate = '${RX_ROOT:-~}/state'\n",
        )
        .unwrap();
        assert_eq!(state_root(home, &reasonix, &|_| None), home.join("state"));
        assert_eq!(
            state_root(home, &reasonix, &|key| (key == "RX_ROOT")
                .then(|| override_path.to_string_lossy().into_owned())),
            override_path.join("state")
        );
        std::fs::write(reasonix.join("config.toml"), "broken = [").unwrap();
        assert_eq!(state_root(home, &reasonix, &|_| None), reasonix);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tilde_expansion_keeps_repeated_separators_under_home() {
        let home = std::env::temp_dir().join("toolport-reasonix-tilde-home");
        for raw in ["~//state", "~///state", r"~\\state", r"~/\/state"] {
            assert_eq!(
                expand(&home, raw, &|_| None),
                Some(home.join("state")),
                "{raw}"
            );
        }
        assert_eq!(expand(&home, "~", &|_| None), Some(home.clone()));
        assert_eq!(expand(&home, home.to_str().unwrap(), &|_| None), Some(home));
    }
}
