use crate::i18n::tr;
use adw::prelude::*;
use gtk::gio;

pub fn install(app: &adw::Application, window: &adw::ApplicationWindow) {
    for (action, keys) in [
        ("win.shortcuts", "<primary>question"),
        ("win.page::send", "<primary>1"),
        ("win.page::transfers", "<primary>2"),
        ("win.page::hardware", "<primary>3"),
        ("win.page::settings", "<primary>4"),
    ] {
        app.set_accels_for_action(action, &[keys]);
    }
    let action = gio::SimpleAction::new("shortcuts", None);
    let weak = window.downgrade();
    action.connect_activate(move |_, _| {
        let Some(window) = weak.upgrade() else { return };
        // Do not cover an incoming consent or other active dialog.
        if window.visible_dialog().is_some() { return; }
        let group = adw::PreferencesGroup::new();
        for (title, accelerator) in [
            ("Select files", "<primary>o"),
            ("Send view", "<primary>1"),
            ("Transfers", "<primary>2"),
            ("Hardware", "<primary>3"),
            ("Settings", "<primary>comma"),
            ("Keyboard shortcuts", "<primary>question"),
            ("Move between controls", "Tab"),
            ("Move back between controls", "<Shift>Tab"),
            ("Activate the focused control", "space"),
            ("Close a dialog or the Notch", "Escape"),
            ("Close application windows", "<primary>q"),
        ] {
            let row = adw::ActionRow::builder().title(tr(title)).build();
            row.add_suffix(&gtk::ShortcutLabel::new(accelerator));
            group.add(&row);
        }
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let note = adw::PreferencesGroup::builder()
            .description(tr("Closing follows your close-behavior setting. Configure the desktop Notch shortcut in Notch preferences. Sending and accepting always require an explicit action."))
            .build();
        page.add(&note);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&page));
        adw::Dialog::builder()
            .title(tr("Keyboard shortcuts"))
            .content_width(460)
            .content_height(560)
            .child(&toolbar)
            .build()
            .present(Some(&window));
    });
    window.add_action(&action);
}
