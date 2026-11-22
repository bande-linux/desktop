#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_installer::{Blockers, Installer, Machine, State, blockers, identity, read};
use bande_packages::{Client, PATH, SERVICE};
use bande_toolkit::{Appearance, TextSystem};
use zbus::blocking::Connection;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::OwnedObjectPath;

const FILES: [(&str, &str); 3] = [
    (
        "DEBIAN/control",
        "Package: tool\nVersion: 1.2-1\nArchitecture: all\nMaintainer: Ada <ada@example.org>\n\
         Installed-Size: 2048\nDescription: A handy tool\n Does handy things.\n",
    ),
    (
        "usr/share/applications/tool.desktop",
        "[Desktop Entry]\nType=Application\nName=Handy Tool\nExec=tool\nIcon=tool\n",
    ),
    (
        "usr/share/icons/hicolor/scalable/apps/tool.svg",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><rect width=\"16\" height=\"16\"/></svg>",
    ),
];

fn build(dir: &Path) -> PathBuf {
    let root = dir.join("tool");
    for (path, text) in FILES {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let deb = dir.join("tool_1.2-1_all.deb");
    let mut build = Command::new("dpkg-deb");
    build
        .args(["--build", "--root-owner-group"])
        .arg(&root)
        .arg(&deb);
    assert!(build.output().unwrap().status.success());
    deb
}

const SIMULATE: u64 = 1 << 2;
const NOT_AUTHORIZED: u32 = 48;
const EXIT_SUCCESS: u32 = 1;
const EXIT_FAILED: u32 = 2;

struct PackageKit;

#[zbus::interface(name = "org.freedesktop.PackageKit")]
impl PackageKit {
    async fn create_transaction(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> OwnedObjectPath {
        let path = OwnedObjectPath::try_from("/transaction").unwrap();
        let _ = server.remove::<Transaction, _>(&path).await;
        server.at(&path, Transaction).await.unwrap();
        path
    }
}

struct Transaction;

#[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
impl Transaction {
    fn set_hints(&self, _hints: Vec<String>) {}

    async fn resolve(
        &self,
        _filter: u64,
        _names: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        Self::finished(&emitter, EXIT_SUCCESS, 0).await.unwrap();
    }

    async fn install_files(
        &self,
        flags: u64,
        _files: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if flags & SIMULATE != 0 {
            Self::finished(&emitter, EXIT_SUCCESS, 0).await.unwrap();
            return;
        }
        Self::error_code(&emitter, NOT_AUTHORIZED, "Failed to obtain authentication.")
            .await
            .unwrap();
        Self::finished(&emitter, EXIT_FAILED, 0).await.unwrap();
    }

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, exit: u32, runtime: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn error_code(emitter: &SignalEmitter<'_>, code: u32, details: &str) -> zbus::Result<()>;
}

fn ready(installer: &mut Installer) {
    let start = Instant::now();
    while !matches!(installer.state(), State::Ready(_)) {
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(10));
        installer.pump();
    }
    installer.ui().scene(520, 480);
}

#[test]
fn a_built_deb_opens_with_its_fields_and_a_truncated_download_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let deb = build(dir.path());
    let opened = read(&deb).unwrap();
    assert_eq!(
        (
            opened.name.as_str(),
            opened.version.as_str(),
            opened.arch.as_str()
        ),
        ("tool", "1.2-1", "all")
    );
    assert_eq!(
        (opened.installed_size, opened.summary.as_str()),
        (2048 * 1024, "A handy tool")
    );
    let shown = identity(&deb).unwrap();
    assert_eq!(
        (shown.name.as_str(), shown.icon.is_some()),
        ("Handy Tool", true)
    );
    let bytes = std::fs::read(&deb).unwrap();
    let truncated = dir.path().join("partial.deb");
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
    assert_eq!((read(&truncated), identity(&truncated)), (None, None));
}

#[test]
fn a_refused_password_says_so_instead_of_returning_silently_to_install() {
    let daemon = zbus::blocking::connection::Builder::session()
        .unwrap()
        .serve_at(PATH, PackageKit)
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deb = build(dir.path());

    let client = Client::new(&Connection::session().unwrap());
    let mut installer = Installer::new(
        client,
        deb,
        Machine::current().unwrap(),
        Appearance::default().palette(),
        TextSystem::new(),
    )
    .unwrap();
    ready(&mut installer);
    let tree = installer.accessibility();
    let (button, _) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Install"))
        .unwrap();
    installer.ui().perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: *button,
        data: None,
    });
    installer.pump();
    assert!(matches!(installer.state(), State::Installing(_)));

    ready(&mut installer);
    let tree = installer.accessibility();
    assert!(tree.nodes.iter().any(|(_, node)| {
        [node.label(), node.value()]
            .iter()
            .flatten()
            .any(|text| text.starts_with("Nothing was changed"))
    }));
    drop(daemon);
}

#[test]
fn apt_unmet_dependencies_name_what_is_missing_and_which_installed_package_is_in_the_way() {
    let own = "\
The following packages have unmet dependencies:
 tool : PreDepends: libnothere (>= 2) but it is not installable
        Depends: libalso-missing but it is not installable
        Depends: libbar but it is not going to be installed
        Breaks: coreutils (< 99) but 9.12-0.3 is to be installed
E: Unable to satisfy dependencies. Reached two conflicting assignments:
   1. tool:amd64=1.0 is selected for install
   2. tool:amd64 is not selected for install because:
      1. coreutils:amd64 is selected for install
      3. tool:amd64 Breaks coreutils (< 99)
";
    assert_eq!(
        blockers(own, "tool"),
        Blockers {
            missing: vec!["libnothere (>= 2)".to_owned(), "libalso-missing".to_owned()],
            conflicts: vec!["coreutils".to_owned()],
        }
    );
    let theirs = "The following packages have unmet dependencies:\n \
        plugin : Depends: tool (= 1.0) but 2.0 is to be installed\n \
        helper : Breaks: tool (< 3) but 2.0 is to be installed\n \
        tool:i386 : Breaks: oldtool (< 2) but 1.5 is to be installed\n";
    assert_eq!(
        blockers(theirs, "tool"),
        Blockers {
            missing: Vec::new(),
            conflicts: vec![
                "plugin".to_owned(),
                "helper".to_owned(),
                "oldtool".to_owned()
            ],
        }
    );
}
