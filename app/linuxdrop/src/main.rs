mod i18n;
mod ipc;
mod settings;
mod ui;

use adw::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(ipc::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    for (name, description) in [
        ("settings", "Open settings"),
        ("hardware", "Show detected hardware"),
        ("transfers", "Show file transfers"),
        ("notch-drop", "Open the desktop notch drop surface"),
    ] {
        app.add_main_option(
            name,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            description,
            None,
        );
    }
    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name("io.github.marius4lui.LinuxDrop");
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("../resources/style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let quit = gio::SimpleAction::new("quit", None);
        let weak = app.downgrade();
        quit.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.quit();
            }
        });
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<primary>q"]);
        app.set_accels_for_action("win.choose", &["<primary>o"]);
        app.set_accels_for_action("win.settings", &["<primary>comma"]);
    });
    app.connect_command_line(|app, command| {
        let mut args: Vec<String> = command
            .arguments()
            .iter()
            .skip(1)
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let page = if command.options_dict().contains("notch-drop") {
            "notch-drop"
        } else if command.options_dict().contains("settings") {
            "settings"
        } else if command.options_dict().contains("hardware") {
            "hardware"
        } else if command.options_dict().contains("transfers") {
            "transfers"
        } else {
            "send"
        };
        if args.first().is_some_and(|a| a == "send" || a == "open") {
            args.remove(0);
        }
        if args.first().is_some_and(|a| a == "--") {
            args.remove(0);
        }
        let files: Vec<gio::File> = args
            .iter()
            .map(|arg| command.create_file_for_arg(arg))
            .collect();
        if let Some(window) = app
            .windows()
            .iter()
            .find(|w| w.title().as_deref() == Some("LinuxDrop"))
        {
            let _ = window.activate_action("win.page", Some(&page.to_variant()));
            if !files.is_empty() {
                let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
                let _ = window.activate_action("win.add-files", Some(&uris.to_variant()));
            }
            if page != "notch-drop" {
                window.present();
            }
        } else {
            ui::build(app, page, files);
        }
        glib::ExitCode::SUCCESS.into()
    });
    app.run()
}
