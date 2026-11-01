use std::fmt::Write as _;
use std::io;
use std::path::Path;

use bande_design::{Accent, Color, Contrast, Palette, Scheme, TextRole};

pub const STYLESHEET: &str = "bande.css";
pub const IMPORT: &str = "@import url(\"bande.css\");";

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn variables(scheme: Scheme, accent: Accent) -> [(&'static str, Color); 34] {
    let palette = Palette::generate(scheme, accent, Contrast::Standard);
    [
        ("accent-color", palette.accent_text),
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
        ("card-bg-color", palette.well),
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
        ("bande-well", palette.well),
        ("bande-hairline", palette.hairline),
        ("bande-ink", palette.ink),
        ("bande-ink2", palette.ink2),
        ("bande-ink3", palette.ink3),
        ("bande-danger", palette.danger),
    ]
}

const WINDOW_CONTROLS: &str = "
headerbar {
  box-shadow: none;
}

headerbar windowcontrols.start {
  margin-left: 6px;
}

windowcontrols {
  min-height: 24px;
  padding: 0 1.5px;
  border-radius: 12px;
  border-spacing: 0;
  background-color: var(--bande-well);
}

windowcontrols:backdrop {
  background-color: transparent;
  box-shadow: inset 0 0 0 1px var(--bande-hairline);
}

windowcontrols > button {
  min-width: 21px;
  min-height: 24px;
  margin: 0;
  padding: 0;
  background: none;
  box-shadow: none;
  color: var(--bande-ink2);
}

windowcontrols:backdrop > button {
  color: var(--bande-ink3);
}

windowcontrols > button:hover {
  color: var(--bande-ink);
}

windowcontrols > button > image {
  min-width: 16px;
  min-height: 16px;
  padding: 0;
  border-radius: 8px;
  background: none;
  box-shadow: none;
}

windowcontrols > button.close:hover > image {
  background-color: var(--bande-danger);
  color: white;
}

windowcontrols > button:active > image {
  background-color: color-mix(in srgb, var(--bande-ink) 12%, transparent);
}
";

fn window_title(css: &mut String) {
    let spec = TextRole::WindowTitle.spec();
    let _ = write!(
        css,
        "\nheaderbar .title,\nwindowtitle .title {{\n  font-family: \"{}\";\n  font-size: {}px;\n  font-weight: {};\n}}\n",
        spec.typeface.family(),
        spec.size,
        spec.weight
    );
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
    css.push_str(WINDOW_CONTROLS);
    window_title(&mut css);
    css
}

pub fn gtk3_stylesheet(accent: Accent, scheme: Scheme) -> String {
    let palette = Palette::generate(scheme, accent, Contrast::Standard);
    [
        ("theme_bg_color", palette.chassis),
        ("theme_fg_color", palette.ink),
        ("theme_base_color", palette.sheet),
        ("theme_text_color", palette.ink),
        ("theme_selected_bg_color", palette.accent),
        ("theme_selected_fg_color", palette.on_accent),
        ("borders", palette.edge),
    ]
    .into_iter()
    .fold(String::new(), |mut css, (name, color)| {
        let _ = writeln!(css, "@define-color {name} {};", hex(color));
        css
    })
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
    install_into(&config_home.join("gtk-4.0"), &stylesheet(accent, scheme))?;
    install_into(
        &config_home.join("gtk-3.0"),
        &gtk3_stylesheet(accent, scheme),
    )
}

fn install_into(directory: &Path, sheet: &str) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    replace(&directory.join(STYLESHEET), sheet)?;
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
