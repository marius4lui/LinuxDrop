use crate::{
    i18n::tr,
    ui::{bytes, label, Ui},
};
use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};

impl Ui {
    pub fn receive_link(self: &Rc<Self>, initial: Option<&str>) -> adw::AlertDialog {
        let entry = adw::EntryRow::builder()
            .title(tr("LocalSend link"))
            .text(initial.unwrap_or(""))
            .build();
        entry.set_widget_name("download-offer-url");
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&entry);
        let dialog = adw::AlertDialog::builder().heading(tr("Receive from a link"))
            .body(tr("Paste the sender's LocalSend link, such as http://192.168.1.20:53317. Download links use unencrypted HTTP on your local network. You will review the files and choose where to save them before downloading."))
            .extra_child(&list).build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("open", &tr("Review offer"))]);
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("open"));
        dialog.set_focus(Some(&entry));
        dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("open", !entry.text().trim().is_empty());
        let weak = dialog.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(dialog) = weak.upgrade() {
                dialog.set_response_enabled("open", !entry.text().trim().is_empty());
            }
        });
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "open" {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let url = entry.text().trim().to_owned();
            let Some(proxy) = ui.proxy.borrow().clone() else {
                ui.receive_link(Some(&url));
                ui.toast("Background service unavailable");
                return;
            };
            glib::MainContext::default().spawn_local(async move {
                match crate::ipc::call(
                    &proxy,
                    "ReceiveDownloadOffer",
                    Some((url.clone(),).to_variant()),
                )
                .await
                {
                    Ok(_) => {
                        ui.show_transfers();
                        ui.refresh();
                    }
                    Err(error) => {
                        ui.receive_link(Some(&url));
                        ui.toast(&error);
                    }
                }
            });
        });
        self.track_service_dialog(&dialog);
        dialog.present(Some(&self.window));
        dialog
    }
    pub fn accept_request(self: &Rc<Self>, transfer: &Value) -> adw::AlertDialog {
        self.review_request(transfer, None, None)
    }

    fn review_request(
        self: &Rc<Self>,
        transfer: &Value,
        previous: Option<&Value>,
        error: Option<&str>,
    ) -> adw::AlertDialog {
        let id = transfer["id"].as_str().unwrap_or_default().to_owned();
        let files = transfer["files"].as_array().cloned().unwrap_or_default();
        let partial = transfer["selection_mode"].as_str() == Some("native")
            || transfer["protocol"].as_str() == Some("localsend");
        let selected = Rc::new(RefCell::new(
            (0..files.len())
                .map(|index| {
                    previous
                        .and_then(|options| options["selected_indices"].as_array())
                        .is_none_or(|indices| {
                            indices
                                .iter()
                                .any(|value| value.as_u64() == Some(index as u64))
                        })
                })
                .collect::<Vec<_>>(),
        ));
        let settings = self.settings.borrow().clone();
        let folder = Rc::new(RefCell::new(
            previous
                .and_then(|options| options["directory"].as_str())
                .or_else(|| transfer["receive_directory"].as_str())
                .or_else(|| settings["receive"]["directory"].as_str())
                .unwrap_or_default()
                .to_owned(),
        ));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if let Some(error) = error {
            content.append(&label(error, "error"));
        }
        let verification = transfer["state"].as_str() == Some("verification");
        if verification {
            content.append(&label("Compare this code on both devices", "compact-note"));
            content.append(&label(
                transfer["verification_code"].as_str().unwrap_or_default(),
                "verification-code",
            ));
        }
        let description = format!(
            "{} · {}",
            transfer["peer_name"].as_str().unwrap_or_default(),
            bytes(transfer["total_bytes"].as_u64().unwrap_or(0))
        );
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Review incoming files"))
            .body(description)
            .extra_child(&content)
            .build();
        dialog.add_responses(&[
            ("cancel", &tr("Cancel")),
            (
                "accept",
                &tr(if verification {
                    "Codes match — accept"
                } else {
                    "Accept selected files"
                }),
            ),
        ]);
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("accept", selected.borrow().iter().any(|value| *value));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (index, file) in files.iter().enumerate() {
            let name = file["name"].as_str().unwrap_or_default();
            let description = format!("{name} · {}", bytes(file["size"].as_u64().unwrap_or(0)));
            let check = gtk::CheckButton::new();
            let filename = label(&description, "");
            filename.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            filename.set_lines(2);
            filename.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            filename.set_hexpand(true);
            check.set_child(Some(&filename));
            check.set_tooltip_text(Some(&description));
            check.update_property(&[gtk::accessible::Property::Label(&description)]);
            check.set_widget_name(&format!("incoming-file-{index}"));
            check.set_active(selected.borrow()[index]);
            let selected = selected.clone();
            let weak = dialog.downgrade();
            check.connect_toggled(move |check| {
                selected.borrow_mut()[index] = check.is_active();
                if let Some(dialog) = weak.upgrade() {
                    dialog.set_response_enabled(
                        "accept",
                        selected.borrow().iter().any(|value| *value),
                    );
                }
            });
            list.append(&check);
        }
        content.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .min_content_height((files.len().min(2) as i32 * 44).max(44))
                .max_content_height(240)
                .child(&list)
                .build(),
        );
        if !partial {
            content.append(&label(
                "The complete request is transferred; only selected files are saved",
                "compact-note",
            ));
        }
        let destination = adw::ActionRow::builder()
            .title(tr("Save files to"))
            .subtitle(folder.borrow().as_str())
            .build();
        destination.set_widget_name("incoming-destination");
        let choose = gtk::Button::from_icon_name("folder-open-symbolic");
        choose.set_valign(gtk::Align::Center);
        choose.set_tooltip_text(Some(&tr("Choose a receiving folder")));
        choose.update_property(&[gtk::accessible::Property::Label(&tr(
            "Choose a receiving folder",
        ))]);
        destination.add_suffix(&choose);
        destination.set_activatable_widget(Some(&choose));
        let box_list = gtk::ListBox::new();
        box_list.add_css_class("boxed-list");
        box_list.set_selection_mode(gtk::SelectionMode::None);
        box_list.append(&destination);
        content.append(&box_list);
        let ui = self.clone();
        let folder_copy = folder.clone();
        choose.connect_clicked(move |_| {
            let picker = gtk::FileDialog::builder()
                .title(tr("Choose a receiving folder"))
                .initial_folder(&gio::File::for_path(folder_copy.borrow().as_str()))
                .build();
            let folder = folder_copy.clone();
            let row = destination.clone();
            let ui_copy = ui.clone();
            picker.select_folder(Some(&ui.window), gio::Cancellable::NONE, move |result| {
                match result {
                    Ok(file) => {
                        if let Some(path) = file.path() {
                            *folder.borrow_mut() = path.to_string_lossy().into_owned();
                            row.set_subtitle(&folder.borrow());
                        } else {
                            ui_copy.toast("Choose a local folder");
                        }
                    }
                    Err(error)
                        if error.matches(gtk::DialogError::Dismissed)
                            || error.matches(gtk::DialogError::Cancelled) => {}
                    Err(error) => ui_copy.toast(&error.to_string()),
                }
            });
        });
        let collision = adw::ComboRow::builder()
            .title(tr("Existing filenames"))
            .model(&gtk::StringList::new(&[
                &tr("Save with a new name"),
                &tr("Reject conflicting files"),
            ]))
            .selected(u32::from(
                previous
                    .and_then(|options| options["collision_policy"].as_str())
                    .or_else(|| settings["receive"]["collision_policy"].as_str())
                    == Some("reject"),
            ))
            .build();
        collision.set_widget_name("incoming-collision");
        box_list.append(&collision);
        content.append(&label(
            "Nothing is saved until you accept. Existing files are never overwritten.",
            "compact-note",
        ));
        let weak = Rc::downgrade(self);
        let request = transfer.clone();
        let generation = self.service_generation();
        dialog.connect_response(None, move |_, response| {
            if response != "accept" { return; }
            let Some(ui) = weak.upgrade() else { return; };
            if ui.service_generation() != generation { return; }
            let mut options = json!({"collision_policy": if collision.selected() == 1 {"reject"} else {"rename"}});
            options["selected_indices"] = json!(selected.borrow().iter().enumerate().filter_map(|(index, value)| value.then_some(index)).collect::<Vec<_>>());
            options["directory"] = json!(folder.borrow().as_str());
            let proxy = ui.proxy.borrow().clone();
            let id = id.clone();
            let request = request.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = match proxy {
                    Some(proxy) => crate::ipc::call(&proxy, "AcceptTransferWithOptions", Some((id, options.to_string()).to_variant())).await.map(|_| ()),
                    None => Err(tr("The sharing service is not connected yet")),
                };
                if ui.service_generation() != generation { return; }
                if let Err(error) = result {
                    // Failed consent must not discard the user's destination or subset.
                    ui.review_request(&request, Some(&options), Some(&error));
                }
                ui.refresh();
            });
        });
        self.track_service_dialog(&dialog);
        dialog.present(Some(&self.window));
        dialog
    }

    pub fn open_received_file(self: &Rc<Self>, path: &str, show_folder: bool) {
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
        let weak = Rc::downgrade(self);
        let completed = move |result: Result<(), glib::Error>| {
            if let Err(error) = result {
                if !error.matches(gtk::DialogError::Dismissed)
                    && !error.matches(gtk::DialogError::Cancelled)
                {
                    if let Some(ui) = weak.upgrade() {
                        ui.toast(&error.to_string());
                    }
                }
            }
        };
        if show_folder {
            launcher.open_containing_folder(Some(&self.window), gio::Cancellable::NONE, completed);
        } else {
            launcher.launch(Some(&self.window), gio::Cancellable::NONE, completed);
        }
    }

    pub fn received_files(self: &Rc<Self>, paths: &[String]) -> adw::Dialog {
        let dialog = adw::Dialog::builder()
            .title(tr("Received files"))
            .content_width(480)
            .content_height((paths.len().min(6) as i32 * 56 + 72).clamp(184, 420))
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_valign(gtk::Align::Start);
        list.add_css_class("boxed-list");
        list.set_margin_top(18);
        list.set_margin_bottom(18);
        list.set_margin_start(18);
        list.set_margin_end(18);
        for (index, path) in paths.iter().enumerate() {
            let file = gio::File::for_path(path);
            let name = file
                .basename()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let row = adw::ActionRow::builder().title(&name).build();
            row.set_use_markup(false);
            row.set_title_lines(2);
            row.set_tooltip_text(Some(path));
            let open = gtk::Button::from_icon_name("document-open-symbolic");
            open.set_widget_name(&format!("received-file-{index}"));
            open.set_valign(gtk::Align::Center);
            open.set_tooltip_text(Some(&tr("Open file")));
            open.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}: {name}",
                tr("Open file")
            ))]);
            let weak = Rc::downgrade(self);
            let path = path.clone();
            open.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_received_file(&path, false);
                }
            });
            row.add_suffix(&open);
            row.set_activatable_widget(Some(&open));
            list.append(&row);
        }
        toolbar.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list)
                .build(),
        ));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
        dialog
    }

    pub fn provide_pin(self: &Rc<Self>, transfer: &Value) {
        let download = transfer["direction"].as_str() == Some("incoming");
        let pin = adw::PasswordEntryRow::builder()
            .title(tr(if download {
                "Sender's PIN"
            } else {
                "Receiving PIN"
            }))
            .build();
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&pin);
        let dialog = adw::AlertDialog::builder().heading(tr("Enter PIN")).body(tr(if download { "Enter the PIN supplied by the sender for this download link." } else { "Enter the PIN shown in the receiving device's LocalSend settings. This is separate from Quick Share code verification." })).extra_child(&list).build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("send", &tr("Continue"))]);
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("send"));
        dialog.set_focus(Some(&pin));
        dialog.set_response_appearance("send", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("send", false);
        let weak = dialog.downgrade();
        pin.connect_changed(move |pin| {
            if let Some(dialog) = weak.upgrade() {
                let text = pin.text();
                dialog.set_response_enabled(
                    "send",
                    (4..=12).contains(&text.len())
                        && text.bytes().all(|byte| byte.is_ascii_digit()),
                );
            }
        });
        let weak = Rc::downgrade(self);
        let id = transfer["id"].as_str().unwrap_or_default().to_owned();
        dialog.connect_response(None, move |_, response| {
            if response == "send" {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate(
                        "ProvideTransferPin",
                        (id.clone(), pin.text().to_string()).to_variant(),
                    );
                }
            }
            pin.set_text("");
        });
        self.track_service_dialog(&dialog);
        dialog.present(Some(&self.window));
    }
}
