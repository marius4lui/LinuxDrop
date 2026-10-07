use crate::{
    i18n::tr,
    ipc,
    ui::{label, protocol_name, Ui},
};
use adw::prelude::*;
use gtk::glib;
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

impl Ui {
    pub fn manage_devices(self: &Rc<Self>) -> adw::Dialog {
        let snapshot = self.snapshot.borrow().clone();
        let mut peers = snapshot["known_peers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for peer in snapshot["peers"].as_array().into_iter().flatten() {
            if let Some(index) = peers.iter().position(|known| known["id"] == peer["id"]) {
                peers[index] = peer.clone();
            } else {
                peers.push(peer.clone());
            }
        }
        peers.sort_by_key(|peer| {
            (
                !peer["favorite"].as_bool().unwrap_or(false),
                peer["name"].as_str().unwrap_or_default().to_lowercase(),
            )
        });
        let dialog = adw::Dialog::builder()
            .title(tr("Manage devices"))
            .content_width(540)
            .content_height(620)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        content.append(&label("Favorites and labels are local preferences, not verified identities. A blocked device may return under a changed protocol identity; always review incoming requests.","compact-note"));
        if peers.is_empty() {
            content.append(
                &adw::StatusPage::builder()
                    .icon_name("computer-symbolic")
                    .title(tr("No known devices yet"))
                    .description(tr(
                        "Discovered devices will appear here, including your saved preferences.",
                    ))
                    .build(),
            );
        }
        let group = adw::PreferencesGroup::new();
        content.append(&group);
        for peer in peers {
            let id = peer["id"].as_str().unwrap_or_default().to_owned();
            let name = peer["display_name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or_else(|| peer["name"].as_str())
                .unwrap_or(&id);
            let row = adw::ExpanderRow::new();
            row.set_use_markup(false);
            row.set_title(name);
            row.set_subtitle(&tr(if peer["available"].as_bool().unwrap_or(false) {
                "Nearby now"
            } else {
                "Not currently nearby"
            }));
            row.set_widget_name(&format!("device:{id}"));
            let status = label("", "compact-note");
            status.set_widget_name(&format!("device:{id}:status"));
            status.set_visible(false);
            status.set_margin_top(8);
            status.set_margin_bottom(8);
            status.set_margin_start(12);
            status.set_margin_end(12);
            row.add_row(&status);
            let favorite = adw::SwitchRow::builder()
                .title(tr("Favorite"))
                .subtitle(tr("Show this device first"))
                .active(peer["favorite"].as_bool().unwrap_or(false))
                .build();
            connect_preference(self, &row, &status, &favorite, &id, "favorite");
            row.add_row(&favorite);
            let alias = adw::EntryRow::builder()
                .title(tr("Local display name"))
                .text(peer["display_name"].as_str().unwrap_or_default())
                .show_apply_button(true)
                .build();
            let weak = Rc::downgrade(self);
            let peer_id = id.clone();
            let parent = row.downgrade();
            let feedback = status.clone();
            let saved_alias = Rc::new(RefCell::new(alias.text().to_string()));
            let original_name = peer["name"].as_str().unwrap_or(&id).to_owned();
            alias.set_widget_name(&format!("device:{id}:display_name"));
            alias.connect_apply(move |entry| {
                let (Some(ui), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
                    return;
                };
                if !parent.is_sensitive() {
                    return;
                }
                let requested = entry.text().to_string();
                let saved = saved_alias.clone();
                let entry = entry.clone();
                let title = parent.clone();
                let original_name = original_name.clone();
                update_preference(
                    ui,
                    parent,
                    feedback.clone(),
                    peer_id.clone(),
                    json!({"display_name":requested}),
                    move |success| {
                        if success {
                            *saved.borrow_mut() = requested.clone();
                            title.set_title(if requested.is_empty() {
                                &original_name
                            } else {
                                &requested
                            });
                        } else {
                            // Re-arm EntryRow's apply action without losing the unsaved draft.
                            entry.set_text(&saved.borrow());
                            entry.set_text(&requested);
                        }
                    },
                );
            });
            row.add_row(&alias);
            let protocols = ["auto", "localsend", "quickshare", "airdrop"];
            let names: Vec<String> = protocols
                .iter()
                .map(|protocol| {
                    if *protocol == "auto" {
                        tr("Automatic")
                    } else {
                        protocol_name(protocol).to_owned()
                    }
                })
                .collect();
            let preferred = peer["preferred_protocol"].as_str().unwrap_or("auto");
            let protocol = adw::ComboRow::builder()
                .title(tr("Preferred protocol"))
                .subtitle(tr("Used when that protocol is available on this device"))
                .model(&gtk::StringList::new(
                    &names.iter().map(String::as_str).collect::<Vec<_>>(),
                ))
                .selected(
                    protocols
                        .iter()
                        .position(|id| *id == preferred)
                        .unwrap_or(0) as u32,
                )
                .build();
            let weak = Rc::downgrade(self);
            let peer_id = id.clone();
            let parent = row.downgrade();
            let feedback = status.clone();
            let saved_protocol = Rc::new(Cell::new(protocol.selected()));
            protocol.set_widget_name(&format!("device:{id}:preferred_protocol"));
            protocol.connect_selected_notify(move |row| {
                let (Some(ui), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
                    return;
                };
                if !parent.is_sensitive() {
                    return;
                }
                let requested = row.selected();
                let Some(protocol) = protocols.get(requested as usize) else {
                    return;
                };
                let saved = saved_protocol.clone();
                let row = row.clone();
                update_preference(
                    ui,
                    parent,
                    feedback.clone(),
                    peer_id.clone(),
                    json!({"preferred_protocol":protocol}),
                    move |success| {
                        if success {
                            saved.set(requested);
                        } else {
                            row.set_selected(saved.get());
                        }
                    },
                );
            });
            row.add_row(&protocol);
            let block = adw::SwitchRow::builder()
                .title(tr("Block this protocol identity"))
                .subtitle(tr(
                    "This is a local filter, not a guarantee against impersonation",
                ))
                .active(peer["blocked"].as_bool().unwrap_or(false))
                .build();
            connect_preference(self, &row, &status, &block, &id, "blocked");
            row.add_row(&block);
            let forget = adw::ActionRow::builder()
                .title(tr("Forget device preferences"))
                .subtitle(tr("The device can be discovered again"))
                .build();
            let button = gtk::Button::from_icon_name("user-trash-symbolic");
            button.set_valign(gtk::Align::Center);
            button.set_tooltip_text(Some(&tr("Forget device preferences")));
            button.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}: {name}",
                tr("Forget device preferences")
            ))]);
            button.set_widget_name(&format!("device:{id}:forget"));
            let weak = Rc::downgrade(self);
            let parent = dialog.downgrade();
            button.connect_clicked(move |_|{
                let Some(ui)=weak.upgrade()else{return;};let Some(proxy)=ui.proxy.borrow().clone()else{ui.toast("Background service unavailable");return;};
                let confirmation=adw::AlertDialog::builder().heading(tr("Forget device preferences?")).body(tr("This removes the local label, favorite and block preference. It does not delete transferred files.")).build();
                confirmation.add_responses(&[("cancel",&tr("Cancel")),("forget",&tr("Forget"))]);confirmation.set_close_response("cancel");confirmation.set_response_appearance("forget",adw::ResponseAppearance::Destructive);
                let id=id.clone();let parent=parent.clone();let owned=ui.clone();
                confirmation.connect_response(None,move |_,response|{
                    if response!="forget"{return;}let ui=owned.clone();let proxy=proxy.clone();let id=id.clone();let parent=parent.clone();
                    glib::MainContext::default().spawn_local(async move{
                        match ipc::call(&proxy,"ForgetPeer",Some((id,).to_variant())).await{Ok(_)=>{if let Some(dialog)=parent.upgrade(){dialog.close();}ui.refresh();},Err(error)=>ui.toast(&error)}
                    });
                });confirmation.present(Some(&ui.window));
            });
            forget.add_suffix(&button);
            forget.set_activatable_widget(Some(&button));
            row.add_row(&forget);
            group.add(&row);
        }
        toolbar.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&content)
                .build(),
        ));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
        dialog
    }
}

