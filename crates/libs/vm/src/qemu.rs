use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bande_xdg::find_executable;
use serde_json::{Value, json};

use crate::library::Machine;
use crate::manifest::{MAX_DISPLAYS, Network, Preset};
use crate::passthrough::tap_name;
use crate::{Error, Result};

struct Platform {
    qemu: &'static str,
    qemu_package: &'static str,
    firmware_code: &'static str,
    firmware_vars: &'static str,
    secure_code: &'static str,
    secure_vars: &'static str,
    firmware_package: &'static str,
    machine: &'static str,
    secure_machine: &'static str,
    secure_globals: &'static [&'static str],
    emulated_cpu: &'static str,
    gpu: &'static str,
    gpu_3d: &'static str,
    basic_display: &'static str,
    tpm: &'static str,
    install: &'static str,
}

#[cfg(target_arch = "x86_64")]
const PLATFORM: Platform = Platform {
    qemu: "qemu-system-x86_64",
    qemu_package: "qemu-system-x86",
    firmware_code: "/usr/share/OVMF/OVMF_CODE_4M.fd",
    firmware_vars: "/usr/share/OVMF/OVMF_VARS_4M.fd",
    secure_code: "/usr/share/OVMF/OVMF_CODE_4M.ms.fd",
    secure_vars: "/usr/share/OVMF/OVMF_VARS_4M.ms.fd",
    firmware_package: "ovmf",
    machine: "q35",
    secure_machine: "q35,smm=on",
    secure_globals: &[
        "-global",
        "ICH9-LPC.disable_s3=1",
        "-global",
        "driver=cfi.pflash01,property=secure,value=on",
    ],
    emulated_cpu: "max",
    gpu: "virtio-vga",
    gpu_3d: "virtio-vga-gl",
    basic_display: "VGA",
    tpm: "tpm-crb",
    install: "sudo apt install qemu-system-x86 ovmf qemu-system-modules-opengl qemu-utils passt",
};

#[cfg(target_arch = "aarch64")]
const PLATFORM: Platform = Platform {
    qemu: "qemu-system-aarch64",
    qemu_package: "qemu-system-arm",
    firmware_code: "/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd",
    firmware_vars: "/usr/share/AAVMF/AAVMF_VARS.fd",
    secure_code: "/usr/share/AAVMF/AAVMF_CODE.ms.fd",
    secure_vars: "/usr/share/AAVMF/AAVMF_VARS.ms.fd",
    firmware_package: "qemu-efi-aarch64",
    machine: "virt,gic-version=3",
    secure_machine: "virt,gic-version=3",
    secure_globals: &[],
    emulated_cpu: "max,pauth-impdef=on",
    gpu: "virtio-gpu-pci",
    gpu_3d: "virtio-gpu-gl-pci",
    basic_display: "ramfb",
    tpm: "tpm-tis-device",
    install: "sudo apt install qemu-system-arm qemu-efi-aarch64 qemu-system-modules-opengl qemu-utils passt",
};

const VIRTIOFSD: &str = "/usr/libexec/virtiofsd";
const RENDER_NODE: &str = "/dev/dri/renderD128";
const SWTPM: &str = "swtpm";
const MIB: u64 = 1024 * 1024;
const OVERHEAD_MIB: u64 = 512;
const TRANSLATION_CACHE_MIB: u64 = 256;
const REQUIRED: [(&str, &str); 5] = [
    (PLATFORM.qemu, PLATFORM.qemu_package),
    ("qemu-img", "qemu-utils"),
    ("passt", "passt"),
    (PLATFORM.firmware_code, PLATFORM.firmware_package),
    (PLATFORM.firmware_vars, PLATFORM.firmware_package),
];
const DBUS_DISPLAY_PACKAGE: &str = "qemu-system-modules-opengl";
const SANDBOX: &str = "on,obsolete=deny,elevateprivileges=deny,spawn=deny,resourcecontrol=deny";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Tcg,
}

#[derive(Clone, Debug)]
pub struct Host {
    pub qemu: PathBuf,
    pub qemu_img: PathBuf,
    pub passt: PathBuf,
    pub firmware_code: PathBuf,
    pub firmware_vars: PathBuf,
    pub accel: Accel,
    pub allow_3d: bool,
}

