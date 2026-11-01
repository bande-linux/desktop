#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod shell;

use shell::{Desktop, install_apps, write_entry};

#[test]
fn the_file_chooser_and_the_shell_dialogs_get_no_dock_tile() {
    let applications = install_apps("bar-dialogs");
    write_entry(&applications, "org.example.After", "After", "true");
    let mut desktop = Desktop::new();
    desktop.open_window("org.bande.FileChooser");
    desktop.open_window("org.bande.MenuBar");
    desktop.open_window("org.example.After");
    let labels = desktop.wait_for_label("After");
    assert!(
        !labels
            .iter()
            .any(|label| label == "FileChooser" || label == "MenuBar"),
        "{labels:?}"
    );
}
