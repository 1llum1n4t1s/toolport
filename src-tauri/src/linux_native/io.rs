//! Worker reads and file events shared by the native shell only.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;

/// Trim once per hidden period, after a short debounce. Mapping or destroying
/// the window cancels the timer; reads and syncs never trigger allocator work.
pub(super) fn track_hidden_memory(window: &adw::ApplicationWindow) {
    #[cfg(target_env = "gnu")]
    {
        use std::cell::RefCell;
        use std::sync::atomic::{AtomicU64, Ordering};
        let epoch = Arc::new(AtomicU64::new(0));
        let source = Rc::new(RefCell::new(None::<gtk::glib::SourceId>));
        let schedule = {
            let epoch = epoch.clone();
            let source = source.clone();
            move |window: &adw::ApplicationWindow| {
                let generation = epoch.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(timer) = source.borrow_mut().take() {
                    timer.remove();
                }
                let epoch = epoch.clone();
                let source_for_timer = source.clone();
                let window = window.downgrade();
                *source.borrow_mut() = Some(gtk::glib::timeout_add_local_once(
                    std::time::Duration::from_secs(30),
                    move || {
                        source_for_timer.borrow_mut().take();
                        if window.upgrade().is_none_or(|window| window.is_mapped()) {
                            return;
                        }
                        drop(gtk::gio::spawn_blocking(move || {
                            if epoch.load(Ordering::SeqCst) == generation {
                                // SAFETY: glibc synchronizes its allocator. No
                                // pointers are passed and only free pages return.
                                unsafe {
                                    libc::malloc_trim(0);
                                }
                            }
                        }));
                    },
                ));
            }
        };
        schedule(window);
        window.connect_unmap(move |window| schedule(window));
        for destroy in [false, true] {
            let source = source.clone();
            let epoch = epoch.clone();
            let cancel = move || {
                epoch.fetch_add(1, Ordering::SeqCst);
                if let Some(timer) = source.borrow_mut().take() {
                    timer.remove();
                }
            };
            if destroy {
                on_window_closed(window, cancel);
            } else {
                window.connect_map(move |_| cancel());
            }
        }
    }
    #[cfg(not(target_env = "gnu"))]
    let _ = window;
}

/// GTK's destroy signal can wait for the last reference. Removing the window
/// from its application ends ownership immediately, even if a worker holds UI.
/// Hiding to the tray keeps the window registered and does not cancel requests.
pub(super) fn on_window_closed(window: &adw::ApplicationWindow, closed: impl Fn() + 'static) {
    let closed: Rc<dyn Fn()> = Rc::new(closed);
    let on_destroy = closed.clone();
    window.connect_destroy(move |_| on_destroy());
    if let Some(app) = window.application() {
        let owner = window.downgrade();
        let on_removed = closed.clone();
        app.connect_window_removed(move |_, removed| {
            if owner
                .upgrade()
                .is_some_and(|window| window.upcast_ref::<gtk::Window>() == removed)
            {
                on_removed();
            }
        });
        app.connect_shutdown(move |_| closed());
    }
}

/// Share requests replace earlier requests; destroyed owners reject completion.
#[derive(Default)]
pub(super) struct RequestGuard {
    generation: Cell<u64>,
    stopped: Cell<bool>,
}

impl RequestGuard {
    pub(super) fn begin(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        !self.stopped.get() && self.generation.get() == generation
    }

    pub(super) fn stop(&self) {
        self.stopped.set(true);
    }
}

/// One read at a time. An invalidation during a read discards that result and
/// starts one fresh read, so bursts cannot queue workers or overwrite newer state.
pub(super) struct SnapshotReader<T> {
    load: Arc<dyn Fn() -> T + Send + Sync>,
    render: Box<dyn Fn(T)>,
    generation: Cell<u64>,
    running: Cell<bool>,
    stopped: Cell<bool>,
}

impl<T: Send + 'static> SnapshotReader<T> {
    pub(super) fn new(
        load: impl Fn() -> T + Send + Sync + 'static,
        render: impl Fn(T) + 'static,
    ) -> Rc<Self> {
        Rc::new(Self {
            load: Arc::new(load),
            render: Box::new(render),
            generation: Cell::new(0),
            running: Cell::new(false),
            stopped: Cell::new(false),
        })
    }

    pub(super) fn refresh(self: &Rc<Self>) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if self.stopped.get() || self.running.replace(true) {
            return;
        }
        let reader = self.clone();
        gtk::glib::spawn_future_local(async move {
            loop {
                let generation = reader.generation.get();
                let load = reader.load.clone();
                let result = gtk::gio::spawn_blocking(move || load()).await;
                if reader.stopped.get() {
                    break;
                }
                if reader.generation.get() != generation {
                    continue;
                }
                if let Ok(value) = result {
                    (reader.render)(value);
                }
                break;
            }
            reader.running.set(false);
        });
    }

    pub(super) fn stop(&self) {
        self.stopped.set(true);
    }
}