fn connect_preference(
    ui: &Rc<Ui>,
    parent: &adw::ExpanderRow,
    status: &gtk::Label,
    row: &adw::SwitchRow,
    id: &str,
    key: &'static str,
) {
    let weak = Rc::downgrade(ui);
    let parent = parent.downgrade();
    let status = status.clone();
    let saved = Rc::new(Cell::new(row.is_active()));
    row.set_widget_name(&format!("device:{id}:{key}"));
    let id = id.to_owned();
    row.connect_active_notify(move |row| {
        let (Some(ui), Some(parent)) = (weak.upgrade(), parent.upgrade()) else {
            return;
        };
        if !parent.is_sensitive() {
            return;
        }
        let requested = row.is_active();
        let saved = saved.clone();
        let row = row.clone();
        update_preference(
            ui,
            parent,
            status.clone(),
            id.clone(),
            json!({key:requested}),
            move |success| {
                if success {
                    saved.set(requested);
                } else {
                    row.set_active(saved.get());
                }
            },
        );
    });
}

fn update_preference(
    ui: Rc<Ui>,
    row: adw::ExpanderRow,
    status: gtk::Label,
    id: String,
    patch: Value,
    completed: impl FnOnce(bool) + 'static,
) {
    // Serialize edits for a device, and suppress change handlers while rolling back.
    row.set_sensitive(false);
    status.remove_css_class("error");
    status.set_label(&tr("Saving device preference…"));
    status.set_visible(true);
    let proxy = ui
        .proxy
        .borrow()
        .clone()
        .filter(|proxy| proxy.g_name_owner().is_some());
    glib::MainContext::default().spawn_local(async move {
        let result = match proxy {
            Some(proxy) => {
                let owner = proxy.g_name_owner();
                let result = ipc::call(
                    &proxy,
                    "UpdatePeerPreferences",
                    Some((id, patch.to_string()).to_variant()),
                )
                .await
                .map(|_| ());
                if proxy.g_name_owner() != owner {
                    Err(tr("Background service unavailable"))
                } else {
                    result
                }
            }
            None => Err(tr("Background service unavailable")),
        };
        completed(result.is_ok());
        row.set_sensitive(true);
        match result {
            Ok(()) => {
                status.set_visible(false);
                ui.refresh();
            }
            Err(error) => {
                status.add_css_class("error");
                status.set_label(&format!(
                    "{}\n{error}",
                    tr("Could not confirm this change. Try again.")
                ));
            }
        }
    });
}
