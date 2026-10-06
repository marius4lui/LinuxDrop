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
        "settings":{"general":{"device_name":"Native review","appearance":"dark","language":"system","close_behavior":"background","autostart":false},"visibility":{"mode":"hidden"},"receive":{"directory":"/tmp","ask_directory":true,"collision_policy":"rename"},"localsend":{"enabled":false}},
        "hardware":{"radios":[],"bluetooth":[]},"backends":[],
        "peers":[{"id":"pixel","name":"Pixel test fixture","platform":"android","available":true,"protocols":["localsend","quickshare"]}],
        "transfers":[{"id":"progress","peer_id":"pixel","peer_name":"Pixel test fixture","protocol":"localsend","direction":"outgoing","state":"transferring","total_bytes":100,"transferred_bytes":10,"files":[{"name":"progress.txt","size":100,"transferred":10}]}]
    })));
    let sent_protocol = Rc::new(RefCell::new(String::new()));
    let accepted_options = Rc::new(RefCell::new(Value::Null));
    let info = gio::DBusNodeInfo::for_xml(&format!("<node><interface name='{}'><method name='GetSnapshot'><arg type='s' direction='out'/></method><method name='PrepareSend'><arg type='as' direction='in'/><arg type='s' direction='out'/></method><method name='StartSend'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='out'/></method><method name='AcceptTransferWithOptions'><arg type='s' direction='in'/><arg type='s' direction='in'/></method><signal name='Changed'><arg type='t'/></signal></interface></node>",ipc::INTERFACE)).unwrap();
    let state = snapshot.clone();
    let sent = sent_protocol.clone();
    let accepted = accepted_options.clone();
    let registration = bus
        .register_object(ipc::PATH, &info.interfaces()[0])
        .method_call(
            move |_, _, _, _, method, parameters, invocation| match method {
                "GetSnapshot" => {
                    invocation.return_value(Some(&(state.borrow().to_string(),).to_variant()))
                }
                "PrepareSend" => {
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        invocation.return_value(Some(&("draft",).to_variant()))
                    });
                }
                "StartSend" => {
                    let (_, _, protocol) = parameters.get::<(String, String, String)>().unwrap();
                    *sent.borrow_mut() = protocol;
                    invocation.return_value(Some(&("sent",).to_variant()));
                }
                "AcceptTransferWithOptions" => {
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
        ui.files
            .borrow_mut()
            .retain(|file| file.path().as_ref() != Some(&root));
        ui.render_files();
        ui.update_send();
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
        ui.stack.set_visible_child_name("send");
        ui.window.set_default_size(1100, 760);
        settle().await;
        capture(&ui, "send-wide-review.png");
        ui.window.set_default_size(480,600);
        ui.stack.set_visible_child_name("transfers");
        let dialog = ui.accept_request(&json!({"id":"incoming-verification","peer_name":"Prüfgerät","protocol":"quickshare","direction":"incoming","state":"verification","verification_code":"1234","total_bytes":100,"receive_directory":"/tmp/Reviewed-Sender","selection_mode":"publish_selected","files":[{"name":"fixture.txt","size":75},{"name":"excluded.txt","size":25}]}));
        settle().await;
        find(&dialog,"incoming-file-1").unwrap().downcast::<gtk::CheckButton>().unwrap().set_active(false);
        capture(&ui,"incoming-verification-review.png");
        assert_eq!(dialog.response_label("accept"),tr("Codes match — accept"));
        dialog.emit_by_name::<()>("response",&[&"accept"]);
        settle().await;
        assert_eq!(accepted_options.borrow()["directory"],"/tmp/Reviewed-Sender","Acceptance must preserve the exact previewed destination, including automatic subfolders");
        assert_eq!(accepted_options.borrow()["selected_indices"],json!([0]),"Quick Share publishes only the selected files");
        dialog.force_close();
        settle().await;
        let download = ui.receive_link(Some("http://192.168.1.20:53317"));
        glib::timeout_future(Duration::from_millis(400)).await;
        let address = find(&download, "download-offer-url").unwrap().downcast::<adw::EntryRow>().unwrap();
        assert_eq!(address.text(), "http://192.168.1.20:53317");
        capture(&ui, "download-offer-review.png");
        download.force_close();
        ui.allow_close.set(true);
        ui.window.close();
    });
    bus.unregister_object(registration).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
