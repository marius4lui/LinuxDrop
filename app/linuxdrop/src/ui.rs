use crate::i18n::{german, tr};
use crate::{ipc, settings};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub proxy: RefCell<Option<gio::DBusProxy>>,
    pub snapshot: RefCell<Value>,
    pub settings: RefCell<Value>,
    stack: adw::ViewStack,
    status: gtk::Button,
    connection: gtk::Label,
    peers: gtk::Box,
    file_box: gtk::Box,
    drop_zone: gtk::Box,
    drop_details: Vec<gtk::Widget>,
    choose_button: gtk::Button,
    files: RefCell<Vec<gio::File>>,
    selected: RefCell<Option<String>>,
    protocol: gtk::DropDown,
    send: gtk::Button,
    share_link: gtk::Button,
    send_caption: gtk::Label,
    transfers: gtk::Box,
    hardware: gtk::Box,
    pub settings_body: gtk::Box,
    busy: Cell<bool>,
    refreshing: Cell<bool>,
    rendered_settings: RefCell<Value>,
    revision: RefCell<String>,
}

pub fn label(text: &str, class: &str) -> gtk::Label {
    let widget = gtk::Label::builder()
        .label(tr(text))
        .xalign(0.0)
        .wrap(true)
        .build();
    if !class.is_empty() {
        widget.add_css_class(class);
    }
    widget
}

pub fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn padded(vertical: i32, horizontal: i32, spacing: i32) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(spacing)
        .margin_top(vertical)
        .margin_bottom(vertical)
        .margin_start(horizontal)
        .margin_end(horizontal)
        .build()
}

fn page(body: &gtk::Box) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .tightening_threshold(560)
        .child(body)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build()
}

