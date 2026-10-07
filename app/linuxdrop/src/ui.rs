use crate::i18n::{german, tr};
use crate::{ipc, settings};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    file_scroll: gtk::ScrolledWindow,
    drop_zone: gtk::Box,
    drop_details: Vec<gtk::Widget>,
    choose_button: gtk::Button,
    files: RefCell<Vec<gio::File>>,
    file_checks: RefCell<HashMap<String, FileCheck>>,
    selected: RefCell<Option<String>>,
    protocol: gtk::DropDown,
    protocol_ids: RefCell<Vec<String>>,
    selected_protocol: RefCell<String>,
    send: gtk::Button,
    share_link: gtk::Button,
    send_caption: gtk::Label,
    transfers: gtk::Box,
    download_link: adw::PreferencesGroup,
    stop_link: gtk::Button,
    stopping_link: Cell<bool>,
    hardware: gtk::Box,
    pub settings_body: gtk::Box,
    pub settings_status: adw::PreferencesGroup,
    settings_status_row: adw::ActionRow,
    settings_status_details: gtk::Button,
    busy: Cell<bool>,
    refreshing: Cell<bool>,
    refresh_again: Cell<bool>,
    service_generation: Cell<u64>,
    settings_generation: Cell<u64>,
    service_ready: Cell<bool>,
    remote_transfer_actions: RefCell<Vec<gtk::Box>>,
    service_dialogs: RefCell<Vec<glib::WeakRef<adw::AlertDialog>>>,
    closing: Cell<bool>,
    allow_close: Cell<bool>,
    pub settings_query: RefCell<String>,
    pub settings_category: Cell<u32>,
    pub settings_drafts: RefCell<HashMap<String, String>>,
    pub settings_writes: RefCell<HashMap<String, settings::WriteState>>,
    pub settings_rendering: Cell<bool>,
    pub settings_resetting: Cell<bool>,
    rendered_settings: RefCell<Value>,
    revision: RefCell<String>,
    rendered_peers: RefCell<Value>,
    rendered_hardware: RefCell<Value>,
    rendered_transfer_structure: RefCell<Value>,
    transfer_progress: RefCell<HashMap<String, (gtk::ProgressBar, gtk::Label)>>,
}

