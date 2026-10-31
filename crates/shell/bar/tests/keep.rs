#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod shell;

use shell::{Desktop, install_apps, write_entry};

#[test]
fn a_window_whose_app_id_names_no_app_takes_the_app_of_its_program() {
    let applications = install_apps("bar-keep");
    let program = std::env::current_exe().expect("test program");
    write_entry(
        &applications,
        "org.example.Tool",
        "Tool",
        &format!("\"{}\"", program.display()),
    );
    let mut desktop = Desktop::new();
    desktop.open_window("org.example.Stray");
    desktop.wait_for_label("Tool");
}
