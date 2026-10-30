use std::path::Path;

pub const DRM_CLASS: &str = "/sys/class/drm";

pub fn connected_displays(drm: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut connected: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (card, connector) = name.split_once('-')?;
            card.strip_prefix("card")?.parse::<u32>().ok()?;
            let folder = entry.path();
            let status = std::fs::read_to_string(folder.join("status")).ok()?;
            if status.trim() != "connected" {
                return None;
            }
            let edid = std::fs::read(folder.join("edid")).unwrap_or_default();
            let identity = match unbound_edid::parse(&edid) {
                Some(edid) => unbound_edid::identity(&edid.make, &edid.model, &edid.serial),
                None => connector.to_owned(),
            };
            Some((connector.to_owned(), identity))
        })
        .collect();
    connected.sort();
    let named: Vec<(&str, String)> = connected
        .iter()
        .map(|(connector, identity)| (connector.as_str(), identity.clone()))
        .collect();
    let distinct = unbound_edid::distinct_identities(&named);
    connected
        .iter()
        .map(|(connector, _)| connector.clone())
        .zip(distinct)
        .collect()
}
