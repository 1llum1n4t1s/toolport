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
        if self.stopped.get() || self.running.replace(true) { return; }
        let reader = self.clone();
        gtk::glib::spawn_future_local(async move {
            loop {
                let generation = reader.generation.get();
                let load = reader.load.clone();
                let result = gtk::gio::spawn_blocking(move || load()).await;
                if reader.stopped.get() { break; }
                if reader.generation.get() != generation { continue; }
                if let Ok(value) = result { (reader.render)(value); }
                break;
            }
            reader.running.set(false);
        });
    }

    pub(super) fn stop(&self) { self.stopped.set(true); }
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
            .monitor_directory(gtk::gio::FileMonitorFlags::WATCH_MOVES, gtk::gio::Cancellable::NONE)
            .ok()
    });
    if let Some(monitor) = monitor {
        let callback = changed.clone();
        let pending = Rc::new(Cell::new(false));
        monitor.connect_changed(move |_, file, other, _| {
            let relevant = [Some(file), other].into_iter().flatten().any(|file| {
                file.basename().is_some_and(|name| matches(&name.to_string_lossy()))
            });
            if !relevant || pending.replace(true) { return; }
            let callback = callback.clone();
            let pending = pending.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                pending.set(false);
                callback();
            });
        });
        window.connect_destroy(move |_| { monitor.cancel(); });
    } else {
        let source = gtk::glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
            changed();
            gtk::glib::ControlFlow::Continue
        });
        let source = std::cell::RefCell::new(Some(source));
        window.connect_destroy(move |_| {
            if let Some(source) = source.borrow_mut().take() { source.remove(); }
        });
    }
}

/// A visible page may need a clock (for relative times). Remove its source
/// altogether while unmapped, rather than waking up just to check visibility.
pub(super) fn while_mapped(widget: &gtk::Box, seconds: u64, tick: impl Fn() + 'static) {
    let tick = Rc::new(tick);
    let source = Rc::new(std::cell::RefCell::new(None::<gtk::glib::SourceId>));
    let source_for_map = source.clone();
    widget.connect_map(move |_| {
        tick();
        let tick = tick.clone();
        *source_for_map.borrow_mut() = Some(gtk::glib::timeout_add_local(
            std::time::Duration::from_secs(seconds), move || {
                tick();
                gtk::glib::ControlFlow::Continue
            },
        ));
    });
    widget.connect_unmap(move |_| {
        if let Some(source) = source.borrow_mut().take() { source.remove(); }
    });
}
