#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PINENTRY: &str = env!("CARGO_BIN_EXE_pinentry-bande");
const FAKE_PROMPT: &str = r#"#!/bin/sh
request=""
while IFS= read -r line && [ -n "$line" ]; do request="$request$line;"; done
printf '%s %s\n' "$1" "$request" >> "$(dirname "$0")/asked"
case "$1 $request" in
    *cancel-me*) echo cancel ;;
    --passphrase*) printf 'allow\tsecret 100%%\n' ;;
    --confirm*) echo not-ok ;;
    *) echo allow ;;
esac
read -r done
"#;

fn script(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn wrapper(folder: &Path) -> PathBuf {
    let prompt = script(folder, "prompt", FAKE_PROMPT);
    script(
        folder,
        "pinentry",
        &format!(
            "#!/bin/sh\nXDG_CURRENT_DESKTOP=X-Bande WAYLAND_DISPLAY=wayland-test exec {PINENTRY} --prompt {} \"$@\"\n",
            prompt.display()
        ),
    )
}

#[test]
fn gpg_agent_gets_passphrases_confirmations_and_cancels_in_the_assuan_protocol() {
    let folder = tempfile::tempdir().unwrap();
    let mut pinentry = Command::new(wrapper(folder.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = pinentry.stdin.take().unwrap();
    let mut output = BufReader::new(pinentry.stdout.take().unwrap());
    let mut greeting = String::new();
    output.read_line(&mut greeting).unwrap();
    assert!(greeting.starts_with("OK Pleased to meet you"));
    let mut exchange = |command: &str| {
        writeln!(input, "{command}").unwrap();
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            let line = line.trim_end().to_owned();
            let last = line.starts_with("OK") || line.starts_with("ERR");
            lines.push(line);
            if last {
                return lines;
            }
        }
    };
    exchange("OPTION ttyname=/dev/pts/1");
    assert_eq!(exchange("SETDESC Unlock%0Athe key %22Alice%22"), ["OK"]);
    assert_eq!(exchange("SETKEYINFO n/0123ABCD"), ["OK"]);
    assert_eq!(exchange("GETPIN"), ["D secret 100%25", "OK"]);
    assert_eq!(exchange("SETREPEAT Repeat:"), ["OK"]);
    assert_eq!(
        exchange("GETPIN"),
        ["S PIN_REPEATED", "D secret 100%25", "OK"]
    );
    assert_eq!(
        exchange("CONFIRM"),
        ["ERR 83886194 Not confirmed <Pinentry>"]
    );
    assert_eq!(exchange("MESSAGE"), ["OK"]);
    exchange("RESET");
    exchange("SETDESC cancel-me");
    assert_eq!(
        exchange("GETPIN"),
        ["ERR 83886179 Operation cancelled <Pinentry>"]
    );
    assert!(exchange("SPAWN /bin/sh")[0].starts_with("ERR 536871187"));
    assert_eq!(exchange("GETINFO flavor"), ["D bande", "OK"]);
    assert_eq!(exchange("BYE"), ["OK closing connection"]);
    drop(input);
    assert!(pinentry.wait().unwrap().success());
    let asked = std::fs::read_to_string(folder.path().join("asked")).unwrap();
    let first = asked.lines().next().unwrap();
    assert_eq!(
        first,
        "--passphrase description\tUnlock\rthe key \"Alice\";"
    );
}

#[test]
fn a_key_generated_through_gpg_is_protected_with_the_passphrase_given() {
    if !Path::new("/usr/bin/gpg-agent").exists() {
        eprintln!("skipping: gpg-agent is not installed");
        return;
    }
    let folder = tempfile::Builder::new()
        .prefix("p")
        .tempdir_in("/tmp")
        .unwrap();
    let home = folder.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pinentry = wrapper(folder.path());
    std::fs::write(
        home.join("gpg-agent.conf"),
        format!("pinentry-program {}\n", pinentry.display()),
    )
    .unwrap();
    let gpg = |arguments: &[&str]| {
        Command::new("gpg")
            .arg("--homedir")
            .arg(&home)
            .arg("--batch")
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let generated = gpg(&[
        "--quick-generate-key",
        "Ann <ann@bande.test>",
        "future-default",
        "default",
        "never",
    ]);
    let asked = std::fs::read_to_string(folder.path().join("asked")).unwrap_or_default();
    let signed = |passphrase: &str| {
        Command::new("gpgconf")
            .arg("--homedir")
            .arg(&home)
            .args(["--reload", "gpg-agent"])
            .status()
            .unwrap();
        gpg(&[
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            passphrase,
            "--clear-sign",
        ])
        .status
        .success()
    };
    let (right, wrong) = (signed("secret 100%"), signed("wrong"));
    let _ = Command::new("gpgconf")
        .arg("--homedir")
        .arg(&home)
        .args(["--kill", "all"])
        .status();
    assert!(generated.status.success(), "{generated:?}");
    assert!(
        asked.contains("--passphrase") && asked.contains("repeat"),
        "{asked}"
    );
    assert!(
        right,
        "the generated key does not open with the passphrase given"
    );
    assert!(!wrong, "the generated key opens with a wrong passphrase");
}