impl Accel {
    pub fn detect() -> Self {
        match OpenOptions::new().read(true).write(true).open("/dev/kvm") {
            Ok(_) => Self::Kvm,
            Err(_) => Self::Tcg,
        }
    }
}

fn dbus_display() -> PathBuf {
    let architecture = std::env::consts::ARCH;
    PathBuf::from(format!("/usr/lib/{architecture}-linux-gnu/qemu/ui-dbus.so"))
}

fn has_dbus_display() -> bool {
    dbus_display().is_file()
}

impl Host {
    pub const INSTALL: &str = PLATFORM.install;

    pub fn detect() -> Result<Self> {
        let [qemu, qemu_img, passt, firmware_code, firmware_vars] =
            REQUIRED.map(|(path, package)| locate(path).ok_or(Error::Missing(package.into())));
        if !has_dbus_display() {
            return Err(Error::Missing(DBUS_DISPLAY_PACKAGE.into()));
        }
        Ok(Self {
            qemu: qemu?,
            qemu_img: qemu_img?,
            passt: passt?,
            firmware_code: firmware_code?,
            firmware_vars: firmware_vars?,
            accel: Accel::detect(),
            allow_3d: true,
        })
    }

    pub fn missing_packages() -> Vec<&'static str> {
        let mut missing: Vec<&'static str> = REQUIRED
            .iter()
            .filter(|(path, _)| locate(path).is_none())
            .map(|(_, package)| *package)
            .collect();
        if !has_dbus_display() {
            missing.push(DBUS_DISPLAY_PACKAGE);
        }
        missing.dedup();
        missing
    }

    /// The folders that hold what `missing_packages` looks for, to watch for an install.
    pub fn package_folders() -> Vec<PathBuf> {
        let binaries = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        let files = REQUIRED
            .iter()
            .map(|(path, _)| PathBuf::from(path))
            .filter(|path| path.is_absolute())
            .chain([dbus_display()])
            .filter_map(|path| path.parent().map(Path::to_path_buf));
        let mut folders: Vec<PathBuf> = binaries.into_iter().chain(files).collect();
        folders.sort();
        folders.dedup();
        folders
    }

    pub fn render_node() -> Option<PathBuf> {
        Some(PathBuf::from(RENDER_NODE)).filter(|node| node.exists())
    }

    pub fn uses_3d(&self, machine: &Machine) -> bool {
        self.allow_3d
            && machine.manifest.graphics_3d
            && machine.manifest.preset != Preset::SecureBootTpm
            && Self::render_node().is_some()
    }

    pub fn virtiofsd() -> Option<PathBuf> {
        locate(VIRTIOFSD)
    }

    pub fn swtpm() -> Option<PathBuf> {
        locate(SWTPM)
    }

    pub fn memory_max(&self, machine: &Machine) -> u64 {
        let overhead = match self.accel {
            Accel::Kvm => OVERHEAD_MIB,
            Accel::Tcg => OVERHEAD_MIB + TRANSLATION_CACHE_MIB,
        };
        (machine.manifest.memory_mib + overhead) * MIB
    }

    pub fn qemu_command(&self, machine: &Machine) -> Result<Vec<String>> {
        let manifest = &machine.manifest;
        let (accel, cpu) = match self.accel {
            Accel::Kvm => ("kvm".to_owned(), "host"),
            Accel::Tcg => (
                format!("tcg,thread=multi,tb-size={TRANSLATION_CACHE_MIB}"),
                PLATFORM.emulated_cpu,
            ),
        };
        let file = |node: &str, path: &Path, read_only: bool| -> Result<String> {
            Ok(json!({
                "driver": "file",
                "node-name": format!("{node}-file"),
                "filename": utf8(path)?,
                "read-only": read_only,
                "aio": "io_uring",
                "cache": {"direct": true},
                "discard": "unmap",
            })
            .to_string())
        };
        let format = |node: &str, driver: &str, read_only: bool| {
            json!({
                "driver": driver,
                "node-name": node,
                "file": format!("{node}-file"),
                "read-only": read_only,
                "discard": "unmap",
            })
            .to_string()
        };
        let socket_client = |id: &str, path: &Path| -> Result<String> {
            Ok(format!("socket,id={id},path={}", keyval(path)?))
        };
        let socket = |id: &str, path: &Path| -> Result<String> {
            Ok(format!(
                "socket,id={id},path={},server=on,wait=off",
                keyval(path)?
            ))
        };
        let mut argv = vec![utf8(&self.qemu)?.to_owned()];
        let mut push = |options: &[&str]| argv.extend(options.iter().map(|&o| o.to_owned()));
        push(&[
            "-name",
            &format!("guest={},debug-threads=on", keyval_str(&manifest.name)),
        ]);
        push(&["-uuid", &manifest.id, "-nodefaults", "-no-user-config"]);
        push(&["-boot", "menu=on,splash-time=0"]);
        let secure = manifest.preset == Preset::SecureBootTpm;
        let has_shares = !manifest.shares.is_empty();
        let machine_type = match secure {
            true => PLATFORM.secure_machine,
            false => PLATFORM.machine,
        };
        let mut machine_options = format!("{machine_type},pflash0=firmware,pflash1=vars");
        if has_shares {
            machine_options.push_str(",memory-backend=shared-memory");
        }
        push(&["-machine", &machine_options]);
        if secure {
            push(PLATFORM.secure_globals);
        }
        push(&["-accel", &accel, "-cpu", cpu]);
        push(&["-smp", &manifest.cpus.to_string()]);
        push(&["-m", &format!("{}M", manifest.memory_mib)]);
        if has_shares {
            push(&[
                "-object",
                &format!(
                    "memory-backend-memfd,id=shared-memory,size={}M,share=on",
                    manifest.memory_mib
                ),
            ]);
        }
        let code = match secure {
            true => Path::new(PLATFORM.secure_code),
            false => self.firmware_code.as_path(),
        };
        push(&["-blockdev", &file("firmware", code, true)?]);
        push(&["-blockdev", &format("firmware", "raw", true)]);
        push(&["-blockdev", &file("vars", &machine.vars(), false)?]);
        push(&["-blockdev", &format("vars", "qcow2", false)]);
        push(&["-blockdev", &file("disk", &machine.disk(), false)?]);
        push(&["-blockdev", &format("disk", "qcow2", false)]);
        match secure {
            true => push(&["-device", "nvme,drive=disk,serial=velox,bootindex=1"]),
            false => push(&["-device", "virtio-blk-pci,drive=disk,bootindex=1"]),
        }
        if machine.seed().exists() {
            push(&["-blockdev", &file("seed", &machine.seed(), true)?]);
            push(&["-blockdev", &format("seed", "raw", true)]);
            push(&["-device", "virtio-blk-pci,drive=seed"]);
        }
        let netdev = match &manifest.network {
            Network::Shared => format!(
                "stream,id=net,server=off,addr.type=unix,addr.path={}",
                keyval(&machine.passt_socket())?
            ),
            Network::Bridged(_) => format!(
                "tap,id=net,ifname={},script=no,downscript=no",
                tap_name(&manifest.id)
            ),
        };
        push(&["-netdev", &netdev]);
        push(&[
            "-device",
            &format!("virtio-net-pci,netdev=net,mac={}", manifest.mac),
        ]);
        push(&["-device", "virtio-rng-pci", "-device", "virtio-serial-pci"]);
        push(&["-chardev", &socket("agent", &machine.agent_socket())?]);
        push(&[
            "-device",
            "virtserialport,id=agent-port,chardev=agent,name=org.qemu.guest_agent.0",
        ]);
        push(&["-chardev", &socket("qmp", &machine.qmp_socket())?]);
        push(&["-object", "monitor-qmp,id=qmp-monitor,chardev=qmp"]);
        push(&["-chardev", &socket("control", &machine.control_socket())?]);
        push(&["-object", "monitor-qmp,id=control-monitor,chardev=control"]);
        push(&[
            "-chardev",
            &format!(
                "{},logfile={},logappend=off",
                socket("serial", &machine.serial_socket())?,
                keyval(&machine.console_log())?
            ),
        ]);
        push(&["-serial", "chardev:serial"]);
        let accelerated = self.uses_3d(machine);
        let gpu = |device: &str| match manifest.displays {
            0 | 1 => device.to_owned(),
            displays => format!("{device},max_outputs={}", displays.min(MAX_DISPLAYS)),
        };
        match (secure, accelerated) {
            (true, _) => push(&["-device", PLATFORM.basic_display]),
            (false, true) => push(&["-device", &gpu(PLATFORM.gpu_3d)]),
            (false, false) => push(&["-device", &gpu(PLATFORM.gpu)]),
        }
        push(&["-device", "qemu-xhci,id=usb"]);
        push(&["-device", "usb-kbd,bus=usb.0"]);
        push(&["-device", "usb-tablet,bus=usb.0"]);
        if let Some(iso) = &manifest.installer {
            push(&["-blockdev", &file("installer", Path::new(iso), true)?]);
            push(&["-blockdev", &format("installer", "raw", true)]);
            push(&[
                "-device",
                "usb-storage,drive=installer,bus=usb.0,removable=on,bootindex=2",
            ]);
        }
        if secure {
            push(&["-chardev", &socket_client("tpm", &machine.tpm_socket())?]);
            push(&["-tpmdev", "emulator,id=tpm,chardev=tpm"]);
            push(&["-device", &format!("{},tpmdev=tpm", PLATFORM.tpm)]);
        }
        for (index, share) in manifest.shares.iter().enumerate() {
            let id = format!("share{index}");
            push(&[
                "-chardev",
                &socket_client(&id, &machine.share_socket(index))?,
            ]);
            push(&[
                "-device",
                &format!(
                    "vhost-user-fs-pci,chardev={id},tag={}",
                    keyval_str(&share.tag)
                ),
            ]);
        }
        push(&[
            "-chardev",
            "qemu-vdagent,id=vdagent,name=vdagent,clipboard=on,mouse=off",
        ]);
        push(&[
            "-device",
            "virtserialport,chardev=vdagent,name=com.redhat.spice.0",
        ]);
        match accelerated {
            true => push(&[
                "-display",
                &format!("dbus,p2p=yes,gl=on,rendernode={RENDER_NODE}"),
            ]),
            false => push(&["-display", "dbus,p2p=yes,gl=off"]),
        }
        if manifest.resume_from.is_some() {
            push(&["-S"]);
        }
        push(&["-sandbox", SANDBOX]);
        Ok(argv)
    }

    pub(crate) fn share_command(&self, machine: &Machine, index: usize) -> Result<Vec<String>> {
        let virtiofsd = Self::virtiofsd().ok_or(Error::Missing("virtiofsd".into()))?;
        let share = machine
            .manifest
            .shares
            .get(index)
            .ok_or_else(|| Error::NotListed(index.to_string()))?;
        Ok(vec![
            utf8(&virtiofsd)?.to_owned(),
            "--socket-path".into(),
            utf8(&machine.share_socket(index))?.to_owned(),
            "--shared-dir".into(),
            share.path.clone(),
            "--sandbox".into(),
            "namespace".into(),
            "--cache".into(),
            "auto".into(),
        ])
    }

    pub(crate) fn tpm_command(&self, machine: &Machine) -> Result<Vec<String>> {
        let swtpm = Self::swtpm().ok_or(Error::Missing(SWTPM.into()))?;
        Ok(vec![
            utf8(&swtpm)?.to_owned(),
            "socket".into(),
            "--tpm2".into(),
            "--tpmstate".into(),
            format!("dir={}", keyval(&machine.tpm_state())?),
            "--ctrl".into(),
            format!("type=unixio,path={}", keyval(&machine.tpm_socket())?),
            "--terminate".into(),
        ])
    }

    pub fn passt_command(&self, machine: &Machine) -> Result<Vec<String>> {
        let mut argv = vec![
            utf8(&self.passt)?.to_owned(),
            "--quiet".into(),
            "--one-off".into(),
            "--pid".into(),
            utf8(&machine.passt_pid_file())?.to_owned(),
            "--socket".into(),
            utf8(&machine.passt_socket())?.to_owned(),
        ];
        for forward in &machine.manifest.forwards {
            argv.push("--tcp-ports".into());
            argv.push(format!("127.0.0.1/{}:{}", forward.host, forward.guest));
        }
        Ok(argv)
    }

    pub(crate) fn create_overlay(&self, base: &Path, overlay: &Path, size_gib: u64) -> Result<()> {
        self.qemu_img(&[
            "create",
            "-q",
            "-f",
            "qcow2",
            "-F",
            "qcow2",
            "-b",
            utf8(base)?,
            utf8(overlay)?,
            &format!("{size_gib}G"),
        ])
        .map(drop)
    }

    pub(crate) fn import_disk(&self, source: &Path, disk: &Path, size_gib: u64) -> Result<()> {
        self.qemu_img(&["convert", "-q", "-O", "qcow2", utf8(source)?, utf8(disk)?])?;
        let size = self.image_info(disk)?["virtual-size"]
            .as_u64()
            .unwrap_or_default();
        let wanted = size_gib * 1024 * MIB;
        if wanted > size {
            self.qemu_img(&["resize", "-q", utf8(disk)?, &format!("{size_gib}G")])?;
        }
        Ok(())
    }

    pub(crate) fn create_blank(&self, disk: &Path, size_gib: u64) -> Result<()> {
        self.qemu_img(&[
            "create",
            "-q",
            "-f",
            "qcow2",
            utf8(disk)?,
            &format!("{size_gib}G"),
        ])
        .map(drop)
    }

    pub(crate) fn create_vars(&self, vars: &Path) -> Result<()> {
        self.create_vars_for(vars, Preset::Linux)
    }

    pub(crate) fn create_vars_for(&self, vars: &Path, preset: Preset) -> Result<()> {
        let source = match preset {
            Preset::Linux => self.firmware_vars.as_path(),
            Preset::SecureBootTpm => Path::new(PLATFORM.secure_vars),
        };
        self.qemu_img(&[
            "convert",
            "-q",
            "-f",
            "raw",
            "-O",
            "qcow2",
            utf8(source)?,
            utf8(vars)?,
        ])
        .map(drop)
    }

    pub(crate) fn snapshot_image(&self, action: &str, tag: &str, image: &str) -> Result<()> {
        self.qemu_img(&["snapshot", action, tag, image]).map(drop)
    }

    pub(crate) fn copy_image(
        &self,
        source: &Path,
        target: &Path,
        snapshot: Option<&str>,
        base: Option<&Path>,
    ) -> Result<()> {
        let at_snapshot = snapshot.map(|tag| format!("snapshot.name={tag}"));
        let mut arguments = vec!["convert", "-q", "-O", "qcow2"];
        if let Some(at_snapshot) = &at_snapshot {
            arguments.extend(["-l", at_snapshot]);
        }
        if let Some(base) = base.filter(|base| !base.as_os_str().is_empty()) {
            arguments.extend(["-B", utf8(base)?, "-F", "qcow2"]);
        }
        arguments.extend([utf8(source)?, utf8(target)?]);
        self.qemu_img(&arguments).map(drop)
    }

    pub(crate) fn backing_of(&self, image: &Path) -> Result<Option<PathBuf>> {
        let info = self.image_info(image)?;
        Ok(info["full-backing-filename"].as_str().map(PathBuf::from))
    }

    pub(crate) fn state_size(&self, image: &Path, tag: &str) -> Result<u64> {
        let info = self.image_info(image)?;
        Ok(info["snapshots"]
            .as_array()
            .and_then(|snapshots| snapshots.iter().find(|snapshot| snapshot["name"] == tag))
            .and_then(|snapshot| snapshot["vm-state-size"].as_u64())
            .unwrap_or_default())
    }

    fn image_info(&self, image: &Path) -> Result<Value> {
        let output = self.qemu_img(&["info", "-U", "--output=json", utf8(image)?])?;
        Ok(serde_json::from_slice(&output)?)
    }

    fn qemu_img(&self, arguments: &[&str]) -> Result<Vec<u8>> {
        let output = Command::new(&self.qemu_img)
            .args(arguments)
            .stdin(Stdio::null())
            .output()?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        Err(Error::Tool {
            program: "qemu-img".into(),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn locate(path: &str) -> Option<PathBuf> {
    match path.starts_with('/') {
        true => Some(PathBuf::from(path)).filter(|path| path.is_file()),
        false => find_executable(path),
    }
}

pub(crate) fn utf8(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| Error::NotUtf8(path.to_owned()))
}

fn keyval(path: &Path) -> Result<String> {
    Ok(keyval_str(utf8(path)?))
}

fn keyval_str(value: &str) -> String {
    value.replace(',', ",,")
}
