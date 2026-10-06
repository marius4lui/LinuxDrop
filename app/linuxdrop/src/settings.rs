use crate::i18n::tr;
use crate::ui::{clear, label, Ui};
use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::{json, Value};
use std::rc::Rc;

struct Field {
    key: &'static str,
    title: &'static str,
    detail: &'static str,
    kind: Kind,
}
enum Kind {
    Text,
    Secret,
    Interfaces,
    Folder,
    Toggle,
    Number(f64, f64, f64),
    Choice(&'static [(&'static str, &'static str)]),
}

const fn field(key: &'static str, title: &'static str, detail: &'static str, kind: Kind) -> Field {
    Field {
        key,
        title,
        detail,
        kind,
    }
}

pub fn render(ui: &Rc<Ui>, config: &Value) {
    let mut focused = gtk::prelude::GtkWindowExt::focus(&ui.window);
    let mut focus_name = None;
    while let Some(widget) = focused {
        if widget.widget_name().starts_with("setting:") {
            focus_name = Some(widget.widget_name().to_string());
            break;
        }
        focused = widget.parent();
    }
    clear(&ui.settings_body);
    ui.settings_body.append(&label("Settings", "hero-title"));
    ui.settings_body.append(&label(
        "Device, receiving, network, and desktop preferences.",
        "hero-subtitle",
    ));
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search settings"))
        .build();
    search.set_text(&ui.settings_query.borrow());
    ui.settings_body.append(&search);
    let category = gtk::DropDown::from_strings(&[&tr("All categories")]);
    category.update_property(&[gtk::accessible::Property::Label(&tr("Settings category"))]);
    ui.settings_body.append(&category);
    let sections: &[(&str, &str, &str, &[Field])] = &[
        (
            "general",
            "General",
            "Your identity and the way LinuxDrop feels",
            &[
                field("language", "Language", "Applies when the app next opens", Kind::Choice(&[("system","System"),("de","Deutsch"),("en","English")])),
                field("close_behavior", "When closing the window", "Background keeps sharing available; quit when idle never interrupts active transfers", Kind::Choice(&[("background","Keep running in background"),("quit_when_idle","Quit when no transfers are active")])),
                Field {
                    key: "device_name",
                    title: "Device name",
                    detail: "The name nearby devices see",
                    kind: Kind::Text,
                },
                Field {
                    key: "appearance",
                    title: "Appearance",
                    detail: "Follow your desktop or choose a theme",
                    kind: Kind::Choice(&[
                        ("system", "System"),
                        ("light", "Light"),
                        ("dark", "Dark"),
                    ]),
                },
                Field {
                    key: "autostart",
                    title: "Start at login",
                    detail: "Keep LinuxDrop ready in the background",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "receive",
            "Receiving",
            "Files are saved only after you accept a request",
            &[
                field("ask_directory", "Choose a folder for each request", "Review the destination before accepting incoming files", Kind::Toggle),
                field("collision_policy", "Existing filenames", "Never overwrite an existing file", Kind::Choice(&[("rename","Save with a new name"),("reject","Reject conflicting files")])),
                field("subfolders", "Organize received files", "Use separate folders for the sender or receiving date", Kind::Choice(&[("none","No subfolders"),("sender","By sender"),("date","By date"),("sender_date","By sender and date")])),
                Field {
                    key: "directory",
                    title: "Save files to",
                    detail: "Choose a local folder for received files",
                    kind: Kind::Folder,
                },
                Field {
                    key: "open_after",
                    title: "Open received files",
                    detail: "Automatically open files from requests you accept",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "open_folder",
                    title: "Show the destination folder",
                    detail: "Open the folder after a completed transfer",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "max_files",
                    title: "Maximum files per request",
                    detail: "Reject requests above this limit",
                    kind: Kind::Number(1.0, 10000.0, 1.0),
                },
                Field {
                    key: "max_bytes",
                    title: "Maximum request size (MB)",
                    detail: "Total uncompressed size allowed in one request",
                    kind: Kind::Number(1.0, 10_000_000.0, 100.0),
                },
            ],
        ),
        (
            "visibility",
            "Visibility",
            "Control when other devices can find you",
            &[
                Field {
                    key: "mode",
                    title: "Discoverable by",
                    detail: "Hidden devices can still send files",
                    kind: Kind::Choice(&[("hidden", "Nobody"), ("everyone", "Everyone nearby")]),
                },
                Field {
                    key: "duration_minutes",
                    title: "Visibility window (minutes)",
                    detail: "How long a temporary public session lasts",
                    kind: Kind::Number(1.0, 1440.0, 1.0),
                },
                Field {
                    key: "hide_on_lock",
                    title: "Hide when the screen locks",
                    detail: "Keep requests and device details private",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "localsend",
            "LocalSend",
            "Share with the LocalSend app on any platform",
            &[
                field("require_pin", "Require a receiving PIN", "LocalSend senders must enter your PIN before transferring", Kind::Toggle),
                field("pin", "Receiving PIN", "Keep this PIN private; diagnostics never include it", Kind::Secret),
                Field {
                    key: "enabled",
                    title: "Enable LocalSend",
                    detail: "Discover and share on your local network",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "https",
                    title: "Encrypted connections",
                    detail: "Use HTTPS for file transfers",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "multicast",
                    title: "Multicast discovery",
                    detail: "Find devices automatically on the local network",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "port",
                    title: "Network port",
                    detail: "Change only when your network requires it",
                    kind: Kind::Number(1024.0, 65535.0, 1.0),
                },
            ],
        ),
        (
            "quickshare",
            "Quick Share",
            "Nearby Share is now called Quick Share",
            &[
                field("port", "Quick Share listening port", "A fixed local TCP port for nearby connections", Kind::Number(1024.0,65535.0,1.0)),
                Field {
                    key: "enabled",
                    title: "Enable Quick Share",
                    detail: "Share with compatible Android and Windows devices",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "ble",
                    title: "Bluetooth discovery",
                    detail: "Advertise nearby when a supported controller is available",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "airdrop",
            "AirDrop · Experimental",
            "Availability depends on supported wireless hardware",
            &[
                field("send", "Allow AirDrop sending", "Send files through the selected AWDL adapter", Kind::Toggle),
                field("receive", "Allow AirDrop receiving", "Receive only after confirming each request", Kind::Toggle),
                Field {
                    key: "enabled",
                    title: "Enable experimental AirDrop",
                    detail: "Use the configured AWDL adapter for Apple devices",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "ble_wakeup",
                    title: "Bluetooth wake-up",
                    detail: "Help nearby Apple devices become discoverable",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "hardware",
            "Hardware",
            "Keep existing connections safe",
            &[
                field("auto_use_usb", "Use suitable USB adapters automatically", "Select available hardware after passive detection; never run an active injection test automatically", Kind::Toggle),
                field("open_on_adapter", "Open Hardware when an adapter arrives", "Show new wireless hardware without changing an active transfer", Kind::Toggle),
                Field {
                    key: "preferred_adapter",
                    title: "Preferred adapter",
                    detail: "Stable adapter ID; leave empty for automatic selection",
                    kind: Kind::Text,
                },
                Field {
                    key: "prefer_usb",
                    title: "Prefer dedicated USB adapters",
                    detail: "Choose a suitable free USB radio when available",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "protect_active_connection",
                    title: "Protect the active connection",
                    detail: "Prevent wireless sharing from interrupting Internet access",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "notifications",
            "Notifications",
            "Stay informed without unnecessary interruptions",
            &[
                field("incoming", "Incoming requests", "Show system notifications for requests and verification codes", Kind::Toggle),
                field("sound", "Notification sounds", "Use your desktop notification sound preference", Kind::Toggle),
                field("private_content", "Keep notification contents private", "Hide sender names and filenames in system notifications", Kind::Toggle),
                Field {
                    key: "completed",
                    title: "Completed transfers",
                    detail: "Show a notification when your files arrive",
                    kind: Kind::Toggle,
                },
                Field {
                    key: "errors",
                    title: "Transfer problems",
                    detail: "Let you know when a transfer needs attention",
                    kind: Kind::Toggle,
                },
            ],
        ),
        (
            "transfers",
            "Transfers and history",
            "Limits apply to new requests",
            &[
                field("bandwidth_limit_mbps", "Bandwidth limit (Mbit/s)", "Shared by all transfers; zero means unlimited", Kind::Number(0.0,100000.0,1.0)),
                field("history_days", "Keep history for days", "Zero keeps records until the count limit or manual deletion", Kind::Number(0.0,3650.0,1.0)),
                Field {
                    key: "max_parallel",
                    title: "Simultaneous transfers",
                    detail: "Maximum number of active transfers",
                    kind: Kind::Number(1.0, 16.0, 1.0),
                },
                Field {
                    key: "history_limit",
                    title: "Recent transfers to keep",
                    detail: "Limit local transfer history",
                    kind: Kind::Number(0.0, 1000.0, 1.0),
                },
            ],
        ),
        ("bluetooth", "Bluetooth", "Controller and nearby advertising", &[
            field("adapter", "Bluetooth controller", "BlueZ controller name, for example hci0; empty selects automatically", Kind::Text),
        ]),
        ("network", "Advanced network", "Choose LAN interfaces for LocalSend and Quick Share; AirDrop uses its dedicated adapter", &[
            field("allowed_interfaces", "Allowed interfaces", "Comma-separated interface names; empty chooses suitable local interfaces automatically", Kind::Interfaces),
            field("allow_virtual_interfaces", "Allow virtual and VPN interfaces", "Only enable this when you intend to announce to those networks", Kind::Toggle),
        ]),
        ("diagnostics", "Advanced diagnostics", "Operational details without file contents or private keys", &[
            field("log_level", "Logging detail", "Detailed logs can contain operational context; review before sharing", Kind::Choice(&[("warn","Warnings and errors"),("info","Normal"),("debug","Detailed")])),
        ]),
    ];
    let mut search_groups: Vec<(adw::PreferencesGroup, Vec<(gtk::Widget, String)>)> = Vec::new();
    for (section, title, description, fields) in sections {
        let group = adw::PreferencesGroup::builder()
            .title(tr(title))
            .description(tr(description))
            .build();
        let mut rows = Vec::new();
        for field in *fields {
            let Some(value) = config.get(section).and_then(|s| s.get(field.key)) else {
                continue;
            };
            let draft_key = format!("{section}.{}", field.key);
            if field.key == "protect_active_connection" {
                let row = adw::ActionRow::builder()
                    .title(tr(field.title))
                    .subtitle(tr(field.detail))
                    .build();
                row.add_suffix(&label("Always enabled", "compact-note"));
                group.add(&row);
                rows.push((row.upcast(), tr(field.title).to_lowercase()));
                continue;
            }
            let widget: gtk::Widget = match &field.kind {
                Kind::Toggle => {
                    let row = adw::SwitchRow::builder()
                        .title(tr(field.title))
                        .subtitle(tr(field.detail))
                        .active(value.as_bool().unwrap_or(false))
                        .build();
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    row.connect_active_notify(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            update(&ui, &section, &key, json!(row.is_active()));
                        }
                    });
                    row.upcast()
                }
                Kind::Choice(choices) => {
                    let translated: Vec<String> =
                        choices.iter().map(|(_, name)| tr(name)).collect();
                    let names: Vec<&str> = translated.iter().map(String::as_str).collect();
                    let selected = choices
                        .iter()
                        .position(|(key, _)| value.as_str() == Some(key))
                        .unwrap_or(0) as u32;
                    let row = adw::ComboRow::builder()
                        .title(tr(field.title))
                        .subtitle(tr(field.detail))
                        .model(&gtk::StringList::new(&names))
                        .selected(selected)
                        .build();
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    let choices = *choices;
                    row.connect_selected_notify(move |row| {
                        if let (Some(ui), Some((value, _))) =
                            (weak.upgrade(), choices.get(row.selected() as usize))
                        {
                            update(&ui, &section, &key, json!(value));
                        }
                    });
                    row.upcast()
                }
                Kind::Number(min, max, step) => {
                    let scale = if field.key == "max_bytes" {
                        1_000_000.0
                    } else {
                        1.0
                    };
                    let persisted = value.as_f64().unwrap_or(*min) / scale;
                    if ui
                        .settings_drafts
                        .borrow()
                        .get(&draft_key)
                        .and_then(|text| text.parse::<f64>().ok())
                        == Some(persisted)
                    {
                        ui.settings_drafts.borrow_mut().remove(&draft_key);
                    }
                    let initial = ui
                        .settings_drafts
                        .borrow()
                        .get(&draft_key)
                        .and_then(|text| text.parse::<f64>().ok())
                        .unwrap_or(persisted);
                    let adjustment =
                        gtk::Adjustment::new(initial, *min, *max, *step, *step * 10.0, 0.0);
                    let row = adw::SpinRow::builder()
                        .title(tr(field.title))
                        .subtitle(tr(field.detail))
                        .adjustment(&adjustment)
                        .digits(0)
                        .build();
                    let weak = Rc::downgrade(ui);
                    let draft_key = draft_key.clone();
                    row.connect_value_notify(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            ui.settings_drafts
                                .borrow_mut()
                                .insert(draft_key.clone(), row.value().to_string());
                        }
                    });
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    // Commit on loss of focus, avoiding a backend restart for every keystroke.
                    let controller = gtk::EventControllerFocus::new();
                    let row_copy = row.clone();
                    controller.connect_leave(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            update(
                                &ui,
                                &section,
                                &key,
                                json!((row_copy.value() * scale) as u64),
                            );
                        }
                    });
                    row.add_controller(controller);
                    row.upcast()
                }
                Kind::Text | Kind::Interfaces => {
                    let interfaces = matches!(field.kind, Kind::Interfaces);
                    let current = if interfaces {
                        value
                            .as_array()
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default()
                    } else {
                        value.as_str().unwrap_or("").to_owned()
                    };
                    if ui.settings_drafts.borrow().get(&draft_key) == Some(&current) {
                        ui.settings_drafts.borrow_mut().remove(&draft_key);
                    }
                    let initial = ui
                        .settings_drafts
                        .borrow()
                        .get(&draft_key)
                        .cloned()
                        .unwrap_or(current);
                    let row = adw::EntryRow::builder()
                        .title(tr(field.title))
                        .text(initial)
                        .show_apply_button(true)
                        .build();
                    row.set_tooltip_text(Some(&tr(field.detail)));
                    let weak = Rc::downgrade(ui);
                    let draft_key = draft_key.clone();
                    row.connect_changed(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            ui.settings_drafts
                                .borrow_mut()
                                .insert(draft_key.clone(), row.text().to_string());
                        }
                    });
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    row.connect_apply(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            let value = if interfaces {
                                json!(row
                                    .text()
                                    .split(',')
                                    .map(str::trim)
                                    .filter(|name| !name.is_empty())
                                    .collect::<Vec<_>>())
                            } else {
                                json!(row.text().as_str())
                            };
                            update(&ui, &section, &key, value);
                        }
                    });
                    row.upcast()
                }
                Kind::Secret => {
                    let current = value.as_str().unwrap_or("").to_owned();
                    if ui.settings_drafts.borrow().get(&draft_key) == Some(&current) {
                        ui.settings_drafts.borrow_mut().remove(&draft_key);
                    }
                    let initial = ui
                        .settings_drafts
                        .borrow()
                        .get(&draft_key)
                        .cloned()
                        .unwrap_or(current);
                    let row = adw::PasswordEntryRow::builder()
                        .title(tr(field.title))
                        .text(initial)
                        .show_apply_button(true)
                        .build();
                    row.set_tooltip_text(Some(&tr(field.detail)));
                    let weak = Rc::downgrade(ui);
                    let draft_key = draft_key.clone();
                    row.connect_changed(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            ui.settings_drafts
                                .borrow_mut()
                                .insert(draft_key.clone(), row.text().to_string());
                        }
                    });
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    row.connect_apply(move |row| {
                        if let Some(ui) = weak.upgrade() {
                            update(&ui, &section, &key, json!(row.text().as_str()));
                        }
                    });
                    row.upcast()
                }
                Kind::Folder => {
                    let row = adw::ActionRow::builder()
                        .title(tr(field.title))
                        .subtitle(value.as_str().unwrap_or("Choose a folder"))
                        .build();
                    let button = gtk::Button::from_icon_name("folder-open-symbolic");
                    button.set_valign(gtk::Align::Center);
                    button.set_tooltip_text(Some(&tr(field.detail)));
                    row.add_suffix(&button);
                    row.set_activatable_widget(Some(&button));
                    let weak = Rc::downgrade(ui);
                    let section = section.to_string();
                    let key = field.key.to_owned();
                    button.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            let dialog = gtk::FileDialog::builder()
                                .title(tr("Choose a receiving folder"))
                                .build();
                            let owned = ui.clone();
                            let section = section.clone();
                            let key = key.clone();
                            dialog.select_folder(
                                Some(&ui.window),
                                gio::Cancellable::NONE,
                                move |result| {
                                    if let Ok(file) = result {
                                        if let Some(path) = file.path() {
                                            update(
                                                &owned,
                                                &section,
                                                &key,
                                                json!(path.to_string_lossy()),
                                            );
                                        } else {
                                            owned.toast("Choose a local folder");
                                        }
                                    }
                                },
                            );
                        }
                    });
                    row.upcast()
                }
            };
            widget.set_widget_name(&format!("setting:{draft_key}"));
            group.add(&widget);
            rows.push((
                widget,
                format!("{} {} {}", tr(title), tr(field.title), tr(field.detail)).to_lowercase(),
            ));
        }
        if !rows.is_empty() {
            if *section == "transfers" {
                let row = adw::ActionRow::builder()
                    .title(tr("Clear transfer history"))
                    .subtitle(tr("Received files remain in their folder"))
                    .build();
                let button = gtk::Button::from_icon_name("user-trash-symbolic");
                button.set_tooltip_text(Some(&tr("Clear transfer history")));
                button.set_valign(gtk::Align::Center);
                row.add_suffix(&button);
                row.set_activatable_widget(Some(&button));
                let weak = Rc::downgrade(ui);
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        let dialog = adw::AlertDialog::builder().heading(tr("Clear transfer history?")).body(tr("This removes completed transfer records. Active transfers and received files are kept.")).build();
                        dialog.add_responses(&[("cancel", &tr("Cancel")), ("clear", &tr("Clear history"))]);
                        dialog.set_close_response("cancel");
                        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
                        let weak = Rc::downgrade(&ui);
                        dialog.connect_response(None, move |_, response| { if response == "clear" { if let Some(ui) = weak.upgrade() { let Some(proxy)=ui.proxy.borrow().clone() else{return;}; glib::MainContext::default().spawn_local(async move {match crate::ipc::call(&proxy,"ClearHistory",None).await {Ok(_)=>{ui.toast("Transfer history cleared");ui.refresh();},Err(error)=>ui.toast(&error)}}); } } });
                        dialog.present(Some(&ui.window));
                    }
                });
                group.add(&row);
                rows.push((row.upcast(), tr("Clear transfer history").to_lowercase()));
            }
            ui.settings_body.append(&group);
            search_groups.push((group, rows));
        }
    }
    let desktop = adw::PreferencesGroup::builder()
        .title(tr("Desktop notch"))
        .description(tr(
            "Position, monitor, and appearance live in the GNOME extension preferences.",
        ))
        .build();
    let open = adw::ActionRow::builder()
        .title(tr("Notch preferences"))
        .subtitle(tr("Requires the LinuxDrop GNOME Shell extension"))
        .build();
    let button = gtk::Button::from_icon_name("preferences-system-symbolic");
    button.set_tooltip_text(Some(&tr("Notch preferences")));
    button.set_valign(gtk::Align::Center);
    open.add_suffix(&button);
    open.set_activatable_widget(Some(&button));
    let weak = Rc::downgrade(ui);
    button.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            match gio::Subprocess::newv(
                &[
                    std::ffi::OsStr::new("gnome-extensions"),
                    std::ffi::OsStr::new("prefs"),
                    std::ffi::OsStr::new("linuxdrop@marius4lui.github.io"),
                ],
                gio::SubprocessFlags::NONE,
            ) {
                Ok(_) => {}
                Err(_) => ui.toast("Install and enable the LinuxDrop GNOME extension first"),
            }
        }
    });
    desktop.add(&open);
    ui.settings_body.append(&desktop);
    search_groups.push((
        desktop,
        vec![(
            open.upcast(),
            format!(
                "{} {} {}",
                tr("Desktop notch"),
                tr("Notch preferences"),
                tr("Position, monitor, and appearance live in the GNOME extension preferences.")
            )
            .to_lowercase(),
        )],
    ));
    let diagnostics = adw::PreferencesGroup::builder()
        .title(tr("About and diagnostics"))
        .build();
    let about = adw::ActionRow::builder()
        .title(tr("LinuxDrop"))
        .subtitle(format!(
            "Version {} · Native nearby sharing",
            env!("CARGO_PKG_VERSION")
        ))
        .build();
    diagnostics.add(&about);
    let row = adw::ActionRow::builder()
        .title(tr("Copy diagnostic report"))
        .subtitle(tr(
            "Service status and capability information; review before sharing",
        ))
        .build();
    let button = gtk::Button::from_icon_name("edit-copy-symbolic");
    button.set_tooltip_text(Some(&tr("Copy diagnostic report")));
    button.set_valign(gtk::Align::Center);
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    let weak = Rc::downgrade(ui);
    button.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            let proxy = ui.proxy.borrow().clone();
            if let Some(proxy) = proxy {
                glib::MainContext::default().spawn_local(async move {
                    match crate::ipc::json(&proxy, "GetDiagnostics").await {
                        Ok(report) => {
                            ui.window.clipboard().set_text(
                                &serde_json::to_string_pretty(&report).unwrap_or_default(),
                            );
                            ui.toast("Diagnostic report copied");
                        }
                        Err(error) => ui.toast(&error),
                    }
                });
            }
        }
    });
    diagnostics.add(&row);
    let mut diagnostic_rows = crate::diagnostics::add_actions(ui, &diagnostics);
    ui.settings_body.append(&diagnostics);
    diagnostic_rows.extend([
        (about.upcast(), tr("About and diagnostics").to_lowercase()),
        (
            row.upcast(),
            format!(
                "{} {}",
                tr("Copy diagnostic report"),
                tr("Service status and capability information; review before sharing")
            )
            .to_lowercase(),
        ),
    ]);
    search_groups.push((diagnostics, diagnostic_rows));
    let names: Vec<String> = std::iter::once(tr("All categories"))
        .chain(
            search_groups
                .iter()
                .map(|(group, _)| group.title().to_string()),
        )
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    category.set_model(Some(&gtk::StringList::new(&names)));
    category.set_selected(ui.settings_category.get().min(search_groups.len() as u32));
    let groups = Rc::new(search_groups);
    let filter: Rc<dyn Fn()> = {
        let groups = groups.clone();
        let search = search.downgrade();
        let category = category.downgrade();
        let weak = Rc::downgrade(ui);
        Rc::new(move || {
            let (Some(search), Some(category)) = (search.upgrade(), category.upgrade()) else {
                return;
            };
            let query = search.text().to_lowercase();
            let selected = category.selected();
            if let Some(ui) = weak.upgrade() {
                *ui.settings_query.borrow_mut() = search.text().to_string();
                ui.settings_category.set(selected);
            }
            for (index, (group, rows)) in groups.iter().enumerate() {
                let mut visible = false;
                for (row, text) in rows {
                    let matches =
                        text.contains(&query) && (selected == 0 || selected == index as u32 + 1);
                    row.set_visible(matches);
                    visible |= matches;
                }
                group.set_visible(visible);
            }
        })
    };
    filter();
    let filter_copy = filter.clone();
    search.connect_search_changed(move |_| filter_copy());
    category.connect_selected_notify(move |_| filter());
    if let Some(name) = focus_name {
        crate::ui::restore_focus(&ui.settings_body, &name);
    }
}

fn update(ui: &Rc<Ui>, section: &str, key: &str, value: Value) {
    if ui.settings.borrow()[section][key] == value {
        return;
    }
    let patch = json!({ section: { key: value } });
    ui.mutate("UpdateSettings", (patch.to_string(),).to_variant());
}
