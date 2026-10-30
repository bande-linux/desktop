use std::fmt::Write as _;
use std::io;
use std::path::Path;

use unbound_design::{Accent, Color, Contrast, Palette, Scheme};

pub const STYLESHEET: &str = "unbound.css";
pub const IMPORT: &str = "@import url(\"unbound.css\");";

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn variables(scheme: Scheme, accent: Accent) -> [(&'static str, Color); 27] {
    let palette = Palette::generate(scheme, accent, Contrast::Standard);
    [
        ("accent-bg-color", palette.accent),
        ("accent-fg-color", palette.on_accent),
        ("window-bg-color", palette.chassis),
        ("window-fg-color", palette.ink),
        ("view-bg-color", palette.sheet),
        ("view-fg-color", palette.ink),
        ("headerbar-bg-color", palette.chassis),
        ("headerbar-fg-color", palette.ink),
        ("headerbar-backdrop-color", palette.chassis),
        ("sidebar-bg-color", palette.chassis),
        ("sidebar-fg-color", palette.ink),
        ("sidebar-backdrop-color", palette.chassis),
        ("secondary-sidebar-bg-color", palette.chassis),
        ("secondary-sidebar-fg-color", palette.ink),
        ("secondary-sidebar-backdrop-color", palette.chassis),
        ("card-bg-color", palette.sheet),
        ("card-fg-color", palette.ink),
        ("dialog-bg-color", palette.sheet),
        ("dialog-fg-color", palette.ink),
        ("popover-bg-color", palette.overlay),
        ("popover-fg-color", palette.ink),
        ("thumbnail-bg-color", palette.overlay),
        ("thumbnail-fg-color", palette.ink),
        ("destructive-bg-color", palette.danger),
        ("error-bg-color", palette.danger),
        ("success-bg-color", palette.success),
        ("warning-bg-color", palette.warning),
    ]
}

fn block(css: &mut String, indent: &str, scheme: Scheme, accent: Accent) {
    let _ = writeln!(css, "{indent}:root {{");
    for (name, color) in variables(scheme, accent) {
        let _ = writeln!(css, "{indent}  --{name}: {};", hex(color));
    }
    let _ = writeln!(css, "{indent}}}");
}

pub fn stylesheet(accent: Accent, current: Scheme) -> String {
    let mut css = String::new();
    block(&mut css, "", current, accent);
    for (query, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let _ = writeln!(css, "\n@media (prefers-color-scheme: {query}) {{");
        block(&mut css, "  ", scheme, accent);
        css.push_str("}\n");
    }
    css
}

pub fn with_import(user: &str) -> Option<String> {
    if user.lines().any(|line| line.trim() == IMPORT) {
        return None;
    }
    Some(format!("{IMPORT}\n{user}"))
}

fn replace(path: &Path, contents: &str) -> io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    let staging = path.with_extension("css.new");
    std::fs::write(&staging, contents)?;
    std::fs::rename(staging, path)
}

pub fn install(config_home: &Path, accent: Accent, scheme: Scheme) -> io::Result<()> {
    let directory = config_home.join("gtk-4.0");
    std::fs::create_dir_all(&directory)?;
    replace(&directory.join(STYLESHEET), &stylesheet(accent, scheme))?;
    let user_path = directory.join("gtk.css");
    let user = match std::fs::read_to_string(&user_path) {
        Ok(user) => user,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    match with_import(&user) {
        Some(imported) => replace(&user_path, &imported),
        None => Ok(()),
    }
}