#[derive(Clone)]
enum FileCheck {
    Checking,
    Ready(u64),
    Invalid(String),
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

pub fn restore_focus(container: &impl IsA<gtk::Widget>, name: &str) -> bool {
    let widget = container.as_ref();
    if widget.widget_name() == name {
        return widget.grab_focus();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if restore_focus(&current, name) {
            return true;
        }
        child = current.next_sibling();
    }
    false
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

pub fn build(app: &adw::Application, initial_page: &str, initial_files: Vec<gio::File>) -> Rc<Ui> {
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
    status.set_tooltip_text(Some(&tr("Change who can discover this device")));
    header.pack_end(&status);
    toolbar.add_top_bar(&header);
    let switcher = adw::ViewSwitcherBar::builder()
        .stack(&stack)
        .reveal(true)
        .build();
    toolbar.add_bottom_bar(&switcher);
    let top_switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    let wide = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("min-width: 760sp").expect("Static breakpoint"),
    );
    wide.add_setter(&header, "title-widget", Some(&top_switcher.to_value()));
    wide.add_setter(&switcher, "reveal", Some(&false.to_value()));
    window.add_breakpoint(wide);
    toolbar.set_content(Some(&stack));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    let body = padded(16, 20, 12);
    let intro = gtk::Box::new(gtk::Orientation::Vertical, 7);
    intro.append(&label("Send files", "hero-title"));
    body.append(&intro);
    let connection = label("Connecting to the sharing service…", "compact-note");
    body.append(&connection);

    let drop = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    drop.add_css_class("drop-zone");
    let icon = gtk::Image::from_icon_name("document-send-symbolic");
    icon.set_pixel_size(28);
    icon.set_halign(gtk::Align::Center);
    icon.add_css_class("drop-icon");
    drop.append(&icon);
    let title = label("Drop files here", "drop-title");
    title.set_hexpand(true);
    drop.append(&title);
    let choose = gtk::Button::with_label(&tr("Choose files"));
    choose.add_css_class("pill");
    choose.set_halign(gtk::Align::Center);
    drop.append(&choose);
    body.append(&drop);
    let file_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let file_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(120)
        .child(&file_box)
        .visible(false)
        .build();
    body.append(&file_scroll);

    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let nearby = label("Nearby", "section-heading");
    nearby.set_hexpand(true);
    heading.append(&nearby);
    let manage = gtk::Button::from_icon_name("system-users-symbolic");
    manage.add_css_class("flat");
    manage.set_tooltip_text(Some(&tr("Manage devices")));
    heading.append(&manage);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some(&tr("Refresh nearby devices")));
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
    protocol.set_tooltip_text(Some(&tr("Transfer protocol")));
    protocol.update_property(&[gtk::accessible::Property::Label(&tr("Transfer protocol"))]);
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
    send_page.append(
        &adw::Clamp::builder()
            .maximum_size(760)
            .tightening_threshold(560)
            .child(&send_bar)
            .build(),
    );
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
    let receive_link = gtk::Button::with_label(&tr("Receive from a link"));
    receive_link.set_halign(gtk::Align::Start);
    receive_link.add_css_class("pill");
    transfer_page.append(&receive_link);
    let download_link = adw::PreferencesGroup::new();
    download_link.set_visible(false);
    let link_row = adw::ActionRow::builder()
        .title(tr("Download link is active"))
        .subtitle(tr("Stopping the link also cancels its active downloads."))
        .subtitle_lines(0)
        .build();
    let stop_link = gtk::Button::with_label(&tr("Stop sharing"));
    stop_link.set_valign(gtk::Align::Center);
    stop_link.set_tooltip_text(Some(&tr("Stop sharing this download link")));
    link_row.add_suffix(&stop_link);
    download_link.add(&link_row);
    transfer_page.append(&download_link);
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
    let settings_status = adw::PreferencesGroup::new();
    settings_status.set_widget_name("settings:service-status");
    settings_status.set_visible(false);
    let settings_status_row = adw::ActionRow::builder().subtitle_lines(0).build();
    settings_status_row.add_prefix(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    let settings_status_details = gtk::Button::with_label(&tr("Details"));
    settings_status_details.set_valign(gtk::Align::Center);
    settings_status_details.set_tooltip_text(Some(&tr("Show sharing service details")));
    settings_status_row.add_suffix(&settings_status_details);
    settings_status.add(&settings_status_row);
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
        file_scroll,
        drop_zone: drop.clone(),
        drop_details: vec![icon.upcast(), title.upcast()],
        choose_button: choose.clone(),
        files: RefCell::new(Vec::new()),
        file_checks: RefCell::new(HashMap::new()),
        selected: RefCell::new(None),
        protocol,
        protocol_ids: RefCell::new(vec!["auto".to_owned()]),
        selected_protocol: RefCell::new("auto".to_owned()),
        send,
        share_link,
        send_caption,
        transfers,
        download_link,
        stop_link,
        stopping_link: Cell::new(false),
        hardware,
        settings_body,
        settings_status,
        settings_status_row,
        settings_status_details,
        busy: Cell::new(false),
        refreshing: Cell::new(false),
        refresh_again: Cell::new(false),
        service_generation: Cell::new(0),
        settings_generation: Cell::new(0),
        service_ready: Cell::new(false),
        remote_transfer_actions: RefCell::new(Vec::new()),
        service_dialogs: RefCell::new(Vec::new()),
        closing: Cell::new(false),
        allow_close: Cell::new(false),
        settings_query: RefCell::new(String::new()),
        settings_category: Cell::new(0),
        settings_drafts: RefCell::new(HashMap::new()),
        settings_writes: RefCell::new(HashMap::new()),
        settings_rendering: Cell::new(false),
        settings_resetting: Cell::new(false),
        rendered_settings: RefCell::new(Value::Null),
        revision: RefCell::new(String::new()),
        rendered_peers: RefCell::new(Value::Null),
        rendered_hardware: RefCell::new(Value::Null),
        rendered_transfer_structure: RefCell::new(Value::Null),
        transfer_progress: RefCell::new(HashMap::new()),
    });

    let weak = Rc::downgrade(&ui);
    ui.stop_link.connect_clicked(move |_| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(proxy) = ui.proxy.borrow().clone() else {
            ui.toast("The sharing service is not connected yet");
            return;
        };
        ui.stop_link.set_sensitive(false);
        ui.stopping_link.set(true);
        glib::MainContext::default().spawn_local(async move {
            if let Err(error) = ipc::call(&proxy, "StopDownloadOffer", Some(().to_variant())).await
            {
                ui.toast(&error);
            }
            ui.stopping_link.set(false);
            ui.refresh();
        });
    });
    let weak = Rc::downgrade(&ui);
    ui.settings_status_details.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.stack.set_visible_child_name("hardware");
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        let Some(ui) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if ui.allow_close.get()
            || ui.settings.borrow()["general"]["close_behavior"].as_str() != Some("quit_when_idle")
        {
            return glib::Propagation::Proceed;
        }
        if ui.closing.replace(true) {
            return glib::Propagation::Stop;
        }
        let Some(proxy) = ui.proxy.borrow().clone() else {
            ui.closing.set(false);
            ui.toast("The sharing service is not connected yet");
            return glib::Propagation::Stop;
        };
        glib::MainContext::default().spawn_local(async move {
            match ipc::call(&proxy, "StopWhenIdle", Some(().to_variant())).await {
                Ok(_) => {
                    ui.allow_close.set(true);
                    ui.window.close();
                }
                Err(error) => {
                    ui.closing.set(false);
                    ui.toast(&error);
                }
            }
        });
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(&ui);
    manage.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.manage_devices();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.protocol.connect_selected_notify(move |dropdown| {
        if let Some(ui) = weak.upgrade() {
            if let Some(id) = ui.protocol_ids.borrow().get(dropdown.selected() as usize) {
                *ui.selected_protocol.borrow_mut() = id.clone();
            }
            ui.update_send();
        }
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
    receive_link.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.receive_link(None);
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
    ui.refresh();
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
    ui
}

#[cfg(test)]
#[path = "native_regressions.rs"]
mod native_regressions;

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
                    "{count} {} hinzugefügt",
                    if count == 1 { "Datei" } else { "Dateien" }
                )
            } else {
                format!(
                    "{count} {} added",
                    if count == 1 { "file" } else { "files" }
                )
            });
            detail_copy.set_label(&tr("Review your files and choose a device."));
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
    pub fn show_transfers(&self) {
        self.stack.set_visible_child_name("transfers");
    }
    fn service_error(&self, error: &str) {
        self.service_ready.set(false);
        self.status.set_sensitive(false);
        for actions in self.remote_transfer_actions.borrow().iter() {
            actions.set_sensitive(false);
        }
        for dialog in self.service_dialogs.borrow_mut().drain(..) {
            if let Some(dialog) = dialog.upgrade() {
                dialog.force_close();
            }
        }
        self.connection
            .set_label(&tr("Sharing service unavailable. Retrying automatically…"));
        self.connection.set_tooltip_text(Some(error));
        self.connection.add_css_class("service-error");
        self.status.set_label(&tr("Offline"));
        self.send.set_sensitive(false);
        self.share_link.set_sensitive(false);
        self.stop_link.set_sensitive(false);
        self.settings_status.set_visible(true);
        self.settings_status_details.set_visible(false);
        self.settings_status_row
            .set_title(&tr("Sharing service is offline"));
        self.settings_status_row.set_subtitle(&tr(
            "The last known settings are shown. LinuxDrop will reconnect automatically.",
        ));
    }
    pub fn track_service_dialog(&self, dialog: &adw::AlertDialog) {
        self.service_dialogs
            .borrow_mut()
            .retain(|dialog| dialog.upgrade().is_some());
        self.service_dialogs.borrow_mut().push(dialog.downgrade());
    }
    pub fn service_generation(&self) -> u64 {
        self.service_generation.get()
    }
    pub fn service_is_ready(&self) -> bool {
        self.service_ready.get()
    }
    fn update_settings_status(&self) {
        let snapshot = self.snapshot.borrow();
        let applying = snapshot["restarting"] == true;
        let affected: Vec<_> = array(&snapshot, "backends")
            .into_iter()
            .filter(|backend| {
                snapshot["settings"][text(backend, "id")]["enabled"] == true
                    && matches!(text(backend, "state"), "error" | "unavailable")
            })
            .map(|backend| protocol_name(text(&backend, "id")).to_owned())
            .collect();
        self.settings_status
            .set_visible(applying || !affected.is_empty());
        self.settings_status_details
            .set_visible(!applying && !affected.is_empty());
        if applying {
            self.settings_status_row
                .set_title(&tr("Applying sharing settings"));
            self.settings_status_row.set_subtitle(&tr(
                "Your choices are saved. Sharing resumes when the services are ready.",
            ));
        } else if !affected.is_empty() {
            self.settings_status_row
                .set_title(&tr("Sharing needs attention"));
            self.settings_status_row.set_subtitle(&format!("{}: {}", affected.join(", "), tr("Your choices are saved, but these services are not ready. Check their status before sending.")));
        }
    }
    fn install_proxy(self: &Rc<Self>, proxy: &gio::DBusProxy) {
        // Every new proxy subscribes before its first snapshot, including reconnects.
        let weak = Rc::downgrade(self);
        let installed = proxy.downgrade();
        proxy.connect_local("g-signal", false, move |values| {
            if values[2].get::<String>().ok().as_deref() == Some("Changed") {
                if let Some(ui) = weak.upgrade() {
                    if installed.upgrade().as_ref() == ui.proxy.borrow().as_ref() {
                        ui.refresh();
                    }
                }
            }
            None
        });
        let weak = Rc::downgrade(self);
        proxy.connect_notify_local(Some("g-name-owner"), move |proxy, _| {
            if let Some(ui) = weak.upgrade() {
                if ui.proxy.borrow().as_ref() != Some(proxy) {
                    return;
                }
                ui.service_generation
                    .set(ui.service_generation.get().wrapping_add(1));
                ui.revision.borrow_mut().clear();
                ui.service_error("Service owner changed");
                // A replacement owner must not wait for the old owner's reply.
                ui.refreshing.set(false);
                ui.refresh_again.set(false);
                ui.refresh();
            }
        });
        *self.proxy.borrow_mut() = Some(proxy.clone());
    }
    pub fn refresh(self: &Rc<Self>) {
        if self.refreshing.replace(true) {
            self.refresh_again.set(true);
            return;
        }
        let ui = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let existing = ui.proxy.borrow().clone();
            let proxy = match existing {
                Some(proxy) => Ok(proxy),
                None => match ipc::connect().await {
                    Ok(proxy) => { ui.install_proxy(&proxy); Ok(proxy) }
                    Err(error) => Err(error),
                },
            };
            let generation = ui.service_generation.get();
            let settings_generation = ui.settings_generation.get();
            let snapshot_proxy = proxy.as_ref().ok().cloned();
            let owner = snapshot_proxy.as_ref().and_then(|proxy| proxy.g_name_owner());
            let result = match proxy {
                Ok(proxy) => {
                    if owner.is_none() {
                        ui.service_error("Background service unavailable");
                    }
                    // Calling the well-known name also permits normal D-Bus activation.
                    // An owner appearing during this call invalidates its generation;
                    // the owner-change handler obtains the authoritative snapshot.
                    ipc::json(&proxy, "GetSnapshot").await
                }
                Err(error) => Err(error),
            };
            if generation != ui.service_generation.get()
                || snapshot_proxy.as_ref() != ui.proxy.borrow().as_ref()
                || snapshot_proxy.as_ref().and_then(|proxy| proxy.g_name_owner()) != owner
            {
                // Never clear a newer refresh flag or restore an old owner's consent.
                return;
            }
            ui.refreshing.set(false);
            if settings_generation != ui.settings_generation.get() {
                // A write completed after this read began. Read again before
                // publishing settings that predate a confirmed user choice.
                ui.refresh_again.set(false);
                ui.refresh();
                return;
            }
            match result {
                Ok(snapshot) => {
                    ui.service_ready.set(true);
                    ui.status.set_sensitive(true);
                    for actions in ui.remote_transfer_actions.borrow().iter() {
                        actions.set_sensitive(true);
                    }
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
                        let peers = ui.snapshot.borrow()["peers"].clone();
                        if *ui.rendered_peers.borrow() != peers {
                            *ui.rendered_peers.borrow_mut() = peers;
                            ui.render_peers();
                        }
                        let mut structure = ui.snapshot.borrow()["transfers"].clone();
                        if let Some(transfers) = structure.as_array_mut() {
                            for transfer in transfers {
                                if let Some(object) = transfer.as_object_mut() { object.remove("transferred_bytes"); }
                                if let Some(files) = transfer["files"].as_array_mut() {
                                    for file in files { if let Some(object) = file.as_object_mut() { object.remove("transferred"); } }
                                }
                            }
                        }
                        if *ui.rendered_transfer_structure.borrow() != structure {
                            *ui.rendered_transfer_structure.borrow_mut() = structure;
                            ui.render_transfers();
                        } else { ui.update_transfer_progress(); }
                        let mut hardware = ui.snapshot.borrow()["hardware"].clone();
                        if let Some(object) = hardware.as_object_mut() { object.remove("observed_unix"); }
                        let hardware_view = serde_json::json!({"hardware":hardware,"backends":ui.snapshot.borrow()["backends"]});
                        if *ui.rendered_hardware.borrow() != hardware_view {
                            *ui.rendered_hardware.borrow_mut() = hardware_view;
                            ui.render_hardware();
                        }
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
                    ui.download_link.set_visible(ui.snapshot.borrow()["download_link_active"] == true);
                    ui.stop_link.set_sensitive(!ui.stopping_link.get());
                    ui.update_settings_status();
                    ui.update_send();
                }
                Err(error) => {
                    ui.service_error(&error);
                }
            }
            if ui.refresh_again.replace(false) {
                // An event arrived while reading the snapshot: resync once, without
                // guessing whether its revision is already represented in the reply.
                glib::idle_add_local_once(move || ui.refresh());
            }
        });
    }
    pub fn invalidate_settings(&self) {
        *self.rendered_settings.borrow_mut() = Value::Null;
    }
    pub fn confirm_settings_write(&self) {
        self.settings_generation
            .set(self.settings_generation.get().wrapping_add(1));
        self.invalidate_settings();
    }
    pub fn mutate(self: &Rc<Self>, method: &str, params: glib::Variant) {
        if method == "ResetSettings"
            && (self.settings_resetting.get() || settings::writes_pending(self))
        {
            self.toast("Wait for the current settings change to finish");
            return;
        }
        if !self.service_ready.get() {
            self.toast("The sharing service is not connected yet");
            return;
        }
        let Some(proxy) = self.proxy.borrow().clone() else {
            self.toast("The sharing service is not connected yet");
            return;
        };
        let ui = self.clone();
        let method = method.to_owned();
        let generation = self.service_generation();
        let owner = proxy.g_name_owner();
        if method == "ResetSettings" {
            self.settings_resetting.set(true);
            self.settings_body.set_sensitive(false);
            self.toast("Restoring default settings…");
        }
        glib::MainContext::default().spawn_local(async move {
            if generation != ui.service_generation()
                || proxy.g_name_owner() != owner
                || !ui.service_is_ready()
            {
                if method == "ResetSettings" {
                    ui.settings_resetting.set(false);
                    ui.settings_body.set_sensitive(true);
                }
                return;
            }
            let result = ipc::call(&proxy, &method, Some(params)).await;
            if method == "ResetSettings" {
                ui.settings_resetting.set(false);
                ui.settings_body.set_sensitive(true);
            }
            if generation != ui.service_generation() || proxy.g_name_owner() != owner {
                return;
            }
            if let Err(error) = result {
                ui.toast(&error);
                *ui.rendered_settings.borrow_mut() = Value::Null;
            } else if method == "ResetSettings" {
                ui.settings_drafts.borrow_mut().clear();
                ui.settings_writes.borrow_mut().clear();
                ui.confirm_settings_write();
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
        let mut added = Vec::new();
        {
            let mut current = self.files.borrow_mut();
            for file in files {
                if !current.iter().any(|f| f == &file) {
                    self.file_checks
                        .borrow_mut()
                        .insert(file.uri().to_string(), FileCheck::Checking);
                    current.push(file.clone());
                    added.push(file);
                }
            }
        }
        self.render_files();
        self.update_send();
        for file in added {
            let ui = self.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = if file.path().is_none() {
                    FileCheck::Invalid(tr("Remote files must be downloaded locally before sharing"))
                } else {
                    match file
                        .query_info_future(
                            "standard::type,standard::size,access::can-read",
                            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                            glib::Priority::DEFAULT,
                        )
                        .await
                    {
                        Ok(info) if info.file_type() == gio::FileType::SymbolicLink => {
                            FileCheck::Invalid(tr(
                                "Select the original file instead of a symbolic link",
                            ))
                        }
                        Ok(info) if info.file_type() != gio::FileType::Regular => {
                            FileCheck::Invalid(tr(
                                "Select regular files; folders need to be imported first",
                            ))
                        }
                        Ok(info)
                            if info.has_attribute("access::can-read")
                                && !info.boolean("access::can-read") =>
                        {
                            FileCheck::Invalid(tr("You do not have permission to read this file"))
                        }
                        Ok(info) => FileCheck::Ready(info.size().max(0) as u64),
                        Err(error) => FileCheck::Invalid(error.to_string()),
                    }
                };
                if ui.files.borrow().iter().any(|current| current == &file) {
                    ui.file_checks
                        .borrow_mut()
                        .insert(file.uri().to_string(), result);
                    ui.render_files();
                    ui.update_send();
                }
            });
        }
    }
    fn render_files(self: &Rc<Self>) {
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window)
            .map(|widget| widget.widget_name().to_string());
        clear(&self.file_box);
        let empty = self.files.borrow().is_empty();
        self.file_scroll.set_visible(!empty);
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
            let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
            details.set_hexpand(true);
            details.append(&title);
            match self.file_checks.borrow().get(file.uri().as_str()) {
                Some(FileCheck::Ready(size)) => {
                    details.append(&label(&bytes(*size), "compact-note"))
                }
                Some(FileCheck::Invalid(reason)) => {
                    details.append(&label(reason, "error"));
                    row.add_css_class("invalid-file");
                }
                _ => details.append(&label("Checking file…", "compact-note")),
            }
            row.append(&details);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.set_tooltip_text(Some(&tr("Remove file")));
            remove.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}: {name}",
                tr("Remove file")
            ))]);
            remove.set_widget_name(&format!("remove:{}", file.uri()));
            let weak = Rc::downgrade(self);
            let file = file.clone();
            remove.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.files.borrow_mut().retain(|f| f != &file);
                    ui.file_checks.borrow_mut().remove(file.uri().as_str());
                    ui.render_files();
                    ui.update_send();
                }
            });
            row.append(&remove);
            self.file_box.append(&row);
        }
        if let Some(focus) = focus {
            restore_focus(&self.file_box, &focus);
        }
    }
    fn update_send(&self) {
        let count = self.files.borrow().len();
        let ready = self.files.borrow().iter().all(|file| {
            matches!(
                self.file_checks.borrow().get(file.uri().as_str()),
                Some(FileCheck::Ready(_))
            )
        });
        let connected = self
            .proxy
            .borrow()
            .as_ref()
            .is_some_and(|proxy| proxy.g_name_owner().is_some())
            && self.service_ready.get();
        let restarting = self.snapshot.borrow()["restarting"] == true;
        let selected = self.selected.borrow().clone();
        let peers = array(&self.snapshot.borrow(), "peers");
        let peer = peers.iter().find(|p| {
            Some(text(p, "id").to_owned()) == selected
                && p["available"].as_bool().unwrap_or(true)
                && !p["blocked"].as_bool().unwrap_or(false)
        });
        let chosen = self.selected_protocol.borrow().clone();
        let protocol_available = chosen == "auto"
            || peer.is_some_and(|peer| {
                array(peer, "protocols")
                    .iter()
                    .any(|id| id.as_str() == Some(&chosen))
            });
        self.send.set_sensitive(
            count > 0
                && ready
                && peer.is_some()
                && protocol_available
                && connected
                && !restarting
                && !self.busy.get(),
        );
        self.share_link
            .set_sensitive(count > 0 && ready && connected && !restarting && !self.busy.get());
        self.send.set_label(&tr(if self.busy.get() {
            "Preparing…"
        } else {
            "Send files"
        }));
        self.send_caption.set_label(&match (count, peer) {
            _ if restarting => tr("Sharing services are restarting; try again shortly"),
            _ if !connected => tr("Sharing service unavailable. Retrying automatically…"),
            (count, _) if count > 0 && !ready => tr("Check the marked files before sending"),
            (_, Some(_)) if !protocol_available => {
                tr("The selected protocol is unavailable; choose another protocol")
            }
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
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window)
            .map(|widget| widget.widget_name().to_string());
        clear(&self.peers);
        let mut peers = array(&self.snapshot.borrow(), "peers");
        peers.sort_by_key(|peer| {
            (
                !peer["favorite"].as_bool().unwrap_or(false),
                text(peer, "display_name").to_lowercase(),
                text(peer, "name").to_lowercase(),
            )
        });
        let selected = self.selected.borrow().clone();
        let mut ids = vec!["auto".to_owned()];
        if let Some(peer) = peers
            .iter()
            .find(|peer| Some(text(peer, "id")) == selected.as_deref())
        {
            ids.extend(
                array(peer, "protocols")
                    .iter()
                    .filter_map(|id| id.as_str().map(str::to_owned)),
            );
        }
        let chosen = self.selected_protocol.borrow().clone();
        let missing = !ids.contains(&chosen);
        if missing {
            ids.push(chosen.clone());
        }
        let names: Vec<String> = ids
            .iter()
            .map(|id| {
                if id == "auto" {
                    tr("Automatic")
                } else if missing && id == &chosen {
                    format!("{} · {}", protocol_name(id), tr("Unavailable"))
                } else {
                    protocol_name(id).to_owned()
                }
            })
            .collect();
        let old_names: Vec<String> = self
            .protocol
            .model()
            .and_downcast::<gtk::StringList>()
            .map(|model| {
                (0..model.n_items())
                    .filter_map(|index| model.string(index).map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if *self.protocol_ids.borrow() != ids || old_names != names {
            *self.protocol_ids.borrow_mut() = ids.clone();
            self.protocol.set_model(Some(&gtk::StringList::new(
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            )));
        }
        self.protocol
            .set_selected(ids.iter().position(|id| id == &chosen).unwrap_or(0) as u32);
        *self.selected_protocol.borrow_mut() = chosen;
        if peers.is_empty() {
            let empty = padded(12, 8, 7);
            empty.append(&label("No devices nearby yet", "section-heading"));
            empty.append(&label("Open a sharing app on your other device. LocalSend and Quick Share work on the same local network.", "empty-text"));
            self.peers.append(&empty);
        }
        for peer in peers {
            let id = text(&peer, "id").to_owned();
            let button = gtk::ToggleButton::new();
            button.set_widget_name(&format!("peer:{id}"));
            button.set_active(self.selected.borrow().as_ref() == Some(&id));
            button.set_sensitive(
                peer["available"].as_bool().unwrap_or(true)
                    && !peer["blocked"].as_bool().unwrap_or(false),
            );
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
            let display_name = peer["display_name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| text(&peer, "name"));
            details.append(&label(display_name, "heading"));
            if peer["favorite"].as_bool().unwrap_or(false) {
                row.prepend(&gtk::Image::from_icon_name("starred-symbolic"));
            }
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
                    *ui.selected_protocol.borrow_mut() = peer_copy["preferred_protocol"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .unwrap_or("auto")
                        .to_owned();
                    ui.render_peers();
                    ui.update_send();
                }
            });
            self.peers.append(&button);
        }
        if let Some(focus) = focus {
            restore_focus(&self.peers, &focus);
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
        let protocol = self.selected_protocol.borrow().clone();
        let submitted: Vec<String> = self
            .files
            .borrow()
            .iter()
            .map(|file| file.uri().to_string())
            .collect();
        self.update_send();
        let ui = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let draft = ipc::prepare_files(&proxy, paths).await?;
                let result = ipc::call(
                    &proxy,
                    "StartSend",
                    Some((draft.clone(), peer, protocol).to_variant()),
                )
                .await;
                if result.is_err() {
                    let _ = ipc::call(&proxy, "DiscardDraft", Some((draft,).to_variant())).await;
                }
                result
            }
            .await;
            ui.busy.set(false);
            match result {
                Ok(_) => {
                    // A user can add another batch while preparation is in flight.
                    // Only consume the exact identities submitted by this send.
                    ui.files
                        .borrow_mut()
                        .retain(|file| !submitted.contains(&file.uri().to_string()));
                    ui.file_checks
                        .borrow_mut()
                        .retain(|uri, _| !submitted.contains(uri));
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
                    let draft = ipc::prepare_files(&proxy, paths).await?;
                    let reply = ipc::call(
                        &proxy,
                        "CreateDownloadOffer",
                        Some((draft.clone(),).to_variant()),
                    )
                    .await;
                    if reply.is_err() {
                        let _ =
                            ipc::call(&proxy, "DiscardDraft", Some((draft,).to_variant())).await;
                    }
                    let result = ipc::string_result(reply?)?;
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
            .body(tr("Keep this window open while sharing. Closing it or pressing Escape immediately disables the link, including copied links."))
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
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window)
            .map(|widget| widget.widget_name().to_string());
        clear(&self.transfers);
        self.remote_transfer_actions.borrow_mut().clear();
        self.transfer_progress.borrow_mut().clear();
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
                "connecting" => "Opening download offer",
                "pin_required" if incoming => "Enter the sender's PIN",
                "pin_required" => "Enter the receiving device's PIN",
                "transferring" => "Transferring",
                "completed" => "Completed",
                "cancelled" => "Cancelled",
                "rejected" => "Declined",
                "failed" => "Transfer failed",
                _ => state,
            };
            card.append(&label(status, "transfer-state"));
            card.append(&label(
                &format!(
                    "{} · {}",
                    protocol_name(text(transfer, "protocol")),
                    bytes(total)
                ),
                "protocol-badge",
            ));
            if incoming && state == "waiting" {
                let destination = self.settings.borrow()["receive"]["directory"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                card.append(&label(
                    &format!("{}: {destination}", tr("Save files to")),
                    "compact-note",
                ));
            }
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
                progress.update_property(&[gtk::accessible::Property::Label(&format!(
                    "{}: {}",
                    tr("Transfer progress"),
                    text(transfer, "peer_name")
                ))]);
                let description = label(
                    &if german() {
                        format!("{} von {}", bytes(done), bytes(total))
                    } else {
                        format!("{} of {}", bytes(done), bytes(total))
                    },
                    "compact-note",
                );
                card.append(&description);
                self.transfer_progress
                    .borrow_mut()
                    .insert(text(transfer, "id").to_owned(), (progress, description));
            }
            if let Some(error) = transfer["error"].as_str() {
                let message = label(error, "error");
                message.set_selectable(true);
                card.append(&message);
            }
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            actions.set_halign(gtk::Align::End);
            if state != "completed" {
                actions.set_sensitive(self.service_ready.get());
                self.remote_transfer_actions
                    .borrow_mut()
                    .push(actions.clone());
            }
            let pending = (state == "waiting" && incoming) || state == "verification";
            if state == "pin_required" {
                let button = gtk::Button::with_label(&tr("Enter PIN"));
                button.add_css_class("suggested-action");
                let weak = Rc::downgrade(self);
                let request = transfer.clone();
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.provide_pin(&request);
                    }
                });
                actions.append(&button);
                self.transfer_button(
                    &actions,
                    "Cancel",
                    "CancelTransfer",
                    text(transfer, "id"),
                    false,
                );
            } else if pending {
                self.transfer_button(
                    &actions,
                    "Decline",
                    "RejectTransfer",
                    text(transfer, "id"),
                    false,
                );
                if state == "verification" && !incoming {
                    self.transfer_button(
                        &actions,
                        "Codes match",
                        "AcceptTransfer",
                        text(transfer, "id"),
                        true,
                    );
                } else {
                    let button = gtk::Button::with_label(&tr("Review and accept"));
                    button.add_css_class("suggested-action");
                    button.set_widget_name(&format!(
                        "transfer:{}:AcceptTransfer",
                        text(transfer, "id")
                    ));
                    let weak = Rc::downgrade(self);
                    let request = transfer.clone();
                    button.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.accept_request(&request);
                        }
                    });
                    actions.append(&button);
                }
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
                let paths: Vec<String> = array(transfer, "saved_paths")
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .collect();
                if let Some(path) = paths.first() {
                    let folder = gtk::Button::from_icon_name("folder-open-symbolic");
                    folder
                        .set_widget_name(&format!("transfer:{}:OpenFolder", text(transfer, "id")));
                    folder.set_tooltip_text(Some(&tr("Show the destination folder")));
                    folder.update_property(&[gtk::accessible::Property::Label(&tr(
                        "Show the destination folder",
                    ))]);
                    let weak = Rc::downgrade(self);
                    let path = path.clone();
                    folder.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.open_received_file(&path, true);
                        }
                    });
                    actions.append(&folder);
                    let open = gtk::Button::with_label(&tr(if paths.len() == 1 {
                        "Open file"
                    } else {
                        "Show received files"
                    }));
                    open.set_widget_name(&format!("transfer:{}:OpenFiles", text(transfer, "id")));
                    let weak = Rc::downgrade(self);
                    open.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            if paths.len() == 1 {
                                ui.open_received_file(&paths[0], false);
                            } else {
                                ui.received_files(&paths);
                            }
                        }
                    });
                    actions.append(&open);
                }
            }
            card.append(&actions);
            self.transfers.append(&card);
        }
        if let Some(focus) = focus {
            restore_focus(&self.transfers, &focus);
        }
    }
    fn update_transfer_progress(&self) {
        for transfer in array(&self.snapshot.borrow(), "transfers") {
            if let Some((progress, description)) =
                self.transfer_progress.borrow().get(text(&transfer, "id"))
            {
                let total = transfer["total_bytes"].as_u64().unwrap_or(0);
                let done = transfer["transferred_bytes"].as_u64().unwrap_or(0);
                progress.set_fraction(if text(&transfer, "state") == "completed" {
                    1.0
                } else if total > 0 {
                    (done as f64 / total as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                });
                description.set_label(&if german() {
                    format!("{} von {}", bytes(done), bytes(total))
                } else {
                    format!("{} of {}", bytes(done), bytes(total))
                });
            }
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
        button.set_widget_name(&format!("transfer:{id}:{method}"));
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
        // Inventory changes must not interrupt a user inspecting an adapter.
        fn expanded_rows(root: &gtk::Widget, expanded: &mut HashMap<String, bool>) {
            if let Some(row) = root.downcast_ref::<adw::ExpanderRow>() {
                expanded.insert(row.widget_name().to_string(), row.is_expanded());
            }
            let mut child = root.first_child();
            while let Some(widget) = child {
                expanded_rows(&widget, expanded);
                child = widget.next_sibling();
            }
        }
        let mut expanded = HashMap::new();
        expanded_rows(self.hardware.upcast_ref(), &mut expanded);
        let focus_name = gtk::prelude::GtkWindowExt::focus(&self.window)
            .filter(|widget| widget.is_ancestor(&self.hardware))
            .and_then(|mut widget| loop {
                let name = widget.widget_name();
                if name.starts_with("hardware:") {
                    break Some(name.to_string());
                }
                widget = widget.parent()?;
            });
        clear(&self.hardware);
        let snapshot = self.snapshot.borrow();
        for backend in array(&snapshot, "backends") {
            let detail = text(&backend, "detail");
            let summary = if detail.starts_with("Sharing restart could not finish:") {
                tr("Sharing could not restart. Retry in Settings after cleanup finishes.")
            } else if text(&backend, "id") == "quickshare"
                && text(&backend, "state") == "ready"
                && detail.contains("Bluetooth unavailable")
            {
                tr("Local network sharing is ready. Bluetooth discovery is unavailable.")
            } else {
                tr(detail)
            };
            let row = adw::ActionRow::builder().use_markup(false).build();
            row.set_title(protocol_name(text(&backend, "id")));
            row.set_subtitle(&summary);
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
                let row = adw::ExpanderRow::builder().use_markup(false).build();
                row.set_title(name);
                row.set_subtitle(&tr(item["driver"]
                    .as_str()
                    .unwrap_or("Detected by the system")));
                let identity = format!("hardware:{key}:{}", item["id"].as_str().unwrap_or(name));
                row.set_widget_name(&identity);
                row.set_expanded(expanded.get(&identity).copied().unwrap_or(false));
                row.add_prefix(&gtk::Image::from_icon_name(icon));
                if let Some(fields) = item.as_object() {
                    for (key, value) in fields {
                        let title = match key.as_str() {
                            "driver" => "Driver",
                            "bus" => "Connection type",
                            "vendor_id" => "Vendor ID",
                            "product_id" => "Product ID",
                            "serial" => "Serial number",
                            "kernel" => "Kernel version",
                            "bands" => "Wireless bands",
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
                            "reserved_for" => "Reserved for",
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
                        let detail = adw::ActionRow::builder().use_markup(false).build();
                        detail.set_title(&tr(title));
                        detail.set_subtitle(&content);
                        detail.set_subtitle_selectable(true);
                        row.add_row(&detail);
                    }
                }
                if key == "radios" {
                    let test = adw::ActionRow::builder()
                        .title(tr("Run active hardware test"))
                        .subtitle(tr(
                            "An explicit test; protected or busy adapters are refused",
                        ))
                        .build();
                    let button = gtk::Button::from_icon_name("system-run-symbolic");
                    button.set_valign(gtk::Align::Center);
                    button.set_tooltip_text(Some(&tr("Run active hardware test")));
                    button.set_widget_name(&format!("{identity}:test"));
                    button.update_property(&[gtk::accessible::Property::Label(&format!(
                        "{}: {name}",
                        tr("Run active hardware test")
                    ))]);
                    button.set_sensitive(
                        !item["protected"].as_bool().unwrap_or(false)
                            && !item["rfkill"].as_bool().unwrap_or(false)
                            && !matches!(
                                item["reserved_for"].as_str(),
                                Some("AirDrop" | "Quick Share")
                            ),
                    );
                    let id = text(&item, "id").to_owned();
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.run_hardware_diagnostic(&id);
                        }
                    });
                    test.add_suffix(&button);
                    test.set_activatable_widget(Some(&button));
                    row.add_row(&test);
                }
                if matches!(key, "radios" | "bluetooth") {
                    let prefer = adw::ActionRow::builder()
                        .title(tr("Use as preferred adapter"))
                        .subtitle(tr(
                            "Selection still respects availability and connection protection",
                        ))
                        .build();
                    let button = gtk::Button::from_icon_name("emblem-favorite-symbolic");
                    button.set_valign(gtk::Align::Center);
                    button.set_tooltip_text(Some(&tr("Use as preferred adapter")));
                    button.set_widget_name(&format!("{identity}:prefer"));
                    button.update_property(&[gtk::accessible::Property::Label(&format!(
                        "{}: {name}",
                        tr("Use as preferred adapter")
                    ))]);
                    let id = text(&item, "id").to_owned();
                    let kind = key.to_owned();
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            let patch = if kind == "radios" {
                                serde_json::json!({"hardware":{"preferred_adapter":id}})
                            } else {
                                serde_json::json!({"bluetooth":{"adapter":id}})
                            };
                            ui.mutate("UpdateSettings", (patch.to_string(),).to_variant());
                        }
                    });
                    prefer.add_suffix(&button);
                    prefer.set_activatable_widget(Some(&button));
                    row.add_row(&prefer);
                }
                let details = adw::ActionRow::builder()
                    .title(tr("Full adapter details"))
                    .subtitle(tr(
                        "Capabilities, evidence sources, channel limits and driver information",
                    ))
                    .build();
                let button = gtk::Button::from_icon_name("go-next-symbolic");
                button.set_valign(gtk::Align::Center);
                button.set_tooltip_text(Some(&tr("Full adapter details")));
                button.set_widget_name(&format!("{identity}:details"));
                button.update_property(&[gtk::accessible::Property::Label(&format!(
                    "{}: {name}",
                    tr("Full adapter details")
                ))]);
                let weak = Rc::downgrade(self);
                let report = item.clone();
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        crate::diagnostics::report_dialog(&ui, "Full adapter details", &report);
                    }
                });
                details.add_suffix(&button);
                details.set_activatable_widget(Some(&button));
                row.add_row(&details);
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
        if let Some(name) = focus_name {
            restore_focus(&self.hardware, &name);
        }
    }
}
