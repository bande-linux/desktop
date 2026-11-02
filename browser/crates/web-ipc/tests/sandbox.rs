use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use web_ipc::sandbox;

fn main() -> ExitCode {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let fonts = sandbox::font_directories();
    let private = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let bus = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/user/1000"))
        .join("bus");
    sandbox::unshare_namespaces();
    if let Err(error) = sandbox::enter(&fonts) {
        eprintln!("cannot enter the sandbox: {error}");
        return ExitCode::FAILURE;
    }
    let font_readable = fonts
        .iter()
        .filter(|directory| directory.starts_with("/usr/share/fonts"))
        .all(|directory| std::fs::read_dir(directory).is_ok());
    let escapes = [
        ("read /etc/passwd", std::fs::read("/etc/passwd").is_ok()),
        ("read a private file", std::fs::read(&private).is_ok()),
        ("list the home folder", std::fs::read_dir(&home).is_ok()),
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
            UnixStream::connect(&bus).is_ok(),
        ),
        ("spawn a process", Command::new("/bin/true").spawn().is_ok()),
    ];
    let mut failed = !font_readable;
    if !font_readable {
        eprintln!("FAIL: the font folders are not readable");
    }
    for (attempt, escaped) in escapes {
        if escaped {
            eprintln!("FAIL: a sandboxed process could {attempt}");
            failed = true;
        }
    }
    if failed {
        return ExitCode::FAILURE;
    }
    let error = Command::new("/bin/false").exec();
    println!("sandbox: no file, socket or exec escapes (exec: {error})");
    ExitCode::SUCCESS
}

fn socket_created(domain: libc::c_int) -> bool {
    let family = match domain {
        libc::AF_INET => rustix::net::AddressFamily::INET,
        _ => rustix::net::AddressFamily::UNIX,
    };
    rustix::net::socket(family, rustix::net::SocketType::STREAM, None).is_ok()
}
