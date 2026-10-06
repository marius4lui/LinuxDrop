use crate::{
    i18n::tr,
    ipc,
    ui::{label, protocol_name, Ui},
};
use adw::prelude::*;
use gtk::glib;
use serde_json::json;
use std::rc::Rc;

impl Ui {
    pub fn manage_devices(self: &Rc<Self>) {
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
            let row = adw::ExpanderRow::builder()
                .title(name)
                .subtitle(if peer["available"].as_bool().unwrap_or(false) {
                    tr("Nearby now")
                } else {
                    tr("Not currently nearby")
                })
                .build();
            let favorite = adw::SwitchRow::builder()
                .title(tr("Favorite"))
                .subtitle(tr("Show this device first"))
                .active(peer["favorite"].as_bool().unwrap_or(false))
                .build();
            connect_preference(self, &favorite, &id, "favorite");
            row.add_row(&favorite);
            let alias = adw::EntryRow::builder()
                .title(tr("Local display name"))
                .text(peer["display_name"].as_str().unwrap_or_default())
                .show_apply_button(true)
                .build();
            let weak = Rc::downgrade(self);
            let peer_id = id.clone();
            alias.connect_apply(move |entry| {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate(
                        "UpdatePeerPreferences",
                        (
                            peer_id.clone(),
                            json!({"display_name":entry.text().as_str()}).to_string(),
                        )
                            .to_variant(),
                    );
                }
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
            protocol.connect_selected_notify(move |row| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(protocol) = protocols.get(row.selected() as usize) {
                        ui.mutate(
                            "UpdatePeerPreferences",
                            (
                                peer_id.clone(),
                                json!({"preferred_protocol":protocol}).to_string(),
                            )
                                .to_variant(),
                        );
                    }
                }
            });
            row.add_row(&protocol);
            let block = adw::SwitchRow::builder()
                .title(tr("Block this protocol identity"))
                .subtitle(tr(
                    "This is a local filter, not a guarantee against impersonation",
                ))
                .active(peer["blocked"].as_bool().unwrap_or(false))
                .build();
            connect_preference(self, &block, &id, "blocked");
            row.add_row(&block);
            let forget = adw::ActionRow::builder()
                .title(tr("Forget device preferences"))
                .subtitle(tr("The device can be discovered again"))
                .build();
            let button = gtk::Button::from_icon_name("user-trash-symbolic");
            button.set_valign(gtk::Align::Center);
            button.set_tooltip_text(Some(&tr("Forget device preferences")));
            let weak = Rc::downgrade(self);
            let parent = dialog.downgrade();
            button.connect_clicked(move |_|{
                let Some(ui)=weak.upgrade()else{return;};let Some(proxy)=ui.proxy.borrow().clone()else{return;};
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
    }
}

fn connect_preference(ui: &Rc<Ui>, row: &adw::SwitchRow, id: &str, key: &'static str) {
    let weak = Rc::downgrade(ui);
    let id = id.to_owned();
    row.connect_active_notify(move |row| {
        if let Some(ui) = weak.upgrade() {
            ui.mutate(
                "UpdatePeerPreferences",
                (id.clone(), json!({key:row.is_active()}).to_string()).to_variant(),
            );
        }
    });
}
