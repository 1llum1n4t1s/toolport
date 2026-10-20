//! Pushed details share the main window and keep list state alive behind Back.
use super::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
pub(super) struct DetailHost {
    stack: gtk::Stack,
    pub selected: Rc<RefCell<Option<String>>>,
    pub tools: Rc<std::cell::Cell<bool>>,
    focus: Rc<RefCell<Option<gtk::glib::WeakRef<gtk::Widget>>>>,
}

impl DetailHost {
    pub fn attach(&self, root: &gtk::Box) {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        while let Some(child) = root.first_child() {
            root.remove(&child);
            list.append(&child);
        }
        self.stack.set_vexpand(true);
        self.stack.add_named(&list, Some("list"));
        root.append(&self.stack);
    }

    pub fn back(&self) {
        let Some(id) = self.selected.borrow_mut().take() else {
            return;
        };
        self.stack.set_visible_child_name("list");
        let target = self
            .stack
            .child_by_name("list")
            .and_then(|list| find_named(&list, &format!("detail-open:{id}")));
        if let Some(target) = target {
            target.grab_focus();
        } else if let Some(focus) = self.focus.borrow().as_ref().and_then(|weak| weak.upgrade()) {
            if focus.root().is_some() {
                focus.grab_focus();
            } else {
                self.stack.child_focus(gtk::DirectionType::TabForward);
            }
        }
        if let Some(detail) = self.stack.child_by_name("detail") {
            self.stack.remove(&detail);
        }
    }

    pub fn push(&self, id: &str, root: &gtk::Box) {
        let current_focus = self
            .stack
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|w| gtk::prelude::GtkWindowExt::focus(&w));
        let previous_path = self.stack.child_by_name("detail").and_then(|old| {
            current_focus
                .as_ref()
                .filter(|focus| focus.is_ancestor(&old))
                .map(|focus| focus_path(focus, &old))
        });
        if self.selected.borrow().is_none() {
            let focus = self
                .stack
                .root()
                .and_downcast::<gtk::Window>()
                .and_then(|w| gtk::prelude::GtkWindowExt::focus(&w));
            *self.focus.borrow_mut() = focus.map(|w| w.downgrade());
        }
        *self.selected.borrow_mut() = Some(id.to_string());
        if let Some(old) = self.stack.child_by_name("detail") {
            self.stack.remove(&old);
        }
        self.stack.add_named(root, Some("detail"));
        self.stack.set_visible_child_name("detail");
        if let Some(focus) = previous_path
            .and_then(|path| follow_focus_path(root.upcast_ref(), &path))
            .filter(|focus| focus.is_sensitive() && focus.is_visible())
        {
            focus.grab_focus();
        } else if let Some(back) = find_class(root.upcast_ref(), "toolport-page-back") {
            back.grab_focus();
        }
    }

    pub fn header(&self, title: &str, clients: bool) -> page_header::PageHeader {
        let header = page_header::PageHeader::new(
            if clients { "Clients" } else { "Servers" },
            if clients { "MCP client" } else { "Overview" },
        );
        header.title.set_label(title);
        let back = header.set_back(
            if clients { "Clients" } else { "Servers" },
            if clients {
                "Back to clients"
            } else {
                "Back to servers"
            },
        );
        let host = self.clone();
        back.connect_clicked(move |_| host.back());
        header
    }
}

fn find_class(root: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if root.has_css_class(class) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_class(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn heading(source: &'static str) -> gtk::Label {
    let label = l10n::label(source);
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Start);
    label.add_css_class("toolport-section-label");
    label
}

fn text(value: &str, mono: bool) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.add_css_class(if mono { "monospace" } else { "toolport-muted" });
    label
}

