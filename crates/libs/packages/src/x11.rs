use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

const X11_LIBRARIES: [&str; 6] = [
    "libx11-6",
    "libxt6",
    "libxt6t64",
    "libxaw7",
    "libxm4",
    "libmotif",
];
const X11_GAME_LAUNCHERS: [&str; 2] = ["minecraft-launcher", "multimc"];
const WAYLAND_TOOLKITS: [&str; 7] = [
    "libgtk-3-",
    "libgtk-4-",
    "libqt5gui5",
    "libqt6gui6",
    "libsdl2-2.0",
    "libsdl3-",
    "libwayland-client",
];

fn depends(field: &str) -> impl Iterator<Item = &str> {
    field.split([',', '|']).filter_map(|item| {
        let name = item.split_whitespace().next()?;
        Some(name.split(':').next().unwrap_or(name))
    })
}

fn desktop_java(name: &str) -> bool {
    name == "default-jre"
        || (name.starts_with("java") && name.ends_with("-runtime"))
        || (name.starts_with("openjdk-") && name.ends_with("-jre"))
}

/// Whether a package with these `Depends` and `Pre-Depends` values draws only through X11: it
/// needs an X11 library or a desktop Java runtime (whose AWT is X11 only) and no Wayland toolkit,
/// or it is a game launcher whose games open only on X11.
pub fn needs_x11<'a>(package: &str, fields: impl IntoIterator<Item = &'a str>) -> bool {
    if X11_GAME_LAUNCHERS.contains(&package) {
        return true;
    }
    let names: Vec<&str> = fields.into_iter().flat_map(depends).collect();
    names
        .iter()
        .any(|name| X11_LIBRARIES.contains(name) || desktop_java(name))
        && !names.iter().any(|name| {
            WAYLAND_TOOLKITS
                .iter()
                .any(|toolkit| name.starts_with(toolkit))
        })
}

/// The packages of an APT `Packages` index that draw only through X11.
pub fn x11_only_packages(index: &str) -> HashSet<String> {
    index
        .split("\n\n")
        .filter_map(|stanza| {
            let mut package = None;
            let mut fields = Vec::new();
            for line in stanza.lines() {
                if let Some(name) = line.strip_prefix("Package: ") {
                    package = Some(name.trim());
                } else if let Some(field) = line
                    .strip_prefix("Depends: ")
                    .or_else(|| line.strip_prefix("Pre-Depends: "))
                {
                    fields.push(field);
                }
            }
            let package = package?;
            needs_x11(package, fields).then(|| package.to_owned())
        })
        .collect()
}

fn dpkg_query(arguments: &[&OsStr]) -> Option<String> {
    Command::new("dpkg-query")
        .args(arguments)
        .output()
        .inspect_err(|error| tracing::warn!("cannot run dpkg-query: {error}"))
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether the installed package that owns `file` draws only through X11.
pub fn installed_needs_x11(file: &Path) -> bool {
    let Some(owners) = dpkg_query(&["--search".as_ref(), file.as_os_str()]) else {
        return false;
    };
    let Some(package) = owners
        .lines()
        .filter(|line| !line.starts_with("diversion "))
        .find_map(|line| line.rsplit_once(": "))
        .and_then(|(packages, _)| packages.split(", ").next())
    else {
        return false;
    };
    dpkg_query(&[
        "--show".as_ref(),
        "--showformat=${Depends}\n${Pre-Depends}\n".as_ref(),
        package.as_ref(),
    ])
    .is_some_and(|fields| needs_x11(package, fields.lines()))
}

/// Whether a Flatpak's metadata asks for X11 without Wayland or a fallback.
pub fn flatpak_needs_x11(metadata: &str) -> bool {
    let mut in_context = false;
    for line in metadata.lines().map(str::trim) {
        if line.starts_with('[') {
            in_context = line == "[Context]";
        } else if in_context && let Some(sockets) = line.strip_prefix("sockets=") {
            let sockets: Vec<_> = sockets.split(';').map(str::trim).collect();
            return sockets.contains(&"x11")
                && !sockets.contains(&"wayland")
                && !sockets.contains(&"fallback-x11");
        }
    }
    false
}
