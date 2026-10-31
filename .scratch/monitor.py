p = 'crates/apps/monitor/src/format.rs'
s = open(p).read()
a = s.index('pub fn format_bytes(bytes: u64) -> String {')
b = s.index('pub fn format_rate')
s = s[:a] + '''pub fn format_bytes(bytes: u64) -> String {
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
        [("value", unbound_l10n::number(bytes as f64 / scale, decimals))],
    )
}

''' + s[b:]
s = s.replace('''pub fn format_percent(percent: f64) -> String {
    format!("{percent:.1}")
}''', '''pub fn format_percent(percent: f64) -> String {
    L10N.decimal(percent, 1)
}''')
open(p, 'w').write(s)
p = 'crates/apps/monitor/src/inspector.rs'
s = open(p).read()
s = s.replace('[("name", name.to_owned().into()), ("pid", pid.into())]', '[("name", name.to_owned().into()), ("pid", pid.to_string().into())]')
open(p, 'w').write(s)