pub(super) fn server(server: &state::ServerView, page: &ServerPage, show_tools: bool) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let tools = ServerToolsPanel::new(&page.app, &server.id);
    let header = page.details.header(&server.name, false);
    header.add_leading(&branding::server_logo(&server.name, &server.transport));
    header
        .subtitle
        .set_label(&format!("{} · {}", server.origin_label, server.transport));
    let refresh = header_icon_button("view-refresh-symbolic");
    l10n::bind_tooltip(&refresh, "Refresh");
    let p = page.clone();
    let refresh_tools = tools.clone();
    let server_id = server.id.clone();
    refresh.connect_clicked(move |_| {
        if p.details.tools.get() {
            refresh_tools.refresh();
        }
        p.reprobe_after_auth_change(&server_id);
        p.reload_registry();
    });
    header.add_action(&refresh);
    root.append(&header.root);
    let tabs = gtk::Stack::new();
    tabs.set_vexpand(true);
    let overview = page_body(24);
    let snapshot = page.last_snapshot.borrow().clone();
    let profile = snapshot
        .as_ref()
        .map(|s| s.active_profile_id.as_str())
        .unwrap_or_default();
    // Reuse the live status/switch/auth controls and every existing action.
    let connection = row_group();
    connection.append(&server_row(server, profile, page.clone(), true));
    overview.append(&connection);
    let target = if let Some(command) = &server.command {
        let mask = crate::registry::secret_arg_mask(&server.args);
        std::iter::once(command.clone())
            .chain(server.args.iter().zip(mask).map(|(arg, secret)| {
                if secret {
                    "<redacted>".into()
                } else {
                    arg.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        server.url.clone().unwrap_or_default()
    };
    overview.append(&text(&target, true));
    if let Some(cwd) = &server.cwd {
        overview.append(&text(cwd, true));
    }
    for env in &server.plain_env {
        overview.append(&text(&format!("{}={}", env.key, env.value), true));
    }
    if !server.secret_keys.is_empty() || !server.secret_references.is_empty() {
        overview.append(&heading("Credentials"));
        for key in &server.secret_keys {
            overview.append(&text(&format!("{key}: ••••••••"), true));
        }
        for (key, reference) in &server.secret_references {
            overview.append(&text(&format!("{key}: {reference}"), true));
        }
    }
    let actions = server_actions(server, page.clone(), false);
    let flow = wrapping_actions();
    while let Some(child) = actions.first_child() {
        actions.remove(&child);
        child.remove_css_class("toolport-menu-item");
        child.add_css_class("toolport-secondary-action");
        flow.insert(&child, -1);
    }
    overview.append(&flow);
    let log = gtk::Box::new(gtk::Orientation::Vertical, 8);
    log.set_widget_name(&format!("server-log:{}", server.id));
    log.append(&heading("Connection log"));
    let error = page
        .health
        .borrow()
        .result(&server.id)
        .and_then(|probe| probe.error.clone());
    log.append(&text(error.as_deref().unwrap_or_default(), true));
    log.set_visible(error.is_some());
    overview.append(&log);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&overview)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    l10n::bind_stack_page_title(
        &tabs.add_titled(&scroll, Some("overview"), &crate::i18n::tr("Overview")),
        "Overview",
    );
    l10n::bind_stack_page_title(
        &tabs.add_titled(&tools.root, Some("tools"), &crate::i18n::tr("Tools")),
        "Tools",
    );
    let switcher = gtk::StackSwitcher::builder()
        .stack(&tabs)
        .halign(gtk::Align::Start)
        .margin_start(24)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    root.append(&switcher);
    root.append(&tabs);
    let tools_for_tab = tools.clone();
    let selected_tools = page.details.tools.clone();
    tabs.connect_visible_child_name_notify(move |tabs| {
        let active = tabs.visible_child_name().as_deref() == Some("tools");
        selected_tools.set(active);
        if active {
            tools_for_tab.refresh();
        }
    });
    page.details.tools.set(show_tools);
    if show_tools {
        tabs.set_visible_child_name("tools");
    }
    page.details.push(&server.id, &root);
}

pub(super) fn update_server_log(page: &ServerPage, server_id: &str, error: Option<&str>) {
    let Some(detail) = page.details.stack.child_by_name("detail") else {
        return;
    };
    if let Some(log) = find_named(&detail, &format!("server-log:{server_id}")) {
        if let Some(label) = log.last_child().and_downcast::<gtk::Label>() {
            label.set_label(error.unwrap_or_default());
        }
        log.set_visible(error.is_some());
    }
}

fn bound_dropdown(control: &gtk::DropDown) {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        item.set_child(Some(
            &gtk::Label::builder()
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(24)
                .xalign(0.0)
                .build(),
        ));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = item.child().and_downcast::<gtk::Label>().unwrap();
        let value = item.item().and_downcast::<gtk::StringObject>().unwrap();
        label.set_label(&value.string());
        label.set_tooltip_text(Some(&value.string()));
    });
    control.set_factory(Some(&factory));
}

