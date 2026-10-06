use crate::{
    i18n::tr,
    ui::{bytes, label, Ui},
};
use adw::prelude::*;
use gtk::gio;
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};

impl Ui {
    pub fn accept_request(self: &Rc<Self>, transfer: &Value) -> adw::AlertDialog {
        let id = transfer["id"].as_str().unwrap_or_default().to_owned();
        let files = transfer["files"].as_array().cloned().unwrap_or_default();
        let partial = transfer["selection_mode"].as_str() == Some("native")
            || transfer["protocol"].as_str() == Some("localsend");
        let selected = Rc::new(RefCell::new(vec![true; files.len()]));
        let settings = self.settings.borrow().clone();
        let folder = Rc::new(RefCell::new(
            transfer["receive_directory"]
                .as_str()
                .or_else(|| settings["receive"]["directory"].as_str())
                .unwrap_or_default()
                .to_owned(),
        ));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
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
        dialog.set_response_enabled("accept", !files.is_empty());
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (index, file) in files.iter().enumerate() {
            let name = file["name"].as_str().unwrap_or_default();
            let check = gtk::CheckButton::with_label(&format!(
                "{name} · {}",
                bytes(file["size"].as_u64().unwrap_or(0))
            ));
            check.set_widget_name(&format!("incoming-file-{index}"));
            check.set_active(true);
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
        let choose = gtk::Button::from_icon_name("folder-open-symbolic");
        choose.set_valign(gtk::Align::Center);
        choose.set_tooltip_text(Some(&tr("Choose a receiving folder")));
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
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        *folder.borrow_mut() = path.to_string_lossy().into_owned();
                        row.set_subtitle(&folder.borrow());
                    } else {
                        ui_copy.toast("Choose a local folder");
                    }
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
                settings["receive"]["collision_policy"].as_str() == Some("reject"),
            ))
            .build();
        box_list.append(&collision);
        content.append(&label(
            "Nothing is saved until you accept. Existing files are never overwritten.",
            "compact-note",
        ));
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "accept" { return; }
            let Some(ui) = weak.upgrade() else { return; };
            let mut options = json!({"collision_policy": if collision.selected() == 1 {"reject"} else {"rename"}});
            options["selected_indices"] = json!(selected.borrow().iter().enumerate().filter_map(|(index, value)| value.then_some(index)).collect::<Vec<_>>());
            options["directory"] = json!(folder.borrow().as_str());
            ui.mutate("AcceptTransferWithOptions", (id.clone(), options.to_string()).to_variant());
        });
        dialog.present(Some(&self.window));
        dialog
    }

    pub fn provide_pin(self: &Rc<Self>, transfer: &Value) {
        let pin = adw::PasswordEntryRow::builder()
            .title(tr("Receiving PIN"))
            .build();
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&pin);
        let dialog = adw::AlertDialog::builder().heading(tr("Enter PIN")).body(tr("Enter the PIN shown in the receiving device's LocalSend settings. This is separate from Quick Share code verification.")).extra_child(&list).build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("send", &tr("Continue"))]);
        dialog.set_close_response("cancel");
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
        dialog.present(Some(&self.window));
    }
}
