const UNIT_PATH: &str = "/org/freedesktop/systemd1/unit/";

fn unescape(text: &str, marker: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(marker) {
        bytes.extend_from_slice(&rest.as_bytes()[..index]);
        let after = index + marker.len();
        let byte = rest
            .get(after..after + 2)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match byte {
            Some(byte) => {
                bytes.push(byte);
                rest = &rest[after + 2..];
            }
            None => {
                bytes.extend_from_slice(marker.as_bytes());
                rest = &rest[after..];
            }
        }
    }
    bytes.extend_from_slice(rest.as_bytes());
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn scope_name(app_id: &str, pid: u32) -> String {
    let mut escaped = String::with_capacity(app_id.len());
    for byte in app_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("\\x{byte:02x}"));
        }
    }
    format!("app-bande-{escaped}-{pid}.scope")
}

pub fn unit_app_ids(unit: &str) -> Vec<String> {
    let Some(rest) = unit.strip_prefix("app-") else {
        return Vec::new();
    };
    let name = rest
        .strip_suffix(".scope")
        .and_then(|scope| scope.rsplit_once('-').map(|(name, _)| name))
        .or_else(|| {
            rest.strip_suffix(".service")
                .map(|service| service.split_once('@').map_or(service, |(name, _)| name))
        });
    let Some(name) = name else {
        return Vec::new();
    };
    let mut ids = vec![unescape(name, "\\x")];
    if let Some((_, id)) = name.split_once('-') {
        ids.push(unescape(id, "\\x"));
    }
    ids
}

pub(crate) fn unit_of_object_path(path: &str) -> Option<String> {
    path.strip_prefix(UNIT_PATH)
        .filter(|label| !label.is_empty() && !label.contains('/'))
        .map(|label| unescape(label, "_"))
}