/// Monitor the parent so atomic replacements and previously absent stores are
/// observed. Fallbacks preserve alert deadlines when the filesystem cannot watch.
pub(super) fn watch_data(
    window: &adw::ApplicationWindow,
    fallback_seconds: u64,
    hidden_alerts: bool,
    matches: impl Fn(&str) -> bool + 'static,
    changed: impl Fn() + 'static,
) {
    let changed = Rc::new(changed);
    let monitor = crate::registry::conduit_dir().and_then(|dir| {
        gtk::gio::File::for_path(dir)
            .monitor_directory(
                gtk::gio::FileMonitorFlags::WATCH_MOVES,
                gtk::gio::Cancellable::NONE,
            )
            .ok()
    });
    if let Some(monitor) = monitor {
        let callback = changed.clone();
        let pending = Rc::new(Cell::new(false));
        monitor.connect_changed(move |_, file, other, _| {
            let relevant = [Some(file), other].into_iter().flatten().any(|file| {
                file.basename()
                    .is_some_and(|name| matches(&name.to_string_lossy()))
            });
            if !relevant || pending.replace(true) {
                return;
            }
            let callback = callback.clone();
            let pending = pending.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                pending.set(false);
                callback();
            });
        });
        on_window_closed(window, move || {
            monitor.cancel();
        });
    } else {
        start_fallback_poll(window, fallback_seconds, hidden_alerts, changed);
    }
}

fn start_fallback_poll(
    window: &adw::ApplicationWindow,
    seconds: u64,
    hidden_alerts: bool,
    changed: Rc<dyn Fn()>,
) {
    let source = Rc::new(std::cell::RefCell::new(None::<gtk::glib::SourceId>));
    let start = {
        let source = source.clone();
        let changed = changed.clone();
        move || {
            if source.borrow().is_none() {
                let changed = changed.clone();
                *source.borrow_mut() = Some(gtk::glib::timeout_add_local(
                    std::time::Duration::from_secs(seconds),
                    move || {
                        changed();
                        gtk::glib::ControlFlow::Continue
                    },
                ));
            }
        }
    };
    if hidden_alerts || window.is_mapped() {
        start();
    }
    if !hidden_alerts {
        window.connect_map(move |_| {
            changed();
            start();
        });
        let source = source.clone();
        window.connect_unmap(move |_| {
            if let Some(timer) = source.borrow_mut().take() {
                timer.remove();
            }
        });
    }
    on_window_closed(window, move || {
        if let Some(timer) = source.borrow_mut().take() {
            timer.remove();
        }
    });
}

