use crate::{
    i18n::tr,
    ipc,
    ui::{label, Ui},
};
use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;
use std::rc::Rc;

pub fn add_actions(ui: &Rc<Ui>, group: &adw::PreferencesGroup) -> Vec<(gtk::Widget, String)> {
    let mut rows = Vec::new();
    for (title, subtitle, action) in [
        (
            "Save diagnostic report",
            "Export a redacted JSON file; review it before sharing",
            "export",
        ),
        (
            "Show default settings",
            "Inspect the defaults before resetting",
            "defaults",
        ),
        (
            "Restart sharing backends",
            "Unavailable during active transfers; the app stays open",
            "restart",
        ),
        (
            "Restore default settings",
            "Reset preferences without deleting received files",
            "reset",
        ),
        (
            "Recovery status",
            "Inspect adapter leases and pending cleanup",
            "recovery",
        ),
    ] {
        let row = adw::ActionRow::builder()
            .title(tr(title))
            .subtitle(tr(subtitle))
            .build();
        let button = gtk::Button::from_icon_name(match action {
            "export" => "document-save-symbolic",
            "restart" => "view-refresh-symbolic",
            "reset" => "edit-undo-symbolic",
            _ => "go-next-symbolic",
        });
        button.set_tooltip_text(Some(&tr(title)));
        button.set_widget_name(&format!("diagnostic:{action}"));
        button.update_property(&[gtk::accessible::Property::Label(&tr(title))]);
        button.set_valign(gtk::Align::Center);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        let weak = Rc::downgrade(ui);
        button.connect_clicked(move |_| { if let Some(ui) = weak.upgrade() {
            match action {
                "export" => export(&ui),
                "defaults" => show_report(&ui, "GetDefaults", "Default settings"),
                "recovery" => show_report(&ui, "GetRecoveryStatus", "Recovery status"),
                "restart" => confirm(&ui, "RestartBackends", "Restart sharing backends", "Discovery is briefly interrupted. Active transfers must finish or be cancelled first."),
                "reset" => confirm(&ui, "ResetSettings", "Restore default settings", "Your preferences will return to their defaults. Received files and device history are not deleted. Active transfers must finish first."),
                _ => {}
            }
        }});
        group.add(&row);
        rows.push((
            row.upcast(),
            format!("{} {}", tr(title), tr(subtitle)).to_lowercase(),
        ));
    }
    rows
}

fn confirm(ui: &Rc<Ui>, method: &'static str, title: &str, description: &str) {
    let link_active = ui.snapshot.borrow()["download_link_active"] == true;
    let active = link_active
        || ui.snapshot.borrow()["transfers"]
            .as_array()
            .is_some_and(|transfers| {
                transfers.iter().any(|transfer| {
                    !matches!(
                        transfer["state"].as_str(),
                        Some("completed" | "rejected" | "cancelled" | "failed")
                    )
                })
            });
    let dialog = adw::AlertDialog::builder()
        .heading(tr(title))
        .body(tr(if link_active {
            "Stop the download link before restarting sharing or changing network settings"
        } else {
            description
        }))
        .build();
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("apply", &tr(title))]);
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("apply", adw::ResponseAppearance::Destructive);
    dialog.set_response_enabled("apply", !active);
    let weak = Rc::downgrade(ui);
    dialog.connect_response(None, move |_, response| {
        if response == "apply" {
            if let Some(ui) = weak.upgrade() {
                if method == "ResetSettings" {
                    ui.settings_drafts.borrow_mut().clear();
                }
                ui.mutate(method, ().to_variant());
            }
        }
    });
    dialog.present(Some(&ui.window));
}

pub fn show_report(ui: &Rc<Ui>, method: &'static str, title: &'static str) {
    let Some(proxy) = ui.proxy.borrow().clone() else {
        ui.toast("The sharing service is not connected yet");
        return;
    };
    let ui = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        match ipc::json(&proxy, method).await {
            Ok(report) => report_dialog(&ui, title, &report),
            Err(error) => ui.toast(&error),
        }
    });
}

pub fn report_dialog(ui: &Rc<Ui>, title: &str, report: &Value) {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    view.buffer()
        .set_text(&serde_json::to_string_pretty(report).unwrap_or_default());
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(320)
        .max_content_height(520)
        .min_content_width(340)
        .child(&view)
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading(tr(title))
        .extra_child(&scroll)
        .build();
    dialog.add_response("close", &tr("Close"));
    dialog.present(Some(&ui.window));
}

fn export(ui: &Rc<Ui>) {
    let Some(proxy) = ui.proxy.borrow().clone() else {
        ui.toast("The sharing service is not connected yet");
        return;
    };
    let ui = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        let report = match ipc::json(&proxy, "ExportDiagnostics").await {
            Ok(report) => report,
            Err(error) => {
                ui.toast(&error);
                return;
            }
        };
        let picker = gtk::FileDialog::builder()
            .title(tr("Save diagnostic report"))
            .initial_name("LinuxDrop-diagnostics.json")
            .build();
        let file = match picker.save_future(Some(&ui.window)).await {
            Ok(file) => file,
            Err(_) => return,
        };
        let bytes = serde_json::to_vec_pretty(&report).unwrap_or_default();
        match file
            .replace_contents_future(bytes, None, false, gio::FileCreateFlags::PRIVATE)
            .await
        {
            Ok(_) => ui.toast("Diagnostic report saved"),
            Err((_, error)) => ui.toast(&error.to_string()),
        }
    });
}

impl Ui {
    pub fn run_hardware_diagnostic(self: &Rc<Self>, radio_id: &str) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&label("This active test may briefly create a monitor interface and change its channel. It never runs automatically. The daemon refuses tests on protected or busy adapters.","compact-note"));
        let channel = adw::SpinRow::builder()
            .title(tr("Test channel"))
            .adjustment(&gtk::Adjustment::new(6.0, 1.0, 196.0, 1.0, 5.0, 0.0))
            .build();
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&channel);
        content.append(&list);
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Run active hardware test"))
            .body(radio_id)
            .extra_child(&content)
            .build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("run", &tr("Run test"))]);
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("run", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        let id = radio_id.to_owned();
        dialog.connect_response(None, move |_, response| {
            if response != "run" {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(proxy) = ui.proxy.borrow().clone() else {
                return;
            };
            let id = id.clone();
            let channel = channel.value() as u16;
            glib::MainContext::default().spawn_local(async move {
                match ipc::call(
                    &proxy,
                    "RunHardwareDiagnostic",
                    Some((id, channel).to_variant()),
                )
                .await
                .and_then(ipc::string_result)
                {
                    Ok(text) => match serde_json::from_str(&text) {
                        Ok(report) => report_dialog(&ui, "Hardware diagnostic", &report),
                        Err(error) => ui.toast(&error.to_string()),
                    },
                    Err(error) => ui.toast(&error),
                }
                ui.refresh();
            });
        });
        dialog.present(Some(&self.window));
    }
}
