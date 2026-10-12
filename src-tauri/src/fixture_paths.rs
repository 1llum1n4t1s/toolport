//! Scratch roots for unit tests and opt-in production-linked fixtures only.
use std::path::PathBuf;

#[cfg(test)]
thread_local! { static ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }

#[cfg(test)]
pub(crate) struct RootGuard(Option<PathBuf>);
#[cfg(test)]
impl RootGuard {
    pub(crate) fn set(root: PathBuf) -> Self {
        Self(ROOT.with(|slot| slot.replace(Some(root))))
    }
}
#[cfg(test)]
impl Drop for RootGuard {
    fn drop(&mut self) { ROOT.with(|slot| { slot.replace(self.0.take()); }); }
}

pub(crate) fn dir(kind: &str) -> Option<PathBuf> {
    #[cfg(test)]
    let root = ROOT.with(|slot| slot.borrow().clone()).or_else(crate::clients::home)?;
    #[cfg(not(test))]
    let root = PathBuf::from(std::env::var_os("TOOLPORT_TEST_ROOT")?);
    assert!(root.is_absolute(), "fixture root must be absolute");
    let suffix = match kind {
        "home" => "",
        "config" => if cfg!(windows) { "AppData/Roaming" } else { ".config" },
        "data" => if cfg!(windows) { "AppData/Roaming" } else { ".local/share" },
        "local-data" => if cfg!(windows) { "AppData/Local" } else { ".local/share" },
        "cache" => if cfg!(windows) { "AppData/Local" } else { ".cache" },
        _ => unreachable!("known fixture root"),
    };
    Some(root.join(suffix))
}
