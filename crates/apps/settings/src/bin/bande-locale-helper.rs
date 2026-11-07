use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const LOCALE_FILE: &str = "/etc/locale.conf";
const DEFAULT_LOCALE_FILE: &str = "/etc/default/locale";
const LOCALE_GEN: &str = "/etc/locale.gen";
const SUPPORTED: &str = "/usr/share/i18n/SUPPORTED";
const KEYBOARD_FILE: &str = "/etc/default/keyboard";
const XKB_RULES: &str = "/usr/share/X11/xkb/rules/evdev.lst";
const SETUPCON: &str = "/usr/bin/setupcon";
const BUILT_IN_LOCALE: &str = "C.UTF-8";
const CATEGORIES: [&str; 6] = [
    "LANG",
    "LC_TIME",
    "LC_NUMERIC",
    "LC_MONETARY",
    "LC_MEASUREMENT",
    "LC_PAPER",
];
const NEW_KEYBOARD: &str = "XKBMODEL=\"pc105\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n";
const MAX_LAYOUTS: usize = 4;

fn supported_names(supported: &str) -> Vec<&str> {
    supported
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(_, charset)| charset.trim() == "UTF-8")
        .map(|(name, _)| name)
        .collect()
}

fn valid_assignment(assignment: &str, names: &[&str]) -> bool {
    let Some((key, value)) = assignment.split_once('=') else {
        return false;
    };
    let known = |value: &str| value == BUILT_IN_LOCALE || names.contains(&value);
    match key {
        "LANG" => known(value),
        "LANGUAGE" => {
            value.is_empty()
                || names
                    .iter()
                    .any(|name| name.replacen(".UTF-8", "", 1) == value)
        }
        _ => CATEGORIES.contains(&key) && (value.is_empty() || known(value)),
    }
}

fn merged_assignments(current: &str, assignments: &[String]) -> String {
    let replaced = |line: &str| {
        line.split_once('=').is_some_and(|(key, _)| {
            assignments
                .iter()
                .any(|assignment| assignment.split_once('=').is_some_and(|(k, _)| k == key))
        })
    };
    let mut merged: Vec<&str> = current.lines().filter(|line| !replaced(line)).collect();
    merged.extend(
        assignments
            .iter()
            .map(String::as_str)
            .filter(|assignment| !assignment.ends_with('=')),
    );
    merged.join("\n") + "\n"
}

fn enabled_locale_gen(current: &str, locales: &[&str]) -> String {
    let mut lines: Vec<String> = current.lines().map(str::to_owned).collect();
    for locale in locales {
        let entry = format!("{locale} UTF-8");
        let enabled = lines.iter().any(|line| line.trim() == entry);
        if enabled {
            continue;
        }
        let commented = lines
            .iter_mut()
            .find(|line| line.trim_start_matches(['#', ' ']).trim_end() == entry);
        match commented {
            Some(line) => line.clone_from(&entry),
            None => lines.push(entry),
        }
    }
    lines.join("\n") + "\n"
}

fn xkb_section<'a>(rules: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let heading = format!("! {name}");
    rules
        .lines()
        .skip_while(move |line| line.trim() != heading)
        .skip(1)
        .take_while(|line| !line.starts_with('!'))
}

fn parse_code(code: &str) -> (&str, &str) {
    code.strip_suffix(')')
        .and_then(|code| code.split_once('('))
        .unwrap_or((code, ""))
}

fn valid_code(code: &str, rules: &str) -> bool {
    let (layout, variant) = parse_code(code);
    let mut layouts = xkb_section(rules, "layout");
    if !layouts.any(|line| line.split_whitespace().next() == Some(layout)) {
        return false;
    }
    if variant.is_empty() {
        return !code.contains('(');
    }
    let owner = format!("{layout}:");
    xkb_section(rules, "variant").any(|line| {
        let mut words = line.split_whitespace();
        words.next() == Some(variant) && words.next() == Some(owner.as_str())
    })
}

