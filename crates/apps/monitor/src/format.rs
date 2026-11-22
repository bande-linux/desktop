use bande_l10n::tr;

pub fn format_rate(bytes_per_second: f64) -> String {
    tr!(
        "{size}/s",
        size = bande_fs::format_size(bytes_per_second.round() as u64)
    )
}

pub fn format_percent(percent: f64) -> String {
    format!("{percent:.1}")
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