pub fn bytes(value: u64) -> String {
    if value < 1000 {
        format!("{value} B")
    } else if value < 1_000_000 {
        format!("{:.1} KB", value as f64 / 1000.0)
    } else if value < 1_000_000_000 {
        format!("{:.1} MB", value as f64 / 1_000_000.0)
    } else {
        format!("{:.2} GB", value as f64 / 1_000_000_000.0)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn array(value: &Value, key: &str) -> Vec<Value> {
    value[key].as_array().cloned().unwrap_or_default()
}
pub fn protocol_name(id: &str) -> &str {
    match id {
        "localsend" => "LocalSend",
        "quickshare" => "Quick Share",
        "airdrop" => "AirDrop",
        _ => id,
    }
}

pub fn build(app: &adw::Application, initial_page: &str, initial_files: Vec<gio::File>) {
    let review_size = std::env::var("LINUXDROP_REVIEW_SIZE")
        .ok()
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some((
                width.parse::<i32>().ok()?.clamp(420, 1920),
                height.parse::<i32>().ok()?.clamp(480, 1440),
            ))
        })
        .unwrap_or((610, 760));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr("LinuxDrop"))
        .default_width(review_size.0)
        .default_height(review_size.1)
        .width_request(420)
        .height_request(480)
        .build();
    window.add_css_class("linuxdrop");
    let stack = adw::ViewStack::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("LinuxDrop", "")));
    let status = gtk::Button::with_label(&tr("Connecting…"));
    status.add_css_class("flat");
    status.add_css_class("status-chip");
    status.set_tooltip_text(Some("Change who can discover this device"));
    header.pack_end(&status);
    toolbar.add_top_bar(&header);
    let switcher = adw::ViewSwitcherBar::builder()
        .stack(&stack)
        .reveal(true)
        .build();
    toolbar.add_bottom_bar(&switcher);
    toolbar.set_content(Some(&stack));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    let body = padded(24, 26, 20);
    let intro = gtk::Box::new(gtk::Orientation::Vertical, 7);
    intro.append(&label("Send files", "hero-title"));
    intro.append(&label("Share with nearby devices.", "hero-subtitle"));
    body.append(&intro);
    let connection = label("Connecting to the sharing service…", "compact-note");
    body.append(&connection);

    let drop = gtk::Box::new(gtk::Orientation::Vertical, 12);
    drop.add_css_class("drop-zone");
    let icon = gtk::Image::from_icon_name("document-send-symbolic");
    icon.set_pixel_size(34);
    icon.set_halign(gtk::Align::Center);
    icon.add_css_class("drop-icon");
    drop.append(&icon);
    let title = label("Drop something here", "drop-title");
    title.set_halign(gtk::Align::Center);
    drop.append(&title);
    let hint = label(
        "Photos, documents, and everything in between",
        "compact-note",
    );
    hint.set_halign(gtk::Align::Center);
    drop.append(&hint);
    let choose = gtk::Button::with_label(&tr("Choose files"));
    choose.add_css_class("pill");
    choose.set_halign(gtk::Align::Center);
    drop.append(&choose);
    body.append(&drop);
    let file_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&file_box);

    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let nearby = label("Nearby", "section-heading");
    nearby.set_hexpand(true);
    heading.append(&nearby);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some("Refresh nearby devices"));
    heading.append(&refresh);
    body.append(&heading);
    let peers = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&peers);
    let send_bar = gtk::Box::new(gtk::Orientation::Vertical, 10);
    send_bar.add_css_class("send-bar");
    let send_caption = label("Choose files and a nearby device", "compact-note");
    send_bar.append(&send_caption);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let protocol = gtk::DropDown::from_strings(&[&tr("Automatic")]);
    protocol.set_hexpand(true);
    protocol.set_tooltip_text(Some("Transfer protocol"));
    actions.append(&protocol);
    let send = gtk::Button::with_label(&tr("Send files"));
    send.add_css_class("suggested-action");
    send.add_css_class("pill");
    send.set_sensitive(false);
    actions.append(&send);
    send_bar.append(&actions);
    let share_link = gtk::Button::with_label(&tr("Share with a link"));
    share_link.add_css_class("flat");
    share_link.set_sensitive(false);
    send_bar.append(&share_link);
    send_bar.set_margin_start(20);
    send_bar.set_margin_end(20);
    send_bar.set_margin_bottom(14);
    let send_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    send_page.append(&page(&body));
    send_page.append(&send_bar);
    stack.add_titled_with_icon(
        &send_page,
        Some("send"),
        &tr("Send"),
        "document-send-symbolic",
    );

    let transfer_page = padded(24, 26, 20);
    transfer_page.append(&label("Transfers", "hero-title"));
    transfer_page.append(&label(
        "Requests, progress, and recently shared files.",
        "hero-subtitle",
    ));
    let transfers = gtk::Box::new(gtk::Orientation::Vertical, 12);
    transfer_page.append(&transfers);
    stack.add_titled_with_icon(
        &page(&transfer_page),
        Some("transfers"),
        &tr("Transfers"),
        "folder-download-symbolic",
    );
    let hardware_page = padded(24, 26, 20);
    hardware_page.append(&label("Hardware", "hero-title"));
    hardware_page.append(&label(
        "Your adapters, their capabilities, and what is ready.",
        "hero-subtitle",
    ));
    let hardware = gtk::Box::new(gtk::Orientation::Vertical, 12);
    hardware_page.append(&hardware);
    stack.add_titled_with_icon(
        &page(&hardware_page),
        Some("hardware"),
        &tr("Hardware"),
        "network-wireless-symbolic",
    );
    let settings_body = padded(24, 20, 20);
    stack.add_titled_with_icon(
        &page(&settings_body),
        Some("settings"),
        &tr("Settings"),
        "preferences-system-symbolic",
    );
    stack.set_visible_child_name(if initial_page == "notch-drop" {
        "send"
    } else {
        initial_page
    });
    let ui = Rc::new(Ui {
        window,
        toasts,
        proxy: RefCell::new(None),
        snapshot: RefCell::new(Value::Null),
        settings: RefCell::new(Value::Null),
        stack,
        status,
        connection,
        peers,
        file_box,
        drop_zone: drop.clone(),
        drop_details: vec![icon.upcast(), title.upcast(), hint.upcast()],
        choose_button: choose.clone(),
        files: RefCell::new(Vec::new()),
        selected: RefCell::new(None),
        protocol,
        send,
        share_link,
        send_caption,
        transfers,
        hardware,
        settings_body,
        busy: Cell::new(false),
        refreshing: Cell::new(false),
        rendered_settings: RefCell::new(Value::Null),
        revision: RefCell::new(String::new()),
    });

    let weak = Rc::downgrade(&ui);
    choose.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.choose_files();
        }
    });
    let weak = Rc::downgrade(&ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.send.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.start_send();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.share_link.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.share_link();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.status.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let visible = ui.settings.borrow()["visibility"]["mode"].as_str() == Some("everyone");
            ui.mutate(
                "SetVisibility",
                (if visible { "hidden" } else { "everyone" },).to_variant(),
            );
        }
    });
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let zone = drop.clone();
    target.connect_enter(move |_, _, _| {
        zone.add_css_class("drag-active");
        gdk::DragAction::COPY
    });
    let zone = drop.clone();
    target.connect_leave(move |_| {
        zone.remove_css_class("drag-active");
    });
    let weak = Rc::downgrade(&ui);
    target.connect_drop(move |_, value, _, _| {
        if let (Some(ui), Ok(files)) = (weak.upgrade(), value.get::<gdk::FileList>()) {
            ui.add_files(files.files().into_iter().collect());
            true
        } else {
            false
        }
    });
    drop.add_controller(target);

    for (name, page_name) in [("settings", "settings"), ("choose", "choose")] {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(&ui);
        action.connect_activate(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                if page_name == "choose" {
                    ui.choose_files();
                } else {
                    ui.stack.set_visible_child_name(page_name);
                }
            }
        });
        ui.window.add_action(&action);
    }
    let action = gio::SimpleAction::new("page", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(&ui);
    action.connect_activate(move |_, value| {
        if let (Some(ui), Some(name)) = (weak.upgrade(), value.and_then(|v| v.str())) {
            if name == "notch-drop" {
                ui.notch_drop();
            } else {
                ui.stack.set_visible_child_name(name);
            }
        }
    });
    ui.window.add_action(&action);
    let action = gio::SimpleAction::new("add-files", Some(glib::VariantTy::STRING_ARRAY));
    let weak = Rc::downgrade(&ui);
    action.connect_activate(move |_, value| {
        if let (Some(ui), Some(uris)) = (weak.upgrade(), value.and_then(|v| v.get::<Vec<String>>()))
        {
            ui.add_files(uris.iter().map(|uri| gio::File::for_uri(uri)).collect());
        }
    });
    ui.window.add_action(&action);
    ui.add_files(initial_files);
    ui.render_peers();
    ui.render_transfers();
    if initial_page == "notch-drop" {
        ui.notch_drop();
    } else {
        ui.window.present();
    }
    let owned = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        match ipc::connect().await {
            Ok(proxy) => {
                let weak = Rc::downgrade(&owned);
                proxy.connect_local("g-signal", false, move |values| {
                    if values[2].get::<String>().ok().as_deref() == Some("Changed") {
                        if let Some(ui) = weak.upgrade() {
                            ui.refresh();
                        }
                    }
                    None
                });
                *owned.proxy.borrow_mut() = Some(proxy);
                owned.refresh();
            }
            Err(error) => owned.service_error(&error),
        }
    });
    // Keep the state with the window lifetime. Reconnect catches service activation/restarts.
    let owned = ui.clone();
    glib::timeout_add_seconds_local(3, move || {
        if owned
            .window
            .application()
            .is_none_or(|app| app.windows().is_empty())
        {
            return glib::ControlFlow::Break;
        }
        owned.refresh();
        glib::ControlFlow::Continue
    });
}

