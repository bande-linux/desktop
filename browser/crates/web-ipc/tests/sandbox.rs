use std::net::{TcpStream, UdpSocket};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use web_ipc::sandbox;

fn main() -> ExitCode {
    let role = std::env::args().nth(1);
    let failed = match role.as_deref() {
        Some("renderer") => renderer(),
        Some("network") => std::env::args()
            .nth(2)
            .is_none_or(|storage| network(Path::new(&storage))),
        _ => {
            let storage =
                std::env::temp_dir().join(format!("web-ipc-sandbox-{}", std::process::id()));
            let executable = std::env::current_exe().unwrap_or_default();
            let failed = ["renderer", "network"].iter().any(|role| {
                let ran = Command::new(&executable).arg(role).arg(&storage).status();
                !ran.is_ok_and(|status| status.success())
            });
            let _ = std::fs::remove_dir_all(&storage);
            if !failed {
                println!(
                    "sandbox: no file, socket or exec escapes from a renderer or the network process"
                );
            }
            failed
        }
    };
    match failed {
        true => ExitCode::FAILURE,
        false => ExitCode::SUCCESS,
    }
}

fn private() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn bus() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/user/1000"))
        .join("bus")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn report(role: &str, escapes: &[(&str, bool)]) -> bool {
    let mut failed = false;
    for (attempt, escaped) in escapes {
        if *escaped {
            eprintln!("FAIL: the {role} sandbox allowed: {attempt}");
            failed = true;
        }
    }
    failed
}

fn exec_escapes() -> bool {
    let error = Command::new("/bin/false").exec();
    error.raw_os_error().is_none()
}

fn renderer() -> bool {
    let fonts = sandbox::font_directories();
    sandbox::unshare_namespaces();
    if let Err(error) = sandbox::enter(&fonts) {
        eprintln!("cannot enter the renderer sandbox: {error}");
        return true;
    }
    let escapes = [
        (
            "no reading the fonts",
            !fonts
                .iter()
                .filter(|directory| directory.starts_with("/usr/share/fonts"))
                .all(|directory| std::fs::read_dir(directory).is_ok()),
        ),
        ("read /etc/passwd", std::fs::read("/etc/passwd").is_ok()),
        ("read a private file", std::fs::read(private()).is_ok()),
        ("list the home folder", std::fs::read_dir(home()).is_ok()),
        (
            "create a file",
            std::fs::write(std::env::temp_dir().join("web-ipc-escape"), b"x").is_ok(),
        ),
        (
            "connect over TCP",
            TcpStream::connect(("127.0.0.1", 9)).is_ok() || socket_created(libc::AF_INET),
        ),
        ("open a Unix socket", socket_created(libc::AF_UNIX)),
        (
            "connect to the session bus",
            UnixStream::connect(bus()).is_ok(),
        ),
        ("spawn a process", Command::new("/bin/true").spawn().is_ok()),
    ];
    report("renderer", &escapes) || exec_escapes()
}

fn network(storage: &Path) -> bool {
    if std::fs::create_dir_all(storage).is_err() {
        return true;
    }
    if let Err(error) = sandbox::enter_network(Some(storage)) {
        eprintln!("cannot enter the network sandbox: {error}");
        return true;
    }
    let escapes = [
        (
            "no writing its storage",
            std::fs::write(storage.join("cookies"), b"x").is_err(),
        ),
        ("no UDP socket", UdpSocket::bind(("0.0.0.0", 0)).is_err()),
        (
            "no certificates",
            std::fs::read_dir("/etc/ssl/certs").is_err(),
        ),
        ("read /etc/passwd", std::fs::read("/etc/passwd").is_ok()),
        ("read a private file", std::fs::read(private()).is_ok()),
        ("list the home folder", std::fs::read_dir(home()).is_ok()),
        (
            "create a file outside its storage",
            std::fs::write(std::env::temp_dir().join("web-ipc-escape"), b"x").is_ok(),
        ),
        (
            "connect to the session bus",
            UnixStream::connect(bus()).is_ok(),
        ),
        ("spawn a process", Command::new("/bin/true").spawn().is_ok()),
    ];
    report("network", &escapes) || exec_escapes()
}

fn socket_created(domain: libc::c_int) -> bool {
    let family = match domain {
        libc::AF_INET => rustix::net::AddressFamily::INET,
        _ => rustix::net::AddressFamily::UNIX,
    };
    rustix::net::socket(family, rustix::net::SocketType::STREAM, None).is_ok()
}