fn keyboard_file(current: &str, codes: &[String]) -> String {
    let (layouts, variants): (Vec<&str>, Vec<&str>) =
        codes.iter().map(|code| parse_code(code)).unzip();
    let base = if current.trim().is_empty() {
        NEW_KEYBOARD
    } else {
        current
    };
    let mut lines: Vec<String> = base
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("XKBLAYOUT=") && !line.starts_with("XKBVARIANT=")
        })
        .map(str::to_owned)
        .collect();
    lines.push(format!("XKBLAYOUT=\"{}\"", layouts.join(",")));
    lines.push(format!("XKBVARIANT=\"{}\"", variants.join(",")));
    lines.join("\n") + "\n"
}

fn real_path(path: &str) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn write_atomically(path: &Path, content: &str) -> io::Result<()> {
    let folder = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let temporary = folder.join(format!(".{}.bande", name.to_string_lossy()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&temporary)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(folder)?.sync_all()
}

fn read_if_present(path: &str) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

fn tool(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

fn run(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{} failed: {status}",
        command.get_program().to_string_lossy()
    )))
}

fn set_locale(assignments: &[String]) -> io::Result<()> {
    let wanted: Vec<&str> = assignments
        .iter()
        .filter(|assignment| !assignment.starts_with("LANGUAGE="))
        .filter_map(|assignment| assignment.split_once('=').map(|(_, value)| value))
        .filter(|value| !value.is_empty() && *value != BUILT_IN_LOCALE)
        .collect();
    let locale_gen = read_if_present(LOCALE_GEN)?;
    let enabled = enabled_locale_gen(&locale_gen, &wanted);
    if enabled != locale_gen {
        write_atomically(&real_path(LOCALE_GEN), &enabled)?;
    }
    if !wanted.is_empty() {
        run(tool("/usr/sbin/locale-gen").arg("--keep-existing"))?;
    }
    let merged = merged_assignments(&read_if_present(LOCALE_FILE)?, assignments);
    let target = real_path(LOCALE_FILE);
    write_atomically(&target, &merged)?;
    if real_path(DEFAULT_LOCALE_FILE) != target {
        write_atomically(&real_path(DEFAULT_LOCALE_FILE), &merged)?;
    }
    Ok(())
}

fn set_keyboard(codes: &[String]) -> io::Result<()> {
    let wanted = keyboard_file(&read_if_present(KEYBOARD_FILE)?, codes);
    write_atomically(&real_path(KEYBOARD_FILE), &wanted)?;
    run(tool("/usr/bin/udevadm").args(["trigger", "--subsystem-match=input", "--action=change"]))?;
    if Path::new(SETUPCON).exists() {
        run(tool(SETUPCON).args(["--force", "--save"]))?;
    }
    Ok(())
}

fn refuse(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn finish(result: io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, assignments)) if command == "locale" && !assignments.is_empty() => {
            let supported = fs::read_to_string(SUPPORTED).unwrap_or_default();
            let names = supported_names(&supported);
            if !assignments
                .iter()
                .all(|assignment| valid_assignment(assignment, &names))
            {
                return refuse("unknown locale");
            }
            finish(set_locale(assignments))
        }
        Some((command, codes))
            if command == "keyboard" && !codes.is_empty() && codes.len() <= MAX_LAYOUTS =>
        {
            let rules = fs::read_to_string(XKB_RULES).unwrap_or_default();
            if !codes.iter().all(|code| valid_code(code, &rules)) {
                return refuse("unknown keyboard layout");
            }
            finish(set_keyboard(codes))
        }
        _ => refuse("bad arguments"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_values_reach_the_system_files() {
        let names = supported_names("sv_SE.UTF-8 UTF-8\nsv_SE ISO-8859-1\n");
        assert!(valid_assignment("LANG=sv_SE.UTF-8", &names));
        assert!(valid_assignment("LC_TIME=", &names));
        assert!(!valid_assignment("LANG=sv_SE", &names));
        assert!(!valid_assignment("LANG=", &names));
        assert!(!valid_assignment("LANG=sv_SE.UTF-8\nLD_PRELOAD=/x", &names));
        assert!(!valid_assignment("LD_PRELOAD=sv_SE.UTF-8", &names));
        let rules = "! layout\n  se  Swedish\n! variant\n  nodeadkeys  se: Swedish\n  dvorak  us: D\n! option\n  dvorak  x\n";
        assert!(valid_code("se", rules));
        assert!(valid_code("se(nodeadkeys)", rules));
        assert!(!valid_code("se(dvorak)", rules));
        assert!(!valid_code("se\"; x", rules));
    }
}
