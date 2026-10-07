//! One native GTK regression scenario, run explicitly in an isolated display/bus.
//! No physical backend or user session is contacted.
use super::*;
use serde_json::json;
use std::time::Duration;

fn find(root: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    let root = root.as_ref();
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

async fn settle() {
    glib::timeout_future(Duration::from_millis(120)).await;
}

fn capture(ui: &Ui, name: &str) {
    let Some(directory) = std::env::var_os("LINUXDROP_TEST_CAPTURES") else {
        return;
    };
    let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(ui.window.width()),
        f64::from(ui.window.height()),
    );
    let node = snapshot.to_node().expect("mapped window render node");
    let renderer = ui.window.renderer().expect("native window renderer");
    let texture = renderer.render_texture(&node, None);
    std::fs::create_dir_all(&directory).unwrap();
    texture
        .save_to_png(std::path::Path::new(&directory).join(name))
        .unwrap();
}

#[test]
#[ignore = "Needs isolated GTK display and session bus; run with dbus-run-session xvfb-run"]
fn native_draft_focus_protocol_and_settings_regressions() {
    // The test backend exposes accessible properties without a screen reader.
    std::env::set_var("GTK_A11Y", "test");
    adw::init().unwrap();
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../resources/style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let app = adw::Application::builder()
        .application_id("io.github.marius4lui.LinuxDrop.NativeTest")
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(ipc::BUS, 0_u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .unwrap();
    let snapshot = Rc::new(RefCell::new(json!({
        "epoch":"native-test", "revision":1,
        "settings":{"general":{"device_name":"Native review","appearance":"dark","language":"system","close_behavior":"background","autostart":false},"visibility":{"mode":"hidden"},"receive":{"directory":"/tmp","ask_directory":true,"collision_policy":"rename"},"localsend":{"enabled":false,"pin":"","require_pin":false}},
        "hardware":{"radios":[],"bluetooth":[]},"backends":[],
        "peers":[{"id":"pixel","name":"Pixel test fixture","platform":"android","available":true,"protocols":["localsend","quickshare"]}],
        "transfers":[{"id":"progress","peer_id":"pixel","peer_name":"Pixel test fixture","protocol":"localsend","direction":"outgoing","state":"transferring","total_bytes":100,"transferred_bytes":10,"files":[{"name":"progress.txt","size":100,"transferred":10}]}]
    })));
    let sent_protocol = Rc::new(RefCell::new(String::new()));
    let accepted_options = Rc::new(RefCell::new(Value::Null));
    let fail_next_accept = Rc::new(Cell::new(false));
    let batches = Rc::new(RefCell::new(Vec::new()));
    let discarded = Rc::new(Cell::new(false));
    let fail_next_stop = Rc::new(Cell::new(true));
    let fail_next_preference = Rc::new(Cell::new(false));
    let preference_calls = Rc::new(Cell::new(0));
    let fail_next_setting = Rc::new(Cell::new(false));
    let setting_calls = Rc::new(Cell::new(0));
    let hold_snapshot = Rc::new(Cell::new(false));
    let pending_snapshots = Rc::new(RefCell::new(Vec::<(
        gio::DBusMethodInvocation,
        glib::Variant,
    )>::new()));
    let info = gio::DBusNodeInfo::for_xml(&format!("<node><interface name='{}'><method name='RunHardwareDiagnostic'><arg type='s' direction='in'/><arg type='q' direction='in'/><arg type='s' direction='out'/></method><method name='StopDownloadOffer'/><method name='ResetSettings'/><method name='UpdateSettings'><arg type='s' direction='in'/></method><method name='UpdatePeerPreferences'><arg type='s' direction='in'/><arg type='s' direction='in'/></method><method name='GetSnapshot'><arg type='s' direction='out'/></method><method name='PrepareSendFiles'><arg type='s' direction='in'/><arg type='a(sh)' direction='in'/><arg type='s' direction='out'/></method><method name='DiscardDraft'><arg type='s' direction='in'/></method><method name='StartSend'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='out'/></method><method name='AcceptTransferWithOptions'><arg type='s' direction='in'/><arg type='s' direction='in'/></method><signal name='Changed'><arg type='t'/></signal></interface></node>",ipc::INTERFACE)).unwrap();
    let state = snapshot.clone();
    let sent = sent_protocol.clone();
    let accepted = accepted_options.clone();
    let fail_accept = fail_next_accept.clone();
    let sent_batches = batches.clone();
    let was_discarded = discarded.clone();
    let fail_stop = fail_next_stop.clone();
    let fail_preference = fail_next_preference.clone();
    let preference_count = preference_calls.clone();
    let fail_setting = fail_next_setting.clone();
    let setting_count = setting_calls.clone();
    let hold = hold_snapshot.clone();
    let pending = pending_snapshots.clone();
    let registration = bus
        .register_object(ipc::PATH, &info.interfaces()[0])
        .method_call(
            move |_, _, _, _, method, parameters, invocation| match method {
                "GetSnapshot" => {
                    let response = (state.borrow().to_string(),).to_variant();
                    if hold.get() {
                        pending.borrow_mut().push((invocation, response));
                    } else {
                        invocation.return_value(Some(&response));
                    }
                }
                "StopDownloadOffer" => {
                    if fail_stop.replace(false) {
                        invocation.return_dbus_error(
                            "io.github.marius4lui.Error",
                            "Cleanup still running",
                        );
                    } else {
                        state.borrow_mut()["download_link_active"] = json!(false);
                        invocation.return_value(None);
                    }
                }
                "ResetSettings" => {
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        invocation
                            .return_dbus_error("io.github.marius4lui.Error", "Test reset refused");
                    });
                }
                "UpdateSettings" => {
                    setting_count.set(setting_count.get() + 1);
                    let (patch,) = parameters.get::<(String,)>().unwrap();
                    let patch: Value = serde_json::from_str(&patch).unwrap();
                    let fail = fail_setting.replace(false);
                    let state = state.clone();
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        if fail {
                            invocation.return_dbus_error(
                                "io.github.marius4lui.Error",
                                "Test setting save failed",
                            );
                        } else {
                            for (section, values) in patch.as_object().unwrap() {
                                for (key, value) in values.as_object().unwrap() {
                                    state.borrow_mut()["settings"][section][key] = value.clone();
                                }
                            }
                            invocation.return_value(None);
                        }
                    });
                }
                "UpdatePeerPreferences" => {
                    preference_count.set(preference_count.get() + 1);
                    let (_, patch) = parameters.get::<(String, String)>().unwrap();
                    let patch: Value = serde_json::from_str(&patch).unwrap();
                    let fail = fail_preference.replace(false);
                    let state = state.clone();
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        if fail {
                            invocation.return_dbus_error(
                                "io.github.marius4lui.Error",
                                "Test preference save failed",
                            );
                        } else {
                            for (key, value) in patch.as_object().unwrap() {
                                state.borrow_mut()["peers"][0][key] = value.clone();
                            }
                            invocation.return_value(None);
                        }
                    });
                }
                "PrepareSendFiles" => {
                    use std::os::fd::FromRawFd;
                    let (_, entries) = parameters
                        .get::<(String, Vec<(String, glib::variant::Handle)>)>()
                        .unwrap();
                    let descriptors = invocation
                        .message()
                        .unix_fd_list()
                        .expect("Native sends must include descriptors");
                    assert!(!entries.is_empty());
                    sent_batches.borrow_mut().push(entries.len());
                    for (_, handle) in entries {
                        let fd = descriptors.get(handle.0).unwrap();
                        let file = unsafe { std::fs::File::from_raw_fd(fd) };
                        assert!(file.metadata().unwrap().is_file());
                    }
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        invocation.return_value(Some(&("draft",).to_variant()))
                    });
                }
                "DiscardDraft" => {
                    was_discarded.set(true);
                    invocation.return_value(None);
                }
                "StartSend" => {
                    let (_, _, protocol) = parameters.get::<(String, String, String)>().unwrap();
                    *sent.borrow_mut() = protocol;
                    invocation.return_value(Some(&("sent",).to_variant()));
                }
                "AcceptTransferWithOptions" => {
                    if fail_accept.replace(false) {
                        invocation.return_dbus_error(
                            "io.github.marius4lui.Error",
                            "Destination is not writable",
                        );
                        return;
                    }
                    let (_, options) = parameters.get::<(String, String)>().unwrap();
                    *accepted.borrow_mut() = serde_json::from_str(&options).unwrap();
                    invocation.return_value(None);
                }
                _ => {
                    invocation.return_dbus_error("io.github.marius4lui.Error", "Unexpected method")
                }
            },
        )
        .build()
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "linuxdrop-native-regression-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("empty-valid.txt");
    let second = root.join("added-during-send.txt");
    std::fs::write(&first, []).unwrap();
    std::fs::write(&second, b"new selection").unwrap();
    glib::MainContext::default().block_on(async {
        let ui = build(&app, "send", vec![]);
        settle().await;
        ui.window.set_default_size(480, 600);
        settle().await;
        capture(&ui, "send-compact-review.png");
        assert!(
            ui.peers.compute_bounds(&ui.window).unwrap().y() < 480.0,
            "Nearby must be visible before the bottom actions at 480x600"
        );
        *ui.selected.borrow_mut() = Some("pixel".into());
        ui.render_peers();
        ui.protocol.set_selected(2);
        assert_eq!(&*ui.selected_protocol.borrow(), "quickshare");
        snapshot.borrow_mut()["peers"][0]["protocols"] = json!(["quickshare", "localsend"]);
        snapshot.borrow_mut()["revision"] = json!(2);
        ui.refresh();
        settle().await;
        assert_eq!(&*ui.selected_protocol.borrow(), "quickshare");
        assert_eq!(
            ui.protocol.selected(),
            1,
            "Reordering protocols must retain the selected identity"
        );
        let peer = find(&ui.peers, "peer:pixel").unwrap();
        peer.grab_focus();
        snapshot.borrow_mut()["transfers"][0]["transferred_bytes"] = json!(45);
        snapshot.borrow_mut()["transfers"][0]["files"][0]["transferred"] = json!(45);
        snapshot.borrow_mut()["revision"] = json!(3);
        ui.refresh();
        settle().await;
        assert_eq!(
            find(&ui.peers, "peer:pixel").unwrap(),
            peer,
            "Progress must preserve the peer widget"
        );
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&ui.window),
            Some(peer),
            "Progress must preserve keyboard focus"
        );
        ui.add_files(vec![
            gio::File::for_path(&first),
            gio::File::for_path(&root),
        ]);
        settle().await;
        assert!(matches!(
            ui.file_checks
                .borrow()
                .get(gio::File::for_path(&first).uri().as_str()),
            Some(FileCheck::Ready(0))
        ));
        assert!(matches!(
            ui.file_checks
                .borrow()
                .get(gio::File::for_path(&root).uri().as_str()),
            Some(FileCheck::Invalid(_))
        ));
        assert!(!ui.send.is_sensitive());
        let remove = find(&ui.file_box, &format!("remove:{}", gio::File::for_path(&root).uri())).unwrap().downcast::<gtk::Button>().unwrap();
        remove.grab_focus();
        remove.emit_clicked();
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&ui.window),
            find(&ui.file_box, &format!("remove:{}", gio::File::for_path(&first).uri())),
            "Removing an invalid file keeps keyboard focus on the neighboring file action");
        assert!(ui.send.is_sensitive());
        ui.start_send();
        glib::timeout_future(Duration::from_millis(30)).await;
        ui.add_files(vec![gio::File::for_path(&second)]);
        glib::timeout_future(Duration::from_millis(350)).await;
        assert_eq!(
            ui.files.borrow().len(),
            1,
            "Later additions must survive successful send"
        );
        assert_eq!(ui.files.borrow()[0].path(), Some(second.clone()));
        assert_eq!(&*sent_protocol.borrow(), "quickshare");
        ui.stack.set_visible_child_name("send");
        let remove = find(&ui.file_box, &format!("remove:{}", gio::File::for_path(&second).uri())).unwrap().downcast::<gtk::Button>().unwrap();
        remove.grab_focus();
        remove.emit_clicked();
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&ui.window), Some(ui.choose_button.clone().upcast()),
            "Removing the final file focuses Choose files so keyboard users can continue");
        ui.add_files(vec![gio::File::for_path(&second)]);
        settle().await;
        ui.stack.set_visible_child_name("settings");
        settle().await;
        let entry = find(&ui.settings_body, "setting:general.device_name")
            .unwrap()
            .downcast::<adw::EntryRow>()
            .unwrap();
        entry.set_text("Unapplied device name");
        entry.grab_focus();
        let mut config = ui.settings.borrow().clone();
        config["general"]["autostart"] = json!(true);
        settings::render(&ui, &config);
        settle().await;
        let entry = find(&ui.settings_body, "setting:general.device_name")
            .unwrap()
            .downcast::<adw::EntryRow>()
            .unwrap();
        assert_eq!(
            entry.text(),
            "Unapplied device name",
            "Unapplied text survives another setting update"
        );
        capture(&ui, "settings-compact-review.png");
        let search = find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap();
        search.set_text(&tr("Start at login"));
        // Render the actual persisted value before simulating a failed change.
        settings::render(&ui, &ui.settings.borrow().clone());
        fail_next_setting.set(true);
        let autostart = find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap();
        autostart.set_active(true);
        settle().await;
        let pending = find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap();
        assert!(!pending.is_sensitive() && pending.is_active(), "A pending setting shows the requested value and prevents duplicate writes");
        assert_eq!(find(&ui.settings_body, "setting-status:general.autostart").unwrap().downcast::<adw::ActionRow>().unwrap().title(), tr("Saving setting…"));
        glib::timeout_future(Duration::from_millis(350)).await;
        let saved = find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap();
        assert!(!saved.is_active() && saved.is_sensitive(), "Failed switches restore the confirmed value");
        let status = find(&ui.settings_body, "setting-status:general.autostart").unwrap().downcast::<adw::ActionRow>().unwrap();
        assert!(status.subtitle().unwrap().contains("Test setting save failed"));
        assert_eq!(setting_calls.get(), 1, "Rebuilding and rollback must not send writes");
        let retry = find(&ui.settings_body, "setting:retry:general.autostart").unwrap();
        assert!(retry.grab_focus(), "Retry must be keyboard focusable");
        let name = std::ffi::CString::new(format!("{}: {}", tr("Try again"), tr("Start at login"))).unwrap();
        let mismatch: Option<glib::GString> = unsafe {
            glib::translate::from_glib_full(gtk::ffi::gtk_test_accessible_check_property(
                retry.as_ptr().cast(), gtk::ffi::GTK_ACCESSIBLE_PROPERTY_LABEL, name.as_ptr(),
            ))
        };
        assert!(mismatch.is_none(), "Retry exposes the setting-specific translated name");
        settle().await;
        capture(&ui, "settings-save-failed.png");
        hold_snapshot.set(true);
        ui.refresh();
        settle().await;
        assert_eq!(pending_snapshots.borrow().len(), 1);
        find(&ui.settings_body, "setting:retry:general.autostart").unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
        glib::timeout_future(Duration::from_millis(400)).await;
        assert!(find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap().is_active());
        assert!(find(&ui.settings_body, "setting-status:general.autostart").is_none());
        hold_snapshot.set(false);
        let (call, old_settings) = pending_snapshots.borrow_mut().remove(0);
        call.return_value(Some(&old_settings));
        settle().await;
        assert!(find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap().is_active(), "A read begun before success cannot revert a confirmed setting");
        assert_eq!(setting_calls.get(), 2);
        assert_eq!(ui.settings_drafts.borrow().get("general.device_name").map(String::as_str), Some("Unapplied device name"));
        // The Apply button's internal dirty state is reset when a row is rebuilt;
        // a separate retry must retain and resend the exact text after failure.
        find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap().set_text(&tr("Device name"));
        fail_next_setting.set(true);
        find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap().emit_by_name::<()>("apply", &[]);
        ui.mutate("ResetSettings", ().to_variant());
        assert!(!ui.settings_resetting.get(), "Reset cannot race a pending field write");
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap().text(), "Unapplied device name");
        find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap().set_text("Revised device name");
        find(&ui.settings_body, "setting:retry:general.device_name").unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(ui.settings.borrow()["general"]["device_name"], "Revised device name", "Retry uses the most recently edited text");
        fail_next_setting.set(true);
        let entry = find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap();
        entry.set_text("Discard this failed edit");
        entry.emit_by_name::<()>("apply", &[]);
        glib::timeout_future(Duration::from_millis(400)).await;
        let entry = find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap();
        entry.set_text("Revised device name");
        let calls = setting_calls.get();
        entry.emit_by_name::<()>("apply", &[]);
        settle().await;
        assert_eq!(setting_calls.get(), calls);
        assert!(find(&ui.settings_body, "setting-status:general.device_name").is_none(), "Restoring the saved value clears the previous error without writing");
        let entry = find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap();
        entry.set_text("Keep after failed reset");
        find(&ui.settings_body, "diagnostic:reset").unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
        ui.window.visible_dialog().unwrap().emit_by_name::<()>("response", &[&"apply"]);
        assert!(ui.settings_resetting.get() && !ui.settings_body.is_sensitive(), "Reset prevents new edits until its receipt");
        let calls = setting_calls.get();
        find(&ui.settings_body, "setting:general.device_name").unwrap().downcast::<adw::EntryRow>().unwrap().emit_by_name::<()>("apply", &[]);
        settle().await;
        assert_eq!(setting_calls.get(), calls);
        glib::timeout_future(Duration::from_millis(300)).await;
        assert!(!ui.settings_resetting.get() && ui.settings_body.is_sensitive());
        assert_eq!(ui.settings_drafts.borrow().get("general.device_name").map(String::as_str), Some("Keep after failed reset"), "An unsuccessful reset preserves unsaved work");
        if let Some(dialog) = ui.window.visible_dialog() { dialog.force_close(); }
        ui.service_error("test settings offline with proxy");
        let calls = setting_calls.get();
        find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap().set_active(false);
        settle().await;
        assert_eq!(setting_calls.get(), calls, "An unready proxy must not send settings changes");
        assert!(find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap().is_active());
        assert!(find(&ui.settings_body, "setting:retry:general.autostart").is_some());
        ui.refresh();
        settle().await;
        find(&ui.settings_body, "setting:retry:general.autostart").unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
        glib::timeout_future(Duration::from_millis(400)).await;
        assert!(!find(&ui.settings_body, "setting:general.autostart").unwrap().downcast::<adw::SwitchRow>().unwrap().is_active());
        find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap().set_text("");
        let require_pin = find(&ui.settings_body, "setting:localsend.require_pin").unwrap();
        assert!(!require_pin.is_sensitive(), "A receiving PIN must be saved before it can be required");
        // Numeric controls must faithfully show every valid saved value and
        // must never change settings just because the user visits the field.
        let numeric_calls = setting_calls.get();
        for saved_bytes in [1_u64, 10_995_116_277_760, 107_374_182_400] {
            let mut numeric_config = ui.settings.borrow().clone();
            numeric_config["receive"]["max_bytes"] = json!(saved_bytes);
            numeric_config["visibility"]["duration_minutes"] = json!(0);
            *ui.settings.borrow_mut() = numeric_config.clone();
            snapshot.borrow_mut()["settings"] = numeric_config.clone();
            settings::render(&ui, &numeric_config);
            let amount = find(&ui.settings_body, "setting:receive.max_bytes").unwrap().downcast::<adw::SpinRow>().unwrap();
            assert_eq!((amount.value() * 1_000_000.0).round() as u64, saved_bytes, "Receive size must not clamp or round an accepted persisted byte count");
            assert_eq!(amount.digits(), 6, "Decimal MB must preserve individual bytes");
            assert_eq!(find(&ui.settings_body, "setting:visibility.duration_minutes").unwrap().downcast::<adw::SpinRow>().unwrap().value(), 0.0, "Unlimited visibility is a valid saved value");
            amount.grab_focus();
            settle().await;
            find(&ui.settings_body, "setting:search").unwrap().grab_focus();
            settle().await;
            assert_eq!(setting_calls.get(), numeric_calls, "Focus-only changes must not write or restart sharing");
        }
        let amount = find(&ui.settings_body, "setting:receive.max_bytes").unwrap().downcast::<adw::SpinRow>().unwrap();
        amount.grab_focus();
        amount.set_value(107_374.182_401);
        find(&ui.settings_body, "setting:search").unwrap().grab_focus();
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(snapshot.borrow()["settings"]["receive"]["max_bytes"], json!(107_374_182_401_u64), "An explicit MB edit preserves the intended byte count");
        let search = find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap();
        search.set_text(&tr("Maximum request size (MB)"));
        settle().await;
        capture(&ui, "settings-numeric-precision.png");
        search.set_text("");
        config["localsend"]["pin"] = json!("1234");
        config["transfers"]["history_limit"] = json!(5000);
        settings::render(&ui, &config);
        assert_eq!(find(&ui.settings_body, "setting:transfers.history_limit").unwrap().downcast::<adw::SpinRow>().unwrap().value(), 5000.0, "A valid saved history limit must not be silently clamped");
        assert!(find(&ui.settings_body, "setting:localsend.require_pin").unwrap().is_sensitive());
        let search = find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap();
        search.set_text(&tr("Device name"));
        search.grab_focus();
        // Rebuild immediately, before SearchEntry's delayed search-changed signal.
        assert_eq!(*ui.settings_query.borrow(), tr("Device name"));
        settings::render(&ui, &config);
        settle().await;
        let search = find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap();
        assert_eq!(search.text(), tr("Device name"));
        let focused = gtk::prelude::GtkWindowExt::focus(&ui.window).unwrap();
        assert!(focused == search.clone().upcast::<gtk::Widget>() || focused.is_ancestor(&search), "Search keeps keyboard focus across a settings resync");
        search.set_text(&format!("  {}   {}  ", tr("Dark"), tr("Appearance")));
        assert!(find(&ui.settings_body, "setting:general.appearance").unwrap().is_visible(),
            "Search must include choice labels and tolerate extra whitespace");
        assert!(!find(&ui.settings_body, "setting:general.device_name").unwrap().is_visible());
        let no_results = find(&ui.settings_body, "settings:no-results").unwrap();
        assert!(!no_results.is_visible());
        search.set_text(&tr("About and diagnostics"));
        let diagnostic = find(&ui.settings_body, "diagnostic:export").unwrap();
        assert!(diagnostic.is_visible(), "A category search must reveal its diagnostic actions");
        for (action, title) in [
            ("export", "Save diagnostic report"),
            ("defaults", "Show default settings"),
            ("restart", "Restart sharing backends"),
            ("reset", "Restore default settings"),
            ("recovery", "Recovery status"),
        ] {
            let button = find(&ui.settings_body, &format!("diagnostic:{action}")).unwrap();
            let name = std::ffi::CString::new(tr(title)).unwrap();
            // GTK's test backend inspects the actual exposed accessible label.
            let mismatch: Option<glib::GString> = unsafe {
                glib::translate::from_glib_full(gtk::ffi::gtk_test_accessible_check_property(
                    button.as_ptr().cast(), gtk::ffi::GTK_ACCESSIBLE_PROPERTY_LABEL, name.as_ptr(),
                ))
            };
            assert!(mismatch.is_none(), "Diagnostic action {action} needs its translated accessible name: {mismatch:?}");
        }
        let category = find(&ui.settings_body, "setting:category").unwrap().downcast::<gtk::DropDown>().unwrap();
        category.set_selected(1);
        assert!(no_results.is_visible(), "A category/search mismatch must explain the empty result");
        settle().await;
        capture(&ui, "settings-no-results-review.png");
        find(&ui.settings_body, "settings:clear-filters").unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
        assert_eq!(search.text(), "");
        assert_eq!(category.selected(), 0);
        assert!(!no_results.is_visible());
        assert!(find(&ui.settings_body, "setting:general.device_name").unwrap().is_visible());
        assert!(diagnostic.is_visible());
        let focused = gtk::prelude::GtkWindowExt::focus(&ui.window).unwrap();
        assert!(focused == search.clone().upcast::<gtk::Widget>() || focused.is_ancestor(&search),
            "Clearing filters must return keyboard focus to search");
        ui.stack.set_visible_child_name("send");
        ui.window.set_default_size(1100, 760);
        settle().await;
        capture(&ui, "send-wide-review.png");
        ui.window.set_default_size(480,600);
        ui.stack.set_visible_child_name("transfers");
        let long_name = format!("{}.txt", "long-filename-without-spaces".repeat(8));
        let dialog = ui.accept_request(&json!({"id":"incoming-verification","peer_name":"Prüfgerät","protocol":"quickshare","direction":"incoming","state":"verification","verification_code":"1234","total_bytes":100,"receive_directory":"/tmp/Reviewed <Sender> & files","selection_mode":"publish_selected","files":[{"name":long_name,"size":75},{"name":"excluded.txt","size":25}]}));
        settle().await;
        let destination = find(&dialog, "incoming-destination").unwrap().downcast::<adw::ActionRow>().unwrap();
        assert!(!destination.uses_markup(), "The receiving directory must be literal text");
        assert_eq!(destination.subtitle().as_deref(), Some("/tmp/Reviewed <Sender> & files"));
        let long_file = find(&dialog, "incoming-file-0").unwrap();
        assert!(long_file.measure(gtk::Orientation::Horizontal, -1).0 < 400,
            "Unbroken filenames must wrap without forcing incoming review wider than a compact window");
        let line_height = long_file.create_pango_layout(Some("A")).pixel_size().1;
        assert!(long_file.measure(gtk::Orientation::Vertical, 300).1 <= line_height * 2 + 16,
            "Long filenames must stay within two lines so other files remain reachable");
        let full_name = format!("{long_name} · {}", bytes(75));
        assert_eq!(long_file.tooltip_text().as_deref(), Some(full_name.as_str()));
        let accessible_name = std::ffi::CString::new(full_name).unwrap();
        // GTK's variadic accessibility assertion is not wrapped by gtk-rs.
        // The LABEL property consumes one NUL-terminated string, kept alive here.
        let mismatch: Option<glib::GString> = unsafe {
            glib::translate::from_glib_full(gtk::ffi::gtk_test_accessible_check_property(
                long_file.as_ptr().cast(),
                gtk::ffi::GTK_ACCESSIBLE_PROPERTY_LABEL,
                accessible_name.as_ptr(),
            ))
        };
        assert!(mismatch.is_none(), "The accessible name must retain the complete filename: {mismatch:?}");
        find(&dialog,"incoming-file-1").unwrap().downcast::<gtk::CheckButton>().unwrap().set_active(false);
        glib::timeout_future(Duration::from_millis(400)).await;
        capture(&ui,"incoming-verification-review.png");
        assert_eq!(dialog.response_label("accept"),tr("Codes match — accept"));
        find(&dialog,"incoming-collision").unwrap().downcast::<adw::ComboRow>().unwrap().set_selected(1);
        fail_next_accept.set(true);
        dialog.emit_by_name::<()>("response",&[&"accept"]);
        dialog.force_close();
        settle().await;
        let retry = ui.window.visible_dialog().expect("Failed acceptance must reopen the review").downcast::<adw::AlertDialog>().unwrap();
        assert!(!find(&retry,"incoming-file-1").unwrap().downcast::<gtk::CheckButton>().unwrap().is_active(), "Retry must preserve excluded files");
        assert_eq!(find(&retry,"incoming-destination").unwrap().downcast::<adw::ActionRow>().unwrap().subtitle().as_deref(),Some("/tmp/Reviewed <Sender> & files"));
        assert_eq!(find(&retry,"incoming-collision").unwrap().downcast::<adw::ComboRow>().unwrap().selected(),1,"Retry must preserve collision policy");
        glib::timeout_future(Duration::from_millis(400)).await;
        capture(&ui, "incoming-retry-review.png");
        retry.emit_by_name::<()>("response",&[&"accept"]);
        settle().await;
        assert_eq!(accepted_options.borrow()["directory"],"/tmp/Reviewed <Sender> & files","Acceptance must preserve the exact previewed destination, including automatic subfolders");
        assert_eq!(accepted_options.borrow()["selected_indices"],json!([0]),"Quick Share publishes only the selected files");
        assert_eq!(accepted_options.borrow()["collision_policy"],"reject");
        retry.force_close();
        settle().await;
        let download = ui.receive_link(Some("http://192.168.1.20:53317"));
        glib::timeout_future(Duration::from_millis(400)).await;
        let address = find(&download, "download-offer-url").unwrap().downcast::<adw::EntryRow>().unwrap();
        assert_eq!(address.text(), "http://192.168.1.20:53317");
        capture(&ui, "download-offer-review.png");
        download.force_close();
        let proxy = ui.proxy.borrow().clone().unwrap();
        ui.snapshot.borrow_mut()["restarting"] = json!(true);
        ui.update_send();
        assert!(!ui.send.is_sensitive() && !ui.share_link.is_sensitive());
        assert_eq!(ui.send_caption.text(), tr("Sharing services are restarting; try again shortly"));
        ui.snapshot.borrow_mut()["restarting"] = json!(false);
        ui.update_send();
        batches.borrow_mut().clear();
        let many = vec![first.to_str().unwrap().to_owned(); 17];
        assert_eq!(ipc::prepare_files(&proxy, many).await.unwrap(), "draft");
        assert_eq!(*batches.borrow(), vec![16, 1], "Native selection must split FD messages");
        let mut broken = vec![first.to_str().unwrap().to_owned(); 16];
        broken.push(root.join("not-present").to_str().unwrap().to_owned());
        assert!(ipc::prepare_files(&proxy, broken).await.is_err());
        assert!(discarded.get(), "A failed later batch releases the partial draft");
        // Apply status lives independently of the settings form. State updates
        // must not replace a focused search field or erase its query.
        snapshot.borrow_mut()["settings"]["localsend"]["enabled"] = json!(true);
        snapshot.borrow_mut()["revision"] = json!(30);
        ui.refresh();
        settle().await;
        ui.stack.set_visible_child_name("settings");
        let search = find(&ui.settings_body, "setting:search").unwrap().downcast::<gtk::SearchEntry>().unwrap();
        search.set_text("pin");
        search.grab_focus();
        snapshot.borrow_mut()["restarting"] = json!(true);
        snapshot.borrow_mut()["revision"] = json!(31);
        ui.refresh();
        settle().await;
        assert!(ui.settings_status.is_visible());
        assert_eq!(ui.settings_status_row.title(), tr("Applying sharing settings"));
        assert!(!ui.settings_status_details.is_visible());
        assert_eq!(find(&ui.settings_body, "setting:search").unwrap(), search.clone().upcast::<gtk::Widget>());
        assert_eq!(search.text(), "pin");
        snapshot.borrow_mut()["restarting"] = json!(false);
        snapshot.borrow_mut()["backends"] = json!([{"id":"localsend","state":"error","detail":"Sharing restart could not finish: test cleanup timeout"}]);
        snapshot.borrow_mut()["revision"] = json!(32);
        ui.refresh();
        settle().await;
        assert_eq!(ui.settings_status_row.title(), tr("Sharing needs attention"));
        assert!(ui.settings_status_details.is_visible());
        assert_eq!(find(&ui.settings_body, "setting:search").unwrap(), search.clone().upcast::<gtk::Widget>());
        ui.window.set_default_size(480, 600);
        glib::timeout_future(Duration::from_millis(400)).await;
        capture(&ui, "settings-apply-error.png");
        ui.settings_status_details.emit_clicked();
        assert_eq!(ui.stack.visible_child_name().as_deref(), Some("hardware"));
        snapshot.borrow_mut()["hardware"]["radios"] = json!([{
            "id":"radio-test", "name":"<b>USB & Wi-Fi</b>", "driver":"test <driver>",
            "protected":false, "rfkill":false, "active_connection":"<b>Home & guest</b>"
        }]);
        snapshot.borrow_mut()["revision"] = json!(321);
        ui.refresh();
        settle().await;
        let adapter = find(&ui.hardware, "hardware:radios:radio-test").unwrap().downcast::<adw::ExpanderRow>().unwrap();
        assert!(!adapter.uses_markup(), "External adapter and driver names must be literal");
        adapter.set_expanded(true);
        settle().await;
        for (action, title) in [("test", "Run active hardware test"), ("prefer", "Use as preferred adapter"), ("details", "Full adapter details")] {
            let button = find(&ui.hardware, &format!("hardware:radios:radio-test:{action}")).unwrap();
            let expected = std::ffi::CString::new(format!("{}: <b>USB & Wi-Fi</b>", tr(title))).unwrap();
            let mismatch: Option<glib::GString> = unsafe {
                glib::translate::from_glib_full(gtk::ffi::gtk_test_accessible_check_property(
                    glib::translate::ToGlibPtr::<*mut gtk::ffi::GtkWidget>::to_glib_none(&button).0.cast(),
                    gtk::ffi::GTK_ACCESSIBLE_PROPERTY_LABEL, expected.as_ptr(),
                ))
            };
            assert!(mismatch.is_none(), "Adapter action needs its translated name and adapter: {mismatch:?}");
        }
        let details = find(&ui.hardware, "hardware:radios:radio-test:details").unwrap();
        assert!(details.grab_focus());
        snapshot.borrow_mut()["hardware"]["radios"][0]["rfkill"] = json!(true);
        snapshot.borrow_mut()["revision"] = json!(322);
        ui.refresh();
        settle().await;
        let adapter = find(&ui.hardware, "hardware:radios:radio-test").unwrap().downcast::<adw::ExpanderRow>().unwrap();
        assert!(adapter.is_expanded(), "Inventory updates preserve adapter inspection");
        let details = find(&ui.hardware, "hardware:radios:radio-test:details").unwrap();
        assert_eq!(gtk::prelude::GtkWindowExt::focus(&ui.window), Some(details), "Inventory updates preserve keyboard focus");
        assert!(!find(&ui.hardware, "hardware:radios:radio-test:test").unwrap().is_sensitive(), "A newly blocked adapter cannot be tested");
        capture(&ui, "hardware-inspection.png");
        ui.run_hardware_diagnostic("radio-test");
        settle().await;
        assert!(ui.window.visible_dialog().is_some());
        ui.service_error("test owner loss");
        settle().await;
        assert!(ui.window.visible_dialog().is_none(), "Service loss dismisses active-test confirmation");
        assert_eq!(ui.settings_status_row.title(), tr("Sharing service is offline"));
        assert!(!ui.settings_status_details.is_visible());
        snapshot.borrow_mut()["backends"] = json!([{"id":"localsend","state":"ready","detail":"test listener active"}]);
        snapshot.borrow_mut()["revision"] = json!(33);
        ui.refresh();
        settle().await;
        assert!(!ui.settings_status.is_visible(), "Recovery removes stale apply errors");
        snapshot.borrow_mut()["download_link_active"] = json!(true);
        ui.refresh();
        settle().await;
        ui.stack.set_visible_child_name("transfers");
        settle().await;
        assert!(ui.download_link.is_visible());
        capture(&ui, "active-download-link.png");
        ui.stop_link.emit_clicked();
        settle().await;
        assert!(ui.download_link.is_visible(), "A failed revocation may not hide the active link");
        assert!(ui.stop_link.is_sensitive(), "A failed stop must remain retryable");
        ui.stop_link.emit_clicked();
        settle().await;
        assert!(!ui.download_link.is_visible());
        snapshot.borrow_mut()["transfers"] = json!([{
            "id":"received", "peer_name":"Pixel test fixture", "protocol":"localsend",
            "direction":"incoming", "state":"completed", "total_bytes":0, "transferred_bytes":0,
            "files":[{"name":"empty-valid.txt","size":0}], "saved_paths":[first]
        }]);
        snapshot.borrow_mut()["revision"] = json!(34);
        ui.refresh();
        settle().await;
        let open = find(&ui.transfers, "transfer:received:OpenFiles").unwrap().downcast::<gtk::Button>().unwrap();
        assert_eq!(open.label().as_deref(), Some(tr("Open file").as_str()));
        assert!(find(&ui.transfers, "transfer:received:OpenFolder").is_some());
        // File management remains usable even while the sharing service is offline.
        // Do not launch a file manager or registered application in this isolated test.
        snapshot.borrow_mut()["transfers"][0]["saved_paths"] = json!([first, second]);
        snapshot.borrow_mut()["transfers"][0]["files"] = json!([
            {"name":"empty-valid.txt","size":0}, {"name":"added-during-send.txt","size":13}
        ]);
        snapshot.borrow_mut()["revision"] = json!(35);
        ui.refresh();
        settle().await;
        ui.service_error("test owner loss after completed receive");
        let open = find(&ui.transfers, "transfer:received:OpenFiles").unwrap().downcast::<gtk::Button>().unwrap();
        assert_eq!(open.label().as_deref(), Some(tr("Show received files").as_str()));
        open.emit_clicked();
        settle().await;
        let received = ui.window.visible_dialog().expect("Multi-file completion opens the received file list");
        assert!(find(&received, "received-file-0").unwrap().is_sensitive());
        assert!(find(&received, "received-file-1").unwrap().is_sensitive());
        glib::timeout_future(Duration::from_millis(400)).await;
        capture(&ui, "completed-received-files.png");
        received.force_close();
        ui.refresh();
        settle().await;
        snapshot.borrow_mut()["peers"][0]["display_name"] = json!("<b>Pixel & tablet</b>");
        ui.refresh();
        settle().await;
        let devices = ui.manage_devices();
        let device = find(&devices, "device:pixel").unwrap().downcast::<adw::ExpanderRow>().unwrap();
        assert!(!device.uses_markup(), "Device names must remain literal text");
        assert_eq!(device.title(), "<b>Pixel & tablet</b>");
        device.set_expanded(true);
        let block = find(&devices, "device:pixel:blocked").unwrap().downcast::<adw::SwitchRow>().unwrap();
        let status = find(&devices, "device:pixel:status").unwrap().downcast::<gtk::Label>().unwrap();
        fail_next_preference.set(true);
        block.set_active(true);
        assert!(!device.is_sensitive(), "Pending writes prevent conflicting edits");
        assert_eq!(status.text(), tr("Saving device preference…"));
        glib::timeout_future(Duration::from_millis(400)).await;
        assert!(!block.is_active(), "A failed block must not appear saved");
        assert!(device.is_sensitive() && status.is_visible());
        assert!(status.text().contains("Test preference save failed"));
        assert_eq!(preference_calls.get(), 1, "Rollback must not issue another write");
        capture(&ui, "device-preference-failed.png");
        block.set_active(true);
        glib::timeout_future(Duration::from_millis(400)).await;
        assert!(block.is_active() && !status.is_visible());
        fail_next_preference.set(true);
        block.set_active(false);
        glib::timeout_future(Duration::from_millis(400)).await;
        assert!(block.is_active(), "Rollback uses the latest confirmed value");
        assert_eq!(preference_calls.get(), 3);
        let protocol = find(&devices, "device:pixel:preferred_protocol").unwrap().downcast::<adw::ComboRow>().unwrap();
        protocol.set_selected(2);
        glib::timeout_future(Duration::from_millis(400)).await;
        fail_next_preference.set(true);
        protocol.set_selected(3);
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(protocol.selected(), 2, "Failed protocol change restores the confirmed selection");
        assert_eq!(preference_calls.get(), 5);
        let alias = find(&devices, "device:pixel:display_name").unwrap().downcast::<adw::EntryRow>().unwrap();
        alias.set_text("My tablet");
        fail_next_preference.set(true);
        alias.emit_by_name::<()>("apply", &[]);
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(alias.text(), "My tablet", "Failed alias saves preserve the draft");
        assert_eq!(device.title(), "<b>Pixel & tablet</b>", "The heading keeps the saved name");
        alias.emit_by_name::<()>("apply", &[]);
        glib::timeout_future(Duration::from_millis(400)).await;
        assert_eq!(device.title(), "My tablet");
        assert!(!status.is_visible());
        ui.service_error("test device preference offline");
        ui.proxy.borrow_mut().take();
        let calls = preference_calls.get();
        block.set_active(false);
        settle().await;
        assert!(block.is_active() && device.is_sensitive());
        assert!(status.is_visible());
        assert!(status.text().contains(&tr("Background service unavailable")));
        assert_eq!(preference_calls.get(), calls, "Offline edits must not silently stick");
        devices.force_close();
        // Hold an old owner's snapshot across a real private-bus replacement.
        // Its eventual reply must not revive stale consent or overwrite the replacement.
        snapshot.borrow_mut()["peers"][0]["blocked"] = json!(false);
        let incoming = json!({"id":"stale-consent", "peer_name":"Old owner", "protocol":"localsend", "direction":"incoming", "state":"waiting", "total_bytes":0, "files":[{"name":"old.txt", "size":0}]});
        snapshot.borrow_mut()["transfers"].as_array_mut().unwrap().push(incoming.clone());
        snapshot.borrow_mut()["revision"] = json!(36);
        ui.refresh();
        settle().await;
        ui.add_files(vec![gio::File::for_path(&first)]);
        settle().await;
        let draft = ui.files.borrow().clone();
        let review = ui.accept_request(&incoming);
        settle().await;
        ui.run_hardware_diagnostic("radio-test");
        let old_hardware_consent = ui.window.visible_dialog().unwrap().downcast::<adw::AlertDialog>().unwrap();
        settle().await;
        assert!(find(&ui.transfers, "transfer:stale-consent:AcceptTransfer").unwrap().is_sensitive());
        hold_snapshot.set(true);
        ui.refresh();
        settle().await;
        assert_eq!(pending_snapshots.borrow().len(), 1);
        bus.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "ReleaseName", Some(&(ipc::BUS,).to_variant()), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE).unwrap();
        settle().await;
        assert!(!ui.service_ready.get());
        assert!(!ui.send.is_sensitive() && !ui.share_link.is_sensitive());
        assert!(!find(&ui.transfers, "transfer:stale-consent:AcceptTransfer").unwrap().is_sensitive());
        assert!(find(&ui.transfers, "transfer:received:OpenFiles").unwrap().is_sensitive(), "Saved-file actions remain available without the service");
        assert!(ui.window.visible_dialog().is_none(), "Owner loss closes stale consent");
        let replacement = gio::DBusConnection::for_address_sync(&std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(), gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION, None, gio::Cancellable::NONE).unwrap();
        snapshot.borrow_mut()["epoch"] = json!("replacement-owner");
        snapshot.borrow_mut()["revision"] = json!(1);
        snapshot.borrow_mut()["transfers"].as_array_mut().unwrap().retain(|transfer| transfer["id"] != "stale-consent");
        let state = snapshot.clone();
        let pending = pending_snapshots.clone();
        let hold = hold_snapshot.clone();
        let replacement_registration = replacement.register_object(ipc::PATH, &info.interfaces()[0]).method_call(move |_,_,_,_,method,_,invocation| {
            assert_eq!(method, "GetSnapshot", "Old consent must not reach the replacement owner");
            let response = (state.borrow().to_string(),).to_variant();
            if hold.get() { pending.borrow_mut().push((invocation,response)); } else { invocation.return_value(Some(&response)); }
        }).build().unwrap();
        replacement.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", Some(&(ipc::BUS,0_u32).to_variant()), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE).unwrap();
        settle().await;
        assert_eq!(pending_snapshots.borrow().len(), 2, "New owner refresh must not wait for the old reply");
        assert!(!ui.service_ready.get(), "Owner presence alone is not a current snapshot");
        assert!(!find(&ui.transfers, "transfer:stale-consent:AcceptTransfer").unwrap().is_sensitive());
        hold_snapshot.set(false);
        let (new_call, new_reply) = pending_snapshots.borrow_mut().remove(1);
        new_call.return_value(Some(&new_reply));
        settle().await;
        assert!(ui.service_ready.get());
        assert_eq!(ui.snapshot.borrow()["epoch"], "replacement-owner");
        let (old_call, old_reply) = pending_snapshots.borrow_mut().remove(0);
        old_call.return_value(Some(&old_reply));
        settle().await;
        assert_eq!(ui.snapshot.borrow()["epoch"], "replacement-owner", "Late old snapshots cannot restore stale state");
        assert!(find(&ui.transfers, "transfer:stale-consent:AcceptTransfer").is_none());
        assert_eq!(*ui.files.borrow(), draft, "Owner replacement preserves local file drafts");
        review.emit_by_name::<()>("response", &[&"accept"]);
        old_hardware_consent.emit_by_name::<()>("response", &[&"run"]);
        settle().await;
        replacement.unregister_object(replacement_registration).unwrap();
        ui.allow_close.set(true);
        ui.window.close();
    });
    bus.unregister_object(registration).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