impl Ui {
    fn notch_drop(self: &Rc<Self>) {
        let Some(app) = self.window.application() else {
            return;
        };
        if let Some(existing) = app
            .windows()
            .iter()
            .find(|w| w.title().as_deref() == Some("LinuxDrop Drop Surface"))
        {
            existing.present();
            return;
        }
        let surface = gtk::ApplicationWindow::builder()
            .application(&app)
            .title(tr("LinuxDrop Drop Surface"))
            .decorated(false)
            .resizable(false)
            .default_width(350)
            .default_height(180)
            .build();
        surface.add_css_class("notch-surface");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let heading = label("Drop files here", "drop-title");
        let detail = label("Then choose a nearby device", "compact-note");
        let icon = gtk::Image::from_icon_name("document-send-symbolic");
        icon.set_pixel_size(30);
        icon.add_css_class("brand-mark");
        content.append(&icon);
        content.append(&heading);
        content.append(&detail);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let cancel = gtk::Button::with_label(&tr("Close"));
        cancel.add_css_class("flat");
        actions.append(&cancel);
        let next = gtk::Button::with_label(&tr("Choose files"));
        next.add_css_class("suggested-action");
        next.set_hexpand(true);
        actions.append(&next);
        content.append(&actions);
        surface.set_child(Some(&content));
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        let heading_copy = heading.clone();
        let detail_copy = detail.clone();
        let next_copy = next.clone();
        target.connect_drop(move |_, value, _, _| {
            let (Some(ui), Ok(files)) = (weak.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            ui.add_files(files.files().into_iter().collect());
            let count = ui.files.borrow().len();
            heading_copy.set_label(&if german() {
                format!(
                    "{count} {} bereit",
                    if count == 1 { "Datei" } else { "Dateien" }
                )
            } else {
                format!(
                    "{count} {} ready",
                    if count == 1 { "file" } else { "files" }
                )
            });
            detail_copy.set_label(&tr("Your files are ready. Choose who to share with."));
            next_copy.set_label(&tr("Choose device"));
            true
        });
        surface.add_controller(target);
        let copy = surface.clone();
        cancel.connect_clicked(move |_| {
            copy.close();
        });
        let weak = Rc::downgrade(self);
        let copy = surface.clone();
        next.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.window.present();
                ui.stack.set_visible_child_name("send");
                if ui.files.borrow().is_empty() {
                    ui.choose_files();
                }
            }
            copy.close();
        });
        let keys = gtk::EventControllerKey::new();
        let copy = surface.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                copy.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        surface.add_controller(keys);
        surface.present();
    }
    pub fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(&tr(message)));
    }
    fn service_error(&self, error: &str) {
        self.connection
            .set_label(&tr("Sharing service unavailable. Retrying automatically…"));
        self.connection.set_tooltip_text(Some(error));
        self.connection.add_css_class("service-error");
        self.status.set_label(&tr("Offline"));
        self.send.set_sensitive(false);
    }
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let existing = ui.proxy.borrow().clone();
            let proxy = match existing {
                Some(proxy) => Ok(proxy),
                None => ipc::connect().await,
            };
            let result = match proxy {
                Ok(proxy) => {
                    *ui.proxy.borrow_mut() = Some(proxy.clone());
                    ipc::json(&proxy, "GetSnapshot").await
                }
                Err(error) => Err(error),
            };
            ui.refreshing.set(false);
            match result {
                Ok(snapshot) => {
                    let revision = format!("{}:{}", snapshot["epoch"], snapshot["revision"]);
                    let settings_value = snapshot.get("settings").cloned().unwrap_or(Value::Null);
                    *ui.settings.borrow_mut() = settings_value.clone();
                    *ui.snapshot.borrow_mut() = snapshot;
                    ui.connection.remove_css_class("service-error");
                    let name = settings_value["general"]["device_name"]
                        .as_str()
                        .unwrap_or("This device");
                    ui.connection.set_label(&if german() {
                        format!("Sichtbarer Gerätename: {name}")
                    } else {
                        format!("Sharing as {name}")
                    });
                    ui.connection.set_tooltip_text(None);
                    let visible = settings_value["visibility"]["mode"].as_str() == Some("everyone");
                    ui.status
                        .set_label(&tr(if visible { "● Visible" } else { "○ Hidden" }));
                    if *ui.revision.borrow() != revision {
                        *ui.revision.borrow_mut() = revision;
                        ui.render_peers();
                        ui.render_transfers();
                        ui.render_hardware();
                    }
                    if *ui.rendered_settings.borrow() != settings_value {
                        *ui.rendered_settings.borrow_mut() = settings_value.clone();
                        settings::render(&ui, &settings_value);
                    }
                    let appearance = settings_value["general"]["appearance"]
                        .as_str()
                        .unwrap_or("system");
                    adw::StyleManager::default().set_color_scheme(match appearance {
                        "dark" => adw::ColorScheme::ForceDark,
                        "light" => adw::ColorScheme::ForceLight,
                        _ => adw::ColorScheme::Default,
                    });
                    ui.update_send();
                }
                Err(error) => {
                    ui.service_error(&error);
                    *ui.proxy.borrow_mut() = None;
                }
            }
        });
    }
    pub fn mutate(self: &Rc<Self>, method: &str, params: glib::Variant) {
        let Some(proxy) = self.proxy.borrow().clone() else {
            self.toast("The sharing service is not connected yet");
            return;
        };
        let ui = self.clone();
        let method = method.to_owned();
        glib::MainContext::default().spawn_local(async move {
            if let Err(error) = ipc::call(&proxy, &method, Some(params)).await {
                ui.toast(&error);
                *ui.rendered_settings.borrow_mut() = Value::Null;
            }
            ui.refresh();
        });
    }
    pub fn choose_files(self: &Rc<Self>) {
        let ui = self.clone();
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose files to share"))
            .accept_label(tr("Add files"))
            .build();
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let Ok(files) = result {
                ui.add_files(
                    (0..files.n_items())
                        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                        .collect(),
                );
            }
        });
    }
    pub fn add_files(self: &Rc<Self>, files: Vec<gio::File>) {
        let mut invalid = 0;
        {
            let mut current = self.files.borrow_mut();
            for file in files {
                if file.path().is_none() {
                    invalid += 1;
                    continue;
                }
                if !current.iter().any(|f| f == &file) {
                    current.push(file);
                }
            }
        }
        if invalid > 0 {
            self.toast("Remote files must be downloaded locally before sharing");
        }
        self.render_files();
        self.update_send();
    }
    fn render_files(self: &Rc<Self>) {
        clear(&self.file_box);
        let empty = self.files.borrow().is_empty();
        for widget in &self.drop_details {
            widget.set_visible(empty);
        }
        if empty {
            self.drop_zone.remove_css_class("has-files");
        } else {
            self.drop_zone.add_css_class("has-files");
        }
        self.choose_button.set_label(&tr(if empty {
            "Choose files"
        } else {
            "Add more files"
        }));
        for file in self.files.borrow().iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("file-row");
            row.append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
            let name = file
                .basename()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let title = label(&name, "");
            title.set_hexpand(true);
            title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            title.set_wrap(false);
            row.append(&title);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.set_tooltip_text(Some("Remove file"));
            let weak = Rc::downgrade(self);
            let file = file.clone();
            remove.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.files.borrow_mut().retain(|f| f != &file);
                    ui.render_files();
                    ui.update_send();
                }
            });
            row.append(&remove);
            self.file_box.append(&row);
        }
    }
    fn update_send(&self) {
        let count = self.files.borrow().len();
        let selected = self.selected.borrow().clone();
        let peers = array(&self.snapshot.borrow(), "peers");
        let peer = peers.iter().find(|p| {
            Some(text(p, "id").to_owned()) == selected && p["available"].as_bool().unwrap_or(true)
        });
        self.send.set_sensitive(
            count > 0 && peer.is_some() && self.proxy.borrow().is_some() && !self.busy.get(),
        );
        self.share_link
            .set_sensitive(count > 0 && self.proxy.borrow().is_some() && !self.busy.get());
        self.send.set_label(&tr(if self.busy.get() {
            "Preparing…"
        } else {
            "Send files"
        }));
        self.send_caption.set_label(&match (count, peer) {
            (0, Some(peer)) => {
                if german() {
                    format!("Dateien für {} hinzufügen", text(peer, "name"))
                } else {
                    format!("Add files to send to {}", text(peer, "name"))
                }
            }
            (0, None) => tr("Choose files and a nearby device"),
            (count, Some(peer)) if german() => format!(
                "{count} {} an {}",
                if count == 1 { "Datei" } else { "Dateien" },
                text(peer, "name")
            ),
            (count, None) if german() => format!(
                "{count} {} bereit · Gerät auswählen",
                if count == 1 { "Datei" } else { "Dateien" }
            ),
            (count, Some(peer)) => format!(
                "{count} {} to {}",
                if count == 1 { "file" } else { "files" },
                text(peer, "name")
            ),
            (count, None) => format!(
                "{count} {} ready · choose a device",
                if count == 1 { "file" } else { "files" }
            ),
        });
    }
    fn render_peers(self: &Rc<Self>) {
        clear(&self.peers);
        let peers = array(&self.snapshot.borrow(), "peers");
        if peers.is_empty() {
            let empty = padded(12, 8, 7);
            empty.append(&label("No devices nearby yet", "section-heading"));
            empty.append(&label("Open a sharing app on your other device. LocalSend and Quick Share work on the same local network.", "empty-text"));
            self.peers.append(&empty);
        }
        for peer in peers {
            let id = text(&peer, "id").to_owned();
            let button = gtk::Button::new();
            button.add_css_class("peer-card");
            if self.selected.borrow().as_ref() == Some(&id) {
                button.add_css_class("selected");
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            let icon = gtk::Image::from_icon_name(match text(&peer, "platform") {
                "android" | "ios" | "phone" => "phone-symbolic",
                _ => "computer-symbolic",
            });
            icon.set_pixel_size(26);
            icon.add_css_class("device-icon");
            row.append(&icon);
            let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
            details.set_hexpand(true);
            details.append(&label(text(&peer, "name"), "heading"));
            let protocols: Vec<String> = array(&peer, "protocols")
                .iter()
                .filter_map(|v| v.as_str().map(|s| protocol_name(s).to_owned()))
                .collect();
            details.append(&label(&protocols.join(" · "), "protocol-badge"));
            row.append(&details);
            row.append(&gtk::Image::from_icon_name(
                if self.selected.borrow().as_ref() == Some(&id) {
                    "object-select-symbolic"
                } else {
                    "go-next-symbolic"
                },
            ));
            button.set_child(Some(&row));
            let weak = Rc::downgrade(self);
            let peer_copy = peer.clone();
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    *ui.selected.borrow_mut() = Some(id.clone());
                    let mut names = vec![tr("Automatic")];
                    names.extend(
                        array(&peer_copy, "protocols")
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| protocol_name(s).to_owned())),
                    );
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    ui.protocol.set_model(Some(&gtk::StringList::new(&names)));
                    ui.protocol.set_selected(0);
                    ui.render_peers();
                    ui.update_send();
                }
            });
            self.peers.append(&button);
        }
    }
    fn start_send(self: &Rc<Self>) {
        let Some(proxy) = self.proxy.borrow().clone() else {
            return;
        };
        let Some(peer) = self.selected.borrow().clone() else {
            return;
        };
        if self.busy.replace(true) {
            return;
        }
        let paths: Vec<String> = self
            .files
            .borrow()
            .iter()
            .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
            .collect();
        let chosen = self.protocol.selected();
        let protocol = if chosen == 0 {
            "auto".to_owned()
        } else {
            array(&self.snapshot.borrow(), "peers")
                .iter()
                .find(|p| text(p, "id") == peer)
                .and_then(|p| {
                    array(p, "protocols")
                        .get(chosen as usize - 1)
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "auto".into())
        };
        self.update_send();
        let ui = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let draft = ipc::string_result(
                    ipc::call(&proxy, "PrepareSend", Some((paths,).to_variant())).await?,
                )?;
                ipc::call(
                    &proxy,
                    "StartSend",
                    Some((draft, peer, protocol).to_variant()),
                )
                .await
            }
            .await;
            ui.busy.set(false);
            match result {
                Ok(_) => {
                    ui.files.borrow_mut().clear();
                    ui.render_files();
                    ui.stack.set_visible_child_name("transfers");
                    ui.refresh();
                }
                Err(error) => ui.toast(&error),
            }
            ui.update_send();
        });
    }
    fn share_link(self: &Rc<Self>) {
        if self.files.borrow().is_empty() {
            self.toast("Select files first");
            return;
        }
        let dialog = adw::AlertDialog::builder().heading(tr("Create a download link"))
            .body(tr("This link shares your selected files over unencrypted HTTP on your local network. Give the PIN only to the intended recipient.")).build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("create", &tr("Create link"))]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "create" {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(proxy) = ui.proxy.borrow().clone() else {
                return;
            };
            let paths: Vec<String> = ui
                .files
                .borrow()
                .iter()
                .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
                .collect();
            glib::MainContext::default().spawn_local(async move {
                let result: Result<Value, String> = async {
                    let draft = ipc::string_result(
                        ipc::call(&proxy, "PrepareSend", Some((paths,).to_variant())).await?,
                    )?;
                    let result = ipc::string_result(
                        ipc::call(&proxy, "CreateDownloadOffer", Some((draft,).to_variant()))
                            .await?,
                    )?;
                    serde_json::from_str(&result).map_err(|error| error.to_string())
                }
                .await;
                match result {
                    Err(error) => ui.toast(&error),
                    Ok(offer) => ui.show_link(&offer),
                }
            });
        });
        dialog.present(Some(&self.window));
    }
    fn show_link(self: &Rc<Self>, offer: &Value) {
        let url = offer["url"].as_str().unwrap_or("").to_owned();
        let pin = offer["pin"].as_str().unwrap_or("").to_owned();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let link = label(&url, "");
        link.set_selectable(true);
        link.set_wrap(true);
        content.append(&link);
        let code = label(&pin, "verification-code");
        code.set_selectable(true);
        content.append(&code);
        let duration = offer["expires_in"].as_u64().unwrap_or(0);
        content.append(&label(
            &if german() {
                format!("Gültig für {duration} Sekunden · nur im lokalen Netzwerk")
            } else {
                format!("Expires in {duration} seconds · local network only")
            },
            "compact-note",
        ));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (name, value, notice) in [
            ("Copy link", url, "Link copied"),
            ("Copy PIN", pin, "PIN copied"),
        ] {
            let button = gtk::Button::with_label(&tr(name));
            button.set_hexpand(true);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.window.clipboard().set_text(&value);
                    ui.toast(notice);
                }
            });
            actions.append(&button);
        }
        content.append(&actions);
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Share with a link"))
            .extra_child(&content)
            .build();
        dialog.add_responses(&[("stop", &tr("Stop sharing"))]);
        dialog.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("stop");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "stop" {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate("StopDownloadOffer", ().to_variant());
                }
            }
        });
        dialog.present(Some(&self.window));
    }
    fn render_transfers(self: &Rc<Self>) {
        clear(&self.transfers);
        let transfers = array(&self.snapshot.borrow(), "transfers");
        if transfers.is_empty() {
            let empty = adw::StatusPage::builder()
                .icon_name("folder-download-symbolic")
                .title(tr("Nothing on the move"))
                .description(tr("Incoming requests and your transfers will appear here."))
                .build();
            self.transfers.append(&empty);
        }
        for transfer in transfers.iter().rev() {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
            card.add_css_class("transfer-card");
            let state = text(transfer, "state");
            let incoming = matches!(text(transfer, "direction"), "incoming" | "receive");
            let title = format!(
                "{} {}",
                if incoming { "↓" } else { "↑" },
                text(transfer, "peer_name")
            );
            let heading = label(&title, "section-heading");
            card.append(&heading);
            let files = array(transfer, "files");
            let names: Vec<&str> = files.iter().map(|f| text(f, "name")).collect();
            card.append(&label(&names.join(", "), "compact-note"));
            let total = transfer["total_bytes"].as_u64().unwrap_or(0);
            let done = transfer["transferred_bytes"].as_u64().unwrap_or(0);
            let status = match state {
                "waiting" if incoming => "Wants to share with you",
                "waiting" => "Waiting for the other device",
                "verification" => "Compare this code on both devices",
                "transferring" => "Transferring",
                "completed" => "Completed",
                "cancelled" => "Cancelled",
                "rejected" => "Declined",
                "failed" => "Transfer failed",
                _ => state,
            };
            card.append(&label(
                &format!(
                    "{} · {}",
                    protocol_name(text(transfer, "protocol")),
                    tr(status)
                ),
                "protocol-badge",
            ));
            if let Some(code) = transfer["verification_code"]
                .as_str()
                .filter(|_| state == "verification")
            {
                card.append(&label(code, "verification-code"));
            }
            if state == "transferring" || state == "completed" {
                let progress = gtk::ProgressBar::new();
                progress.set_fraction(if state == "completed" {
                    1.0
                } else if total > 0 {
                    (done as f64 / total as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                });
                card.append(&progress);
                card.append(&label(
                    &if german() {
                        format!("{} von {}", bytes(done), bytes(total))
                    } else {
                        format!("{} of {}", bytes(done), bytes(total))
                    },
                    "compact-note",
                ));
            }
            if let Some(error) = transfer["error"].as_str() {
                let message = label(error, "error");
                message.set_selectable(true);
                card.append(&message);
            }
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            actions.set_halign(gtk::Align::End);
            let pending = (state == "waiting" && incoming) || state == "verification";
            if pending {
                self.transfer_button(
                    &actions,
                    "Decline",
                    "RejectTransfer",
                    text(transfer, "id"),
                    false,
                );
                self.transfer_button(
                    &actions,
                    if state == "verification" {
                        "Codes match"
                    } else {
                        "Accept"
                    },
                    "AcceptTransfer",
                    text(transfer, "id"),
                    true,
                );
            } else if !matches!(state, "completed" | "failed" | "cancelled" | "rejected") {
                self.transfer_button(
                    &actions,
                    "Cancel",
                    "CancelTransfer",
                    text(transfer, "id"),
                    false,
                );
            }
            if state == "completed" {
                if let Some(path) = array(transfer, "saved_paths")
                    .first()
                    .and_then(Value::as_str)
                {
                    let file = gio::File::for_path(path);
                    let open = gtk::Button::with_label(&tr("Open file"));
                    let weak = Rc::downgrade(self);
                    open.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            let launcher = gtk::FileLauncher::new(Some(&file));
                            launcher.launch(Some(&ui.window), gio::Cancellable::NONE, move |_| {});
                        }
                    });
                    actions.append(&open);
                }
            }
            card.append(&actions);
            self.transfers.append(&card);
        }
    }
    fn transfer_button(
        self: &Rc<Self>,
        container: &gtk::Box,
        title: &str,
        method: &str,
        id: &str,
        primary: bool,
    ) {
        let button = gtk::Button::with_label(&tr(title));
        if primary {
            button.add_css_class("suggested-action");
        }
        let weak = Rc::downgrade(self);
        let method = method.to_owned();
        let id = id.to_owned();
        button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.mutate(&method, (id.clone(),).to_variant());
            }
        });
        container.append(&button);
    }
    fn render_hardware(self: &Rc<Self>) {
        clear(&self.hardware);
        let snapshot = self.snapshot.borrow();
        for backend in array(&snapshot, "backends") {
            let detail = text(&backend, "detail");
            let summary = if text(&backend, "id") == "quickshare"
                && text(&backend, "state") == "ready"
                && detail.contains("Bluetooth unavailable")
            {
                tr("Local network sharing is ready. Bluetooth discovery is unavailable.")
            } else {
                tr(detail)
            };
            let row = adw::ActionRow::builder()
                .title(protocol_name(text(&backend, "id")))
                .subtitle(summary)
                .build();
            row.set_tooltip_text(Some(detail));
            row.add_prefix(&gtk::Image::from_icon_name(
                if matches!(text(&backend, "state"), "ready" | "running") {
                    "emblem-ok-symbolic"
                } else {
                    "dialog-information-symbolic"
                },
            ));
            row.add_suffix(&label(text(&backend, "state"), "protocol-badge"));
            let group = adw::PreferencesGroup::new();
            group.add(&row);
            self.hardware.append(&group);
        }
        self.hardware
            .append(&label("Detected hardware", "section-heading"));
        let inventory = &snapshot["hardware"];
        let mut any = false;
        for (key, title, icon) in [
            ("radios", "Wireless adapters", "network-wireless-symbolic"),
            ("interfaces", "Network interfaces", "network-wired-symbolic"),
            ("bluetooth", "Bluetooth controllers", "bluetooth-symbolic"),
        ] {
            let items = array(inventory, key);
            if items.is_empty() {
                continue;
            }
            any = true;
            let group = adw::PreferencesGroup::builder().title(tr(title)).build();
            for item in items {
                let name = ["name", "interface", "id"]
                    .iter()
                    .find_map(|k| item[*k].as_str())
                    .unwrap_or("Adapter");
                let row = adw::ExpanderRow::builder()
                    .title(name)
                    .subtitle(tr(item["driver"]
                        .as_str()
                        .unwrap_or("Detected by the system")))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name(icon));
                if let Some(fields) = item.as_object() {
                    for (key, value) in fields {
                        let title = match key.as_str() {
                            "driver" => "Driver",
                            "bus" => "Connection type",
                            "state" => "State",
                            "default_route" => "Internet connection",
                            "active_connection" => "Network name",
                            "nm_managed" => "Managed by NetworkManager",
                            "rfkill" => "Radio blocked",
                            "monitor" => "Monitor mode",
                            "management_injection" => "Management frame injection",
                            "data_injection" => "Data frame injection",
                            "awdl" => "AWDL compatibility",
                            "protected" => "Protected from interruption",
                            "powered" => "Powered on",
                            "supported_advertisements" => "Bluetooth advertisement capacity",
                            "active_advertisements" => "Active advertisements",
                            "modes" => "Supported radio modes",
                            _ => continue,
                        };
                        let content = match value {
                            Value::Null => tr("Unknown"),
                            Value::Bool(value) => tr(if *value { "Yes" } else { "No" }),
                            Value::String(s) => tr(s),
                            Value::Array(items) => items
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                            Value::Object(_) => format!(
                                "{} · {}",
                                tr(value["value"].as_str().unwrap_or("unknown")),
                                value["evidence"].as_str().unwrap_or("")
                            ),
                            _ => value.to_string(),
                        };
                        let detail = adw::ActionRow::builder()
                            .title(tr(title))
                            .subtitle(content)
                            .build();
                        detail.set_subtitle_selectable(true);
                        row.add_row(&detail);
                    }
                }
                group.add(&row);
            }
            self.hardware.append(&group);
        }
        if !any {
            self.hardware.append(&label("No radio adapters detected. Local network sharing can still work through Ethernet or the virtual machine network.","empty-text"));
        }
        if array(inventory, "radios").is_empty() {
            self.hardware
                .append(&label("No wireless adapters detected", "compact-note"));
        }
        if array(inventory, "bluetooth").is_empty() {
            self.hardware
                .append(&label("No Bluetooth controllers detected", "compact-note"));
        }
        self.hardware.append(&label("Hardware checks are passive. Your active Internet connection stays protected; monitor mode is never enabled just because a USB adapter is inserted.","compact-note"));
    }
}
