use crate::i18n::{tr, trf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::LazyLock;
use std::time::Duration;

use adw::prelude::*;
use image::GenericImageView;
use ksni::blocking::TrayMethods;

/// Written by the approval page when its queue changes. The
/// count must be visible with the window hidden - that is the whole point of a
/// tray - so it lives here rather than in any page state.
static PENDING_APPROVALS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn set_pending(count: usize) {
    if PENDING_APPROVALS.swap(count, Ordering::Relaxed) != count {
        TRAY_COMMANDS.with(|commands| {
            if let Some(commands) = commands.borrow().as_ref() {
                let _ = commands.send(Command::Pending(count));
            }
        });
    }
}

thread_local! {
    static TRAY_COMMANDS: std::cell::RefCell<Option<mpsc::Sender<Command>>> = const { std::cell::RefCell::new(None) };
}

/// Recompute menu labels on the SNI worker after a saved language change.
pub(super) fn refresh_language() {
    TRAY_COMMANDS.with(|commands| {
        if let Some(commands) = commands.borrow().as_ref() {
            let _ = commands.send(Command::LanguageChanged);
        }
    });
}

enum Command {
    Pending(usize),
    LanguageChanged,
    Show,
    ShowApprovals,
    Quit,
}

pub(super) struct ToolportTray {
    commands: mpsc::Sender<Command>,
    pending: usize,
}

impl ksni::Tray for ToolportTray {
    fn id(&self) -> String {
        "toolport".into()
    }

    fn title(&self) -> String {
        match self.pending {
            0 => "Toolport".into(),
            1 => tr("Toolport - 1 request awaiting action"),
            n => trf(
                "Toolport - {count} requests awaiting action",
                &[("count", &n.to_string())],
            ),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            ..Default::default()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICON: LazyLock<ksni::Icon> = LazyLock::new(|| {
            let image = image::load_from_memory_with_format(
                include_bytes!("../../icons/32x32.png"),
                image::ImageFormat::Png,
            )
            .expect("the bundled Toolport tray icon is a valid PNG");
            let (width, height) = image.dimensions();
            let mut data = image.into_rgba8().into_vec();
            for pixel in data.as_chunks_mut::<4>().0 {
                pixel.rotate_right(1);
            }
            ksni::Icon {
                width: width as i32,
                height: height as i32,
                data,
            }
        });
        vec![ICON.clone()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.commands.send(Command::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};

        vec![
            StandardItem {
                label: tr("Open Toolport"),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(Command::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                enabled: self.pending > 0,
                label: trf(
                    "Pending approvals ({count})",
                    &[("count", &self.pending.to_string())],
                ),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(Command::ShowApprovals);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: tr("Quit Toolport"),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.commands.send(Command::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// How long a hidden launch waits for a tray host before showing the window.
/// At login the host (the Omarchy shell, Waybar) often starts in the same
/// second as Toolport and registers its watcher a moment later.
pub(super) const HOST_GRACE: Duration = Duration::from_secs(15);

/// A hidden launch found no tray host yet. Keep the window hidden while the host
/// may still be starting, and present it if none appears within `grace` so the
/// app never sits running with no way to reach it.
pub(super) fn present_unless_host_appears(app: &adw::Application, grace: Duration) {
    let appeared = std::rc::Rc::new(std::cell::Cell::new(false));
    let present = appeared.clone();
    let watcher = gtk::gio::bus_watch_name(
        gtk::gio::BusType::Session,
        "org.kde.StatusNotifierWatcher",
        gtk::gio::BusNameWatcherFlags::NONE,
        move |_, _, _| present.set(true),
        |_, _| {},
    );
    let app = app.clone();
    gtk::glib::timeout_add_local_once(grace, move || {
        gtk::gio::bus_unwatch_name(watcher);
        if !appeared.get() {
            if let Some(window) = app.windows().into_iter().next() {
                window.present();
            }
        }
    });
}

pub(super) fn start(app: &adw::Application) -> Option<ksni::blocking::Handle<ToolportTray>> {
    let (sender, receiver) = mpsc::channel();
    let handle = ToolportTray {
        commands: sender.clone(),
        pending: 0,
    }
    .assume_sni_available(true)
    .spawn()
    .ok()?;
    PENDING_APPROVALS.store(0, Ordering::Relaxed);
    TRAY_COMMANDS.with(|commands| *commands.borrow_mut() = Some(sender));
    let app: gtk::glib::SendWeakRef<adw::Application> = app.downgrade().into();
    let handle_for_updates = handle.clone();
    // Block on commands on one worker. GTK wakes only for an actual tray action;
    // pending-count updates go straight to the SNI worker even while hidden.
    std::thread::spawn(move || {
        while let Ok(command) = receiver.recv() {
            if let Command::Pending(pending) = command {
                let _ = handle_for_updates.update(move |tray| tray.pending = pending);
                continue;
            }
            if matches!(command, Command::LanguageChanged) {
                let _ = handle_for_updates.update(|_| {});
                continue;
            }
            let app = app.clone();
            gtk::glib::idle_add_once(move || {
                let Some(app) = app.upgrade() else {
                    return;
                };
                match command {
                    Command::Show => app.activate(),
                    Command::ShowApprovals => {
                        app.activate();
                        if let Some(action) = app.lookup_action("show-servers") {
                            action.activate(None);
                        }
                    }
                    Command::Quit => app.quit(),
                    Command::Pending(_) | Command::LanguageChanged => {}
                }
            });
        }
    });
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_are_actionable_only_when_pending() {
        let (commands, _receiver) = mpsc::channel();
        for pending in [0, 1, 3] {
            let tray = ToolportTray {
                commands: commands.clone(),
                pending,
            };
            let menu = ksni::Tray::menu(&tray);
            let ksni::MenuItem::Standard(item) = &menu[1] else {
                panic!("approval item");
            };
            assert_eq!(item.enabled, pending > 0);
        }
    }

    #[test]
    fn tray_uses_the_embedded_icon_without_a_theme_lookup() {
        let (commands, _receiver) = mpsc::channel();
        let tray = ToolportTray {
            commands,
            pending: 0,
        };
        assert!(ksni::Tray::icon_name(&tray).is_empty());
        let icons = ksni::Tray::icon_pixmap(&tray);
        assert_eq!(icons.len(), 1);
        assert_eq!((icons[0].width, icons[0].height), (32, 32));
        assert_eq!(icons[0].data.len(), 32 * 32 * 4);
    }

    /// The tray menu is drawn by the host's icon theme, not by Toolport's CSS.
    /// Omarchy's Yaru-sage has no `window-new` or `security-high`, so named
    /// icons rendered as fallback squares next to a loud red exit glyph. The
    /// menu stays text-only so it cannot break on whatever theme is active.
    #[test]
    fn tray_menu_items_do_not_name_theme_icons() {
        let (commands, _receiver) = mpsc::channel();
        let tray = ToolportTray {
            commands,
            pending: 0,
        };
        for item in ksni::Tray::menu(&tray) {
            if let ksni::MenuItem::Standard(item) = item {
                assert!(
                    item.icon_name.is_empty(),
                    "{} names a theme icon",
                    item.label
                );
            }
        }
    }
}
