use bande_power::adapter::{AdapterPolicy, POWER_SAVER, Situation};

fn on(battery: bool, low_power: bool) -> Situation {
    Situation {
        on_battery: battery,
        low_power_on_battery: low_power,
        keep_running_with_lid_closed: false,
    }
}

#[test]
fn power_saver_follows_the_battery_and_the_earlier_mode_comes_back_on_power() {
    let mut policy = AdapterPolicy::default();
    assert_eq!(policy.profile_for(on(false, true), "performance"), None);
    assert_eq!(
        policy.profile_for(on(true, true), "performance").as_deref(),
        Some(POWER_SAVER)
    );
    assert_eq!(policy.profile_for(on(true, true), POWER_SAVER), None);
    assert_eq!(
        policy.profile_for(on(false, true), POWER_SAVER).as_deref(),
        Some("performance")
    );
    assert_eq!(policy.profile_for(on(false, true), "performance"), None);
}

#[test]
fn a_mode_chosen_by_hand_on_battery_is_kept_when_power_returns() {
    let mut policy = AdapterPolicy::default();
    assert_eq!(
        policy.profile_for(on(true, true), "balanced").as_deref(),
        Some(POWER_SAVER)
    );
    assert_eq!(policy.profile_for(on(false, true), "performance"), None);
    assert_eq!(policy.profile_for(on(true, false), "balanced"), None);
}

#[test]
fn turning_the_setting_off_on_battery_restores_the_earlier_mode() {
    let mut policy = AdapterPolicy::default();
    policy.profile_for(on(true, true), "balanced");
    assert_eq!(
        policy.profile_for(on(true, false), POWER_SAVER).as_deref(),
        Some("balanced")
    );
}

#[test]
fn the_lid_is_held_only_on_power_when_asked() {
    let keep = |battery| Situation {
        on_battery: battery,
        low_power_on_battery: false,
        keep_running_with_lid_closed: true,
    };
    assert!(AdapterPolicy::holds_lid(keep(false)));
    assert!(!AdapterPolicy::holds_lid(keep(true)));
    assert!(!AdapterPolicy::holds_lid(Situation::default()));
}
