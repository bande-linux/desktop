use bande_status::history::{ChargeSample, charge_samples, charging_spans, time_on_battery};

const CHARGING: u32 = 1;
const DISCHARGING: u32 = 2;
const FULL: u32 = 4;

fn history() -> Vec<ChargeSample> {
    charge_samples(vec![
        (10_000, 70.0, DISCHARGING),
        (1_000, 40.0, DISCHARGING),
        (4_000, 35.0, CHARGING),
        (7_000, 100.0, FULL),
        (0, 50.0, DISCHARGING),
        (8_000, 0.0, DISCHARGING),
        (8_200, 96.0, DISCHARGING),
    ])
}

#[test]
fn upowers_records_become_samples_in_time_order_without_empty_readings() {
    let samples = history();

    assert_eq!(
        samples.iter().map(|sample| sample.at).collect::<Vec<_>>(),
        [1_000, 4_000, 7_000, 8_200, 10_000]
    );
    assert_eq!(
        samples
            .iter()
            .map(|sample| sample.plugged_in)
            .collect::<Vec<_>>(),
        [false, true, true, false, false]
    );
}

#[test]
fn time_on_battery_counts_from_the_first_reading_after_unplugging() {
    let samples = history();

    assert_eq!(time_on_battery(&samples, 11_000), Some(2_800));
    assert_eq!(time_on_battery(&samples[..3], 11_000), None);
    assert_eq!(time_on_battery(&samples[..1], 11_000), None);
}

#[test]
fn charging_spans_run_from_plugging_in_to_unplugging_or_to_now() {
    let samples = history();

    assert_eq!(charging_spans(&samples, 11_000), [(4_000, 8_200)]);
    assert_eq!(charging_spans(&samples[..3], 11_000), [(4_000, 11_000)]);
}
