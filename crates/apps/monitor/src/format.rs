use crate::L10N;

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1_000 {
        return L10N.tr_args("size-bytes", [("count", bytes.into())]);
    }
    let units = [
        ("size-kilobytes", 1e3, 0_usize),
        ("size-megabytes", 1e6, 1),
        ("size-gigabytes", 1e9, 2),
        ("size-terabytes", 1e12, 2),
    ];
    let (id, scale, decimals) = units
        .into_iter()
        .find(|(_, scale, decimals)| {
            let shift = 10f64.powi(*decimals as i32);
            (bytes as f64 / scale * shift).round() < 1e3 * shift
        })
        .unwrap_or(units[3]);
    L10N.tr_args(
        id,
        [(
            "value",
            unbound_l10n::number(bytes as f64 / scale, decimals),
        )],
    )
}

pub fn format_rate(bytes_per_second: f64) -> String {
    L10N.tr_args(
        "per-second",
        [("size", format_bytes(bytes_per_second.round() as u64).into())],
    )
}

pub fn format_percent(percent: f64) -> String {
    L10N.decimal(percent, 1)
}

pub fn format_cpu_time(seconds: f64) -> String {
    let hundredths = (seconds * 100.0).round() as u64;
    let (hours, minutes) = (hundredths / 360_000, hundredths / 6_000 % 60);
    let seconds = hundredths % 6_000;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{:02}", seconds / 100)
    } else {
        format!("{minutes}:{:02}.{:02}", seconds / 100, seconds % 100)
    }
}

pub fn format_hours(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 3600, seconds / 60 % 60)
}
