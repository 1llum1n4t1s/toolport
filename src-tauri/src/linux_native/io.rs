//! Worker reads and file events shared by the native shell only.
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;

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
/// observed. The slow fallback exists only when the filesystem cannot watch.
pub(super) fn watch_data(
    window: &adw::ApplicationWindow,
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
        window.connect_destroy(move |_| {
            monitor.cancel();
        });
    } else {
        let source = gtk::glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
            changed();
            gtk::glib::ControlFlow::Continue
        });
        let source = std::cell::RefCell::new(Some(source));
        window.connect_destroy(move |_| {
            if let Some(source) = source.borrow_mut().take() {
                source.remove();
            }
        });
    }
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