pub(super) fn client(client: &state::ClientView, page: &ClientPage) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = page.details.header(&client.name, true);
    header.add_leading(&branding::client_logo(&client.id));
    let refresh = header_icon_button("view-refresh-symbolic");
    l10n::bind_tooltip(&refresh, "Scan clients again");
    let p = page.clone();
    refresh.connect_clicked(move |_| p.refresh());
    header.add_action(&refresh);
    root.append(&header.root);
    let body = page_body(24);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 8);
    info.append(&client_rows::status_badge(client));
    if let Some(detected) = &client.detected {
        info.append(&text(&detected.config_path, true));
    } else if let Some(profile) = &client.claude_profile {
        info.append(&text(&profile.folder, true));
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_valign(gtk::Align::Center);
    let connected = client.gateway_state == state::ClientGatewayState::Connected;
    let customized = client.gateway_state == state::ClientGatewayState::Customized;
    let writable = !client.config_error && (client.app_present || client.config_exists);
    let profiles = page.profiles.borrow().clone();
    let names = profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
    let scope = gtk::DropDown::from_strings(&names);
    scope.set_selected(
        profiles
            .iter()
            .position(|p| Some(p.id.as_str()) == client.scope_id.as_deref().or(Some("")))
            .unwrap_or(0) as u32,
    );
    bound_dropdown(&scope);
    scope.set_sensitive(writable && !customized);
    scope.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Access"))]);
    let selected_scope = Rc::new(RefCell::new(client.scope_id.clone()));
    let saved = selected_scope.clone();
    scope.connect_selected_notify(move |control| {
        *saved.borrow_mut() = profiles
            .get(control.selected() as usize)
            .map(|p| p.id.clone());
    });
    actions.append(&scope);
    if connected && !customized {
        let apply = l10n::button("Apply");
        apply.add_css_class("toolport-secondary-action");
        apply.set_sensitive(false);
        let apply_for_scope = apply.clone();
        let scope_for_apply = selected_scope.clone();
        let current_scope = client.scope_id.clone();
        scope.connect_selected_notify(move |_| {
            apply_for_scope.set_sensitive(
                writable
                    && scope_for_apply.borrow().as_deref().unwrap_or("")
                        != current_scope.as_deref().unwrap_or(""),
            )
        });
        let c = client.clone();
        let p = page.clone();
        let selected = selected_scope.clone();
        apply.connect_clicked(move |button| {
            run_client_mutation(
                &c,
                true,
                false,
                selected.borrow().clone(),
                button,
                p.clone(),
            )
        });
        actions.append(&apply);
    }
    let action = l10n::button(if customized || client.legacy_bearer_argv {
        client_reset_label(client)
    } else if connected {
        "Disconnect"
    } else {
        "Connect to Toolport"
    });
    action.add_css_class(if connected {
        "toolport-destructive"
    } else {
        "suggested-action"
    });
    action.set_sensitive(writable);
    let c = client.clone();
    let p = page.clone();
    let scope_for_connect = selected_scope.clone();
    action.connect_clicked(move |button| {
        if customized || c.legacy_bearer_argv {
            confirm_client_reset(&c, button.clone(), p.clone());
        } else if connected {
            confirm_client_disconnect(&c, button.clone(), p.clone());
        } else {
            let mut c = c.clone();
            c.scope_id = scope_for_connect.borrow().clone();
            confirm_client_migrate(&c, p.clone());
        }
    });
    actions.append(&action);
    if (customized || client.legacy_bearer_argv)
        && client.gateway_state != state::ClientGatewayState::Disconnected
    {
        let disconnect = l10n::button("Disconnect");
        disconnect.add_css_class("toolport-destructive");
        disconnect.set_sensitive(writable);
        let c = client.clone();
        let p = page.clone();
        disconnect.connect_clicked(move |button| {
            confirm_client_disconnect(&c, button.clone(), p.clone())
        });
        actions.append(&disconnect);
    }
    if client.location_removable {
        let remove = client_rows::remove_location_button(client, page.clone());
        remove.set_sensitive(!client.config_error && !connected && !customized);
        actions.append(&remove);
    }
    body.append(&page_header::HeaderWrap::new(&info, &actions));
    if client.config_error {
        let p = page.clone();
        body.append(&error_callout(
            "Couldn't read this client's configuration",
            &crate::i18n::tr(
                "Could not read this client config. Check its syntax and file permissions.",
            ),
            move || p.refresh(),
        ));
    }
    if let Some((id, notice)) = page.detail_notice.borrow().as_ref() {
        if id == &client.id {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = text(notice, false);
            label.set_hexpand(true);
            row.append(&label);
            let dismiss = header_icon_button("window-close-symbolic");
            l10n::bind_tooltip(&dismiss, "Dismiss restart reminder");
            let p = page.clone();
            let row_for_dismiss = row.clone();
            dismiss.connect_clicked(move |_| {
                p.detail_notice.borrow_mut().take();
                row_for_dismiss.set_visible(false);
            });
            row.append(&dismiss);
            body.append(&row);
        }
    }
    if customized {
        body.append(&text(&crate::i18n::tr("Toolport is leaving your hand-edited gateway entry as-is. Reset it only if you want Toolport to manage the entry."), false));
    }
    if client.legacy_bearer_argv {
        body.append(&text(&crate::i18n::tr("This older HTTP connection exposes a bearer credential in process arguments. Review migration to stdio; it stays unchanged until you confirm."), false));
    }
    if !customized {
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 8);
        copy.append(&heading("Discovery mode"));
        copy.append(&text(&client_rows::discovery_summary(client).1, false));
        let modes = [None, Some("lazy"), Some("grouped"), Some("full")];
        let mode = gtk::DropDown::from_strings(&[
            &crate::i18n::tr("Auto"),
            &crate::i18n::tr("Lazy · search"),
            &crate::i18n::tr("Grouped · per-server"),
            &crate::i18n::tr("Full · every tool"),
        ]);
        bound_dropdown(&mode);
        mode.set_selected(
            modes
                .iter()
                .position(|m| *m == client.discovery_mode.as_deref())
                .unwrap_or(0) as u32,
        );
        mode.set_sensitive(writable);
        mode.set_valign(gtk::Align::Start);
        mode.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
            "Discovery mode",
        ))]);
        let c = client.clone();
        let p = page.clone();
        mode.connect_selected_notify(move |control| {
            let Some(mode) = modes.get(control.selected() as usize).copied() else {
                return;
            };
            control.set_sensitive(false);
            let id = c.id.clone();
            let name = c.name.clone();
            let p = p.clone();
            let control = control.clone();
            gtk::glib::spawn_future_local(async move {
                let result = gtk::gio::spawn_blocking(move || {
                    crate::registry_controller::set_client_discovery(&id, mode)
                })
                .await;
                match result {
                    Ok(Ok(_)) => {
                        p.show_confirmation(&crate::i18n::trf(
                            "Updated discovery for {value0}.",
                            &[("value0", &name)],
                        ));
                        p.refresh();
                    }
                    Ok(Err(e)) => {
                        control.set_sensitive(true);
                        p.show_error(&e);
                    }
                    Err(_) => {
                        control.set_sensitive(true);
                        p.show_error(&crate::i18n::tr(
                            "the discovery update stopped unexpectedly",
                        ));
                    }
                }
            });
        });
        body.append(&page_header::HeaderWrap::new(&copy, &mode));
    }
    if connected && !customized {
        let reachable = gtk::Box::new(gtk::Orientation::Vertical, 8);
        reachable.append(&heading("Servers it can reach"));
        reachable.append(&skeleton_rows("Loading servers… "));
        body.append(&reachable);
        let scope = client.scope_id.clone();
        gtk::glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(move || {
                let registry = crate::registry::load()?;
                let default_scope = registry.default_access_id();
                let scope = scope
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&default_scope);
                Ok::<_, String>(
                    registry
                        .servers
                        .iter()
                        .filter(|s| registry.is_enabled(scope, &s.id))
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .await;
            while let Some(child) = reachable.last_child() {
                if child == reachable.first_child().unwrap() {
                    break;
                }
                reachable.remove(&child);
            }
            match result {
                Ok(Ok(names)) if !names.is_empty() => for name in names { reachable.append(&text(&name, false)); },
                Ok(Ok(_)) => reachable.append(&text(&crate::i18n::tr("No enabled servers in this access set yet. Turn servers on in Servers and include them under Settings > Advanced."), false)),
                _ => reachable.append(&text(&crate::i18n::tr("Servers unavailable"), false)),
            }
        });
    } else if !client.app_present && !client.config_exists {
        body.append(&text(
            &crate::i18n::tr("not installed on this machine"),
            false,
        ));
    }
    if client.uses_connectors {
        body.append(&text(&crate::i18n::tr("Uses account connectors"), false));
        body.append(&text(&crate::i18n::trf("Those live in {value1}'s Customize → Connectors and sync to your account, outside the local config files Toolport reads. Connecting Toolport adds a local gateway entry so your Toolport-managed servers appear in {value2} too.", &[("value1", &client.name), ("value2", &client.name)]), false));
    }
    if let Some(detected) = &client.detected {
        client_imports(
            &body,
            detected,
            page,
            writable,
            &client.imported_names,
            selected_scope,
        );
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&body)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    root.append(&scroll);
    page.details.push(&client.id, &root);
}

