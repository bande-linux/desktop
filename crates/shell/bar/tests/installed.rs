#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod shell;

use shell::{Desktop, install_apps, write_entry};

#[test]
fn a_running_app_takes_the_name_of_an_entry_installed_while_the_bar_runs() {
    let applications = install_apps("bar-installed");
    let mut desktop = Desktop::new();
    desktop.open_window("org.example.Late");
    desktop.wait_for_label("Late");
    write_entry(&applications, "org.example.Late", "Late Arrival", "true");
    desktop.wait_for_label("Late Arrival");
}
