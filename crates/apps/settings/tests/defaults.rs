#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;

use accesskit::Role;
use support::{has_text, nodes};
use unbound_settings::{PaneId, SettingsView, Update, choose_default, default_apps};
use unbound_toolkit::{Palette, Scheme, TextSystem};
use unbound_xdg::Apps;

fn app(data: &Path, id: &str, name: &str, types: &str) {
    fs::write(
        data.join("applications").join(id),
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=run %U\nMimeType={types}\n"),
    )
    .expect("entry");
}

#[test]
fn choosing_a_browser_writes_each_web_type_it_opens_and_the_menu_shows_it() {
    let root = tempfile::tempdir().expect("temporary folder");
    let data = root.path().join("data");
    let config = root.path().join("config");
    fs::create_dir_all(data.join("applications")).expect("applications");
    app(
        &data,
        "firefox.desktop",
        "Firefox",
        "text/html;x-scheme-handler/http;x-scheme-handler/https;",
    );
    app(
        &data,
        "org.example.Web.desktop",
        "Web",
        "x-scheme-handler/http;x-scheme-handler/https;",
    );
    app(&data, "editor.desktop", "Editor", "text/plain;");
    let mut apps = Apps::load(vec![data.clone()], std::slice::from_ref(&config));

    choose_default(&mut apps, "web-browser", "firefox.desktop").expect("choose");
    let list = fs::read_to_string(config.join("mimeapps.list")).expect("mimeapps.list");
    for mime in [
        "x-scheme-handler/https",
        "x-scheme-handler/http",
        "text/html",
    ] {
        assert!(list.contains(&format!("{mime}=firefox.desktop;")), "{list}");
    }

    choose_default(&mut apps, "web-browser", "org.example.Web.desktop").expect("choose");
    let list = fs::read_to_string(config.join("mimeapps.list")).expect("mimeapps.list");
    assert!(list.contains("x-scheme-handler/https=org.example.Web.desktop;"));
    assert!(
        list.contains("text/html=firefox.desktop;"),
        "Web does not open HTML files, so Firefox keeps them"
    );

    let defaults = default_apps(&apps);
    let web = defaults
        .iter()
        .find(|role| role.role == "web-browser")
        .expect("web browser");
    assert_eq!(web.apps.len(), 2);
    assert_eq!(web.current.as_deref(), Some("org.example.Web.desktop"));
    let music = defaults
        .iter()
        .find(|role| role.role == "music")
        .expect("music");
    assert!(music.apps.is_empty());

    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.apply(Update::DefaultApps(defaults));
    view.select_pane(PaneId::Apps);
    assert!(nodes(&mut view).iter().any(|(_, node)| {
        node.role() == Role::ComboBox
            && node.label() == Some("Web browser")
            && node.value() == Some("Web")
    }));
    assert!(nodes(&mut view).iter().any(|(_, node)| {
        node.role() == Role::ComboBox
            && node.label() == Some("Text")
            && node.value() == Some("Editor")
    }));
    assert!(has_text(&mut view, "No app installed"));
}