/// A visible page may need a clock (for relative times). Remove its source
/// altogether while unmapped, rather than waking up just to check visibility.
pub(super) fn while_mapped(widget: &gtk::Box, seconds: u64, tick: impl Fn() + 'static) {
    let tick = Rc::new(tick);
    let source = Rc::new(std::cell::RefCell::new(None::<gtk::glib::SourceId>));
    let source_for_map = source.clone();
    widget.connect_map(move |widget| {
        tick();
        if !widget.is_mapped() {
            return;
        }
        let tick = tick.clone();
        *source_for_map.borrow_mut() = Some(gtk::glib::timeout_add_local(
            std::time::Duration::from_secs(seconds),
            move || {
                tick();
                gtk::glib::ControlFlow::Continue
            },
        ));
    });
    widget.connect_unmap(move |_| {
        if let Some(source) = source.borrow_mut().take() {
            source.remove();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn requests_reject_replacement_and_destroyed_owner() {
        let guard = RequestGuard::default();
        let first = guard.begin();
        assert!(guard.is_current(first));
        let second = guard.begin();
        assert!(!guard.is_current(first));
        assert!(guard.is_current(second));
        guard.stop();
        assert!(!guard.is_current(second));
        assert!(!guard.is_current(guard.begin()));
    }

    #[test]
    #[ignore = "requires an isolated GTK desktop; run in omabox"]
    fn fallback_polls_keep_hidden_alerts_and_stop_hidden_page_work() {
        adw::init().unwrap();
        let window = adw::ApplicationWindow::builder()
            .hide_on_close(true)
            .build();
        let alerts = Rc::new(Cell::new(0));
        let pages = Rc::new(Cell::new(0));
        let count = alerts.clone();
        start_fallback_poll(
            &window,
            1,
            true,
            Rc::new(move || count.set(count.get() + 1)),
        );
        let count = pages.clone();
        start_fallback_poll(
            &window,
            1,
            false,
            Rc::new(move || count.set(count.get() + 1)),
        );
        gtk::glib::MainContext::default().block_on(async {
            gtk::glib::timeout_future(std::time::Duration::from_millis(1100)).await;
            assert!(alerts.get() >= 1);
            assert_eq!(pages.get(), 0);
            window.present();
            gtk::glib::timeout_future(std::time::Duration::from_millis(1100)).await;
            assert!(pages.get() >= 2);
            window.set_visible(false);
            let hidden_pages = pages.get();
            let hidden_alerts = alerts.get();
            gtk::glib::timeout_future(std::time::Duration::from_millis(1100)).await;
            assert_eq!(pages.get(), hidden_pages);
            assert!(alerts.get() > hidden_alerts);
        });
        window.destroy();
    }

    #[test]
    fn delayed_reads_leave_main_context_free_and_discard_superseded_results() {
        let context = gtk::glib::MainContext::new();
        context
            .with_thread_default(|| {
                context.block_on(async {
                    let owner = std::thread::current().id();
                    let calls = Arc::new(AtomicUsize::new(0));
                    let worker_calls = calls.clone();
                    let rendered = Rc::new(std::cell::RefCell::new(Vec::new()));
                    let result = rendered.clone();
                    let reader = SnapshotReader::new(
                        move || {
                            assert_ne!(std::thread::current().id(), owner);
                            let call = worker_calls.fetch_add(1, Ordering::SeqCst) + 1;
                            std::thread::sleep(std::time::Duration::from_millis(80));
                            call
                        },
                        move |value| result.borrow_mut().push(value),
                    );
                    reader.refresh();
                    gtk::glib::timeout_future(std::time::Duration::from_millis(10)).await;
                    assert!(
                        rendered.borrow().is_empty(),
                        "the main context advanced before slow IO completed"
                    );
                    for _ in 0..30 {
                        reader.refresh();
                    }
                    while reader.running.get() {
                        gtk::glib::timeout_future(std::time::Duration::from_millis(5)).await;
                    }
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        2,
                        "bursts coalesce to one replacement read"
                    );
                    assert_eq!(
                        *rendered.borrow(),
                        [2],
                        "an invalidated result never renders"
                    );
                    reader.refresh();
                    reader.stop();
                    while reader.running.get() {
                        gtk::glib::timeout_future(std::time::Duration::from_millis(5)).await;
                    }
                    assert_eq!(
                        *rendered.borrow(),
                        [2],
                        "destroyed windows reject late results"
                    );
                })
            })
            .unwrap();
    }
    #[test]
    #[ignore = "requires an isolated GTK desktop; run in omabox"]
    fn mapped_timers_stop_and_store_replacements_arrive_while_hidden() {
        adw::init().unwrap();
        let data = crate::registry::DataDirTestEnv::new("gtk-events-hidden");
        let app = adw::Application::builder()
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gtk::gio::Cancellable::NONE).unwrap();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .content(&root)
            .build();
        let ticks = Rc::new(Cell::new(0));
        let tick_count = ticks.clone();
        while_mapped(&root, 1, move || tick_count.set(tick_count.get() + 1));
        let changes = Rc::new(Cell::new(0));
        let change_count = changes.clone();
        watch_data(
            &window,
            15,
            true,
            |name| name == "registry.json",
            move || change_count.set(change_count.get() + 1),
        );
        window.present();
        gtk::glib::MainContext::default().block_on(async {
            gtk::glib::timeout_future(std::time::Duration::from_millis(1100)).await;
            assert!(ticks.get() >= 2);
            window.set_visible(false);
            let hidden_ticks = ticks.get();
            let path = data.dir.join("registry.json");
            crate::registry::atomic_write(&path, "fixture replacement").unwrap();
            gtk::glib::timeout_future(std::time::Duration::from_millis(1100)).await;
            assert_eq!(
                ticks.get(),
                hidden_ticks,
                "hidden pages have no timer callbacks"
            );
            assert!(
                changes.get() > 0,
                "atomic replacements wake hidden watchers"
            );
            window.present();
            gtk::glib::timeout_future(std::time::Duration::from_millis(50)).await;
            assert!(ticks.get() > hidden_ticks, "mapping refreshes immediately");
        });
        window.close();
    }
}
