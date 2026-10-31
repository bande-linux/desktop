use std::path::PathBuf;

use unbound_xdg::state_home;

#[test]
fn state_home_uses_an_absolute_xdg_state_home_and_otherwise_the_home_folder() {
    unsafe {
        std::env::set_var("HOME", "/home/anna");
        std::env::set_var("XDG_STATE_HOME", "/var/state/anna");
    }
    assert_eq!(state_home(), Some(PathBuf::from("/var/state/anna")));
    unsafe { std::env::set_var("XDG_STATE_HOME", "relative/state") };
    assert_eq!(state_home(), Some(PathBuf::from("/home/anna/.local/state")));
    unsafe { std::env::remove_var("XDG_STATE_HOME") };
    assert_eq!(state_home(), Some(PathBuf::from("/home/anna/.local/state")));
}