fn client_imports(
    body: &gtk::Box,
    detected: &crate::clients::DetectedClient,
    page: &ClientPage,
    writable: bool,
    imported_names: &[String],
    selected_scope: Rc<RefCell<Option<String>>>,
) {
    let servers = detected
        .servers
        .iter()
        .filter(|s| !crate::clients::detected_is_gateway(s))
        .chain(detected.plugin_servers.iter())
        .collect::<Vec<_>>();
    if servers.is_empty() {
        return;
    }
    body.append(&heading("Import into Toolport"));
    let explanation = gtk::Box::new(gtk::Orientation::Vertical, 8);
    explanation.append(&text(
        &crate::i18n::trf(
            "{value1} copies a server into Toolport; {value2} keeps its own copy.",
            &[
                ("value1", &crate::i18n::tr("Import")),
                ("value2", &detected.name),
            ],
        ),
        false,
    ));
    explanation.append(&text(&crate::i18n::trf("{review} copies it, then removes it from {client}'s config so the gateway is the only source (plugin servers stay). The cutover that actually saves context.", &[("review", &crate::i18n::tr("Review and connect")), ("client", &detected.name)]), false));
    let how = gtk::Expander::new(Some(&crate::i18n::tr("How importing works")));
    how.set_child(Some(&explanation));
    body.append(&how);
    if detected.gateway_installed
        && servers
            .iter()
            .any(|s| !imported_names.contains(&s.name.to_lowercase()))
    {
        let warning = text(&crate::i18n::trf("{client} is already connected to Toolport. Import on its own leaves a copy here too, so these tools load twice, once directly and once through the gateway. Use {review} to avoid that.", &[("client", &detected.name), ("review", &crate::i18n::tr("Review and connect"))]), false);
        warning.add_css_class("warning");
        body.append(&warning);
    }
    body.append(&text(&crate::i18n::tr("Review each command and URL. Values are saved using your keychain choices. Missing inputs stay off."), false));
    let all = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let import = l10n::button("Import all");
    import.add_css_class("toolport-secondary-action");
    import.set_sensitive(writable);
    let keys = servers
        .iter()
        .filter(|s| !imported_names.contains(&s.name.to_lowercase()))
        .map(|s| crate::clients::import_dedupe_key(&s.name, s.command.as_deref(), &s.args))
        .collect::<Vec<_>>();
    import.set_sensitive(writable && !keys.is_empty());
    let p = page.clone();
    import.connect_clicked(move |_| review_imports(&p, keys.clone()));
    all.append(&import);
    if servers
        .iter()
        .any(|s| detected.servers.iter().any(|local| std::ptr::eq(*s, local)))
    {
        let review = l10n::button("Review and connect");
        review.add_css_class("toolport-secondary-action");
        review.set_sensitive(writable);
        let p = page.clone();
        let id = detected.id.clone();
        review.connect_clicked(move |_| {
            let c = p
                .detail_clients
                .borrow()
                .iter()
                .find(|c| c.id == id)
                .cloned();
            if let Some(mut c) = c {
                c.scope_id = selected_scope.borrow().clone();
                confirm_client_migrate(&c, p.clone());
            }
        });
        all.append(&review);
    }
    body.append(&all);
    let group = row_group();
    for server in servers {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
        row.add_css_class("toolport-card");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let name = text(&server.name, false);
        name.remove_css_class("toolport-muted");
        name.add_css_class("heading");
        copy.append(&name);
        let target = server
            .command
            .clone()
            .map(|command| {
                let mask = crate::registry::secret_arg_mask(&server.args);
                std::iter::once(command)
                    .chain(server.args.iter().zip(mask).map(|(arg, secret)| {
                        if secret {
                            "<redacted>".into()
                        } else {
                            arg.clone()
                        }
                    }))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| server.url.clone().unwrap_or_default());
        copy.append(&text(&target, true));
        if detected
            .plugin_servers
            .iter()
            .any(|plugin| std::ptr::eq(plugin, server))
        {
            copy.append(&heading("Plugin"));
        }
        let imported = imported_names.contains(&server.name.to_lowercase());
        let import = l10n::button(if imported { "in Toolport" } else { "Import" });
        import.add_css_class("toolport-secondary-action");
        import.set_halign(gtk::Align::End);
        import.set_sensitive(writable && !imported);
        let key = crate::clients::import_dedupe_key(
            &server.name,
            server.command.as_deref(),
            &server.args,
        );
        let p = page.clone();
        import.connect_clicked(move |_| review_imports(&p, vec![key.clone()]));
        row.append(&page_header::HeaderWrap::new(&copy, &import));
        group.append(&row);
    }
    body.append(&group);
}

fn review_imports(page: &ClientPage, keys: Vec<String>) {
    let page = page.clone();
    gtk::glib::spawn_future_local(async move {
        match gtk::gio::spawn_blocking(move || {
            crate::registry_controller::preview_client_imports().map(|items| {
                items
                    .into_iter()
                    .filter(|item| keys.contains(&item.key))
                    .collect::<Vec<_>>()
            })
        })
        .await
        {
            Ok(Ok(items)) if !items.is_empty() => page.show_import_review(items),
            Ok(Ok(_)) => page.show_confirmation(&crate::i18n::tr("Nothing new to import")),
            Ok(Err(e)) => page.show_error(&e),
            Err(_) => page.show_error(&crate::i18n::tr("The import scan stopped unexpectedly.")),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut all = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(widget) = child {
            all.extend(widgets(&widget));
            child = widget.next_sibling();
        }
        all
    }
    fn fixture_client(id: &str) -> state::ClientView {
        state::ClientView {
            detected: None,
            imported_names: Vec::new(),
            id: id.into(),
            name: "Claude Code".into(),
            app_present: true,
            config_exists: true,
            uses_connectors: false,
            server_count: 0,
            movable_server_count: 0,
            other_server_count: 0,
            gateway_state: state::ClientGatewayState::Disconnected,
            shared_http: false,
            legacy_bearer_argv: false,
            scope_id: None,
            scope_name: None,
            discovery_mode: None,
            inherited_discovery: None,
            config_error: false,
            location_removable: true,
            claude_profile: None,
        }
    }
    fn snapshot(clients: Vec<state::ClientView>) -> state::ClientSnapshot {
        state::ClientSnapshot {
            clients,
            profiles: vec![
                state::ProfileView {
                    id: "".into(),
                    name: "Default access".into(),
                },
                state::ProfileView {
                    id: "scope-work".into(),
                    name: "Very long work access name that must wrap safely".into(),
                },
            ],
            sessions: vec![],
            sessions_error: false,
        }
    }
    fn app() -> adw::Application {
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("com.tsout.Toolport.B5DetailsTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gtk::gio::Cancellable::NONE).unwrap();
        app
    }
    #[test]
    #[ignore = "requires an isolated GTK desktop; run in omabox"]
    fn b5_clients_keep_exact_instance_refresh_and_safe_error_states() {
        let _data = crate::registry::DataDirTestEnv::new("b5-clients");
        let app = app();
        let (_, servers, _) = build_content(&app, crate::approval_broker::start_native());
        let page = ClientPage::new(&app, servers);
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .content(&page.root)
            .default_width(1024)
            .default_height(700)
            .build();
        let first = fixture_client("claude-code@personal");
        let second = fixture_client("codex@work");
        page.render(snapshot(vec![first.clone(), second.clone()]));
        window.present();
        let open = find_named(page.root.upcast_ref(), "detail-open:codex@work")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        open.grab_focus();
        open.emit_clicked();
        assert_eq!(
            page.details.selected.borrow().as_deref(),
            Some("codex@work")
        );
        assert_eq!(
            page.details.stack.visible_child_name().as_deref(),
            Some("detail")
        );
        let active = page.details.stack.child_by_name("detail").unwrap();
        assert!(widgets(&active)
            .iter()
            .any(|w| w.has_css_class("toolport-page-back")));
        let picker = widgets(&active)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::DropDown>().ok())
            .next()
            .unwrap();
        picker.set_selected(1); // Choosing access before Connect must never write a config.
        assert!(crate::registry::load().unwrap().client_scopes.is_empty());
        let mut changed = second.clone();
        changed.config_error = true;
        page.render(snapshot(vec![first.clone(), changed.clone()]));
        assert_eq!(
            page.details.selected.borrow().as_deref(),
            Some("codex@work")
        );
        let active = page.details.stack.child_by_name("detail").unwrap();
        let controls = widgets(&active);
        assert!(controls
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Button>())
            .find(|b| b.label().as_deref() == Some("Connect to Toolport"))
            .is_some_and(|b| !b.is_sensitive()));
        assert!(controls
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::DropDown>())
            .all(|c| !c.is_sensitive()));
        assert!(controls
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Label>())
            .any(|l| l.text() == "Couldn't read this client's configuration"));
        page.render(snapshot(vec![first.clone(), second.clone()]));
        page.details.back();
        assert_eq!(
            page.details.stack.visible_child_name().as_deref(),
            Some("list")
        );
        let replacement = find_named(page.root.upcast_ref(), "detail-open:codex@work").unwrap();
        assert!(replacement.is_focus());
        client(&second, &page);
        let mut disconnected = second.clone();
        disconnected.gateway_state = state::ClientGatewayState::Connected;
        page.render(snapshot(vec![first.clone(), disconnected]));
        assert!(
            widgets(&page.details.stack.child_by_name("detail").unwrap())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Button>())
                .any(|b| b.label().as_deref() == Some("Disconnect"))
        );
        page.render(snapshot(vec![first]));
        assert!(page.details.selected.borrow().is_none());
        assert_eq!(app.windows().len(), 1, "details stay in the main window");
        window.close();
    }

    #[test]
    #[ignore = "requires an isolated GTK desktop; run in omabox"]
    fn b5_server_details_push_back_refresh_and_preserve_capability_tabs() {
        let _data = crate::registry::DataDirTestEnv::new("b5-servers");
        let app = app();
        let (root, page, _) = build_content(&app, crate::approval_broker::start_native());
        let window = adw::ApplicationWindow::builder()
            .application(&app)
            .content(&root)
            .default_width(1120)
            .default_height(700)
            .build();
        let server = state::ServerView {
            id: "b5-server".into(),
            name: "A very long server title for a pushed detail page".into(),
            origin_label: "Personal".into(),
            transport: "Local stdio".into(),
            transport_id: "stdio".into(),
            command: Some("fixture-command".into()),
            args: vec!["--token".into(), "private-fixture-value".into()],
            launch: None,
            url: None,
            cwd: Some("/fixture/work".into()),
            inherit_env: false,
            secret_keys: vec!["API_KEY".into()],
            plain_env: vec![],
            secret_references: Default::default(),
            client_credentials: None,
            enabled: false,
            requires_review: false,
            team_route_removed: false,
            probe_fingerprint: 1,
        };
        let snapshot = state::RegistrySnapshot {
            servers: vec![server.clone()],
            profiles: vec![],
            enabled_count: 0,
            profile_count: 0,
            active_profile_id: "default".into(),
            paired: false,
            personal_sync: false,
        };
        page.render(state::RegistryState::Ready(snapshot.clone()));
        window.present();
        let open = find_named(root.upcast_ref(), "detail-open:b5-server")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        open.grab_focus();
        open.emit_clicked();
        assert_eq!(page.details.selected.borrow().as_deref(), Some("b5-server"));
        assert_eq!(app.windows().len(), 1);
        let labels = widgets(&page.details.stack.child_by_name("detail").unwrap())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Label>().ok())
            .map(|l| l.text().to_string())
            .collect::<Vec<_>>();
        assert!(labels
            .iter()
            .any(|s| s == "fixture-command --token <redacted>"));
        assert!(!labels.iter().any(|s| s.contains("private-fixture-value")));
        for title in [
            "Overview",
            "Tools",
            "Resources",
            "Prompts",
            "Credentials",
            "Edit",
            "Duplicate",
            "Remove",
        ] {
            assert!(labels.iter().any(|s| s == title), "missing {title}");
        }
        let mut changed = snapshot.clone();
        changed.servers[0].cwd = Some("/fixture/updated".into());
        page.render(state::RegistryState::Ready(changed));
        assert!(
            widgets(&page.details.stack.child_by_name("detail").unwrap())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Label>())
                .any(|l| l.text() == "/fixture/updated")
        );
        page.details.back();
        assert!(find_named(root.upcast_ref(), "detail-open:b5-server")
            .unwrap()
            .is_focus());
        server_details_for_test(&server, &page);
        page.render(state::RegistryState::Ready(state::RegistrySnapshot {
            servers: vec![],
            ..snapshot
        }));
        assert!(page.details.selected.borrow().is_none());
        window.close();
    }
    #[test]
    #[ignore = "requires an isolated GTK desktop; run in omabox"]
    fn b5_discovery_mutation_targets_only_the_selected_profile() {
        let _data = crate::registry::DataDirTestEnv::new("b5-profile-discovery");
        let app = app();
        let (_, servers, _) = build_content(&app, crate::approval_broker::start_native());
        let page = ClientPage::new(&app, servers);
        let first = fixture_client("claude-code@personal");
        let second = fixture_client("codex@work");
        page.render(snapshot(vec![first, second.clone()]));
        client(&second, &page);
        let active = page.details.stack.child_by_name("detail").unwrap();
        let dropdowns = widgets(&active)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::DropDown>().ok())
            .collect::<Vec<_>>();
        assert_eq!(dropdowns.len(), 2);
        dropdowns[1].set_selected(3);
        let context = gtk::glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if crate::registry::load()
                .unwrap()
                .client_discovery
                .get("codex@work")
                .is_some_and(|mode| mode == "full")
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let registry = crate::registry::load().unwrap();
        assert_eq!(
            registry
                .client_discovery
                .get("codex@work")
                .map(String::as_str),
            Some("full")
        );
        assert!(!registry
            .client_discovery
            .contains_key("claude-code@personal"));
        assert!(
            registry.client_scopes.is_empty(),
            "discovery does not rewrite access"
        );
    }

    fn server_details_for_test(server: &state::ServerView, page: &ServerPage) {
        super::server(server, page, false);
    }
}
