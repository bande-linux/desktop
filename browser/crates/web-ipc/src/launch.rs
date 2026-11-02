use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const PASSED: &[&str] = &[
    "HOME",
    "LANG",
    "LC_ALL",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    "FONTCONFIG_FILE",
    "FONTCONFIG_PATH",
];

#[derive(Debug, Clone)]
pub struct Program {
    pub path: PathBuf,
    pub arguments: Vec<String>,
}

impl Program {
    pub fn beside_executable(name: &str) -> io::Result<Program> {
        let executable = std::env::current_exe()?;
        let directory = executable
            .parent()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(Program {
            path: directory.join(name),
            arguments: Vec::new(),
        })
    }

    pub fn launch(&self, control: OwnedFd, network: Option<OwnedFd>) -> io::Result<Child> {
        let mut command = Command::new(&self.path);
        command
            .args(&self.arguments)
            .env_clear()
            .envs(
                PASSED
                    .iter()
                    .filter_map(|name| Some((name, std::env::var_os(name)?))),
            )
            .stdin(Stdio::from(control))
            .stdout(network.map_or_else(Stdio::null, Stdio::from))
            .stderr(Stdio::inherit());
        command.spawn()
    }
}

pub fn inherited_control() -> io::Result<OwnedFd> {
    std::io::stdin().as_fd().try_clone_to_owned()
}

pub fn inherited_network() -> io::Result<OwnedFd> {
    std::io::stdout().as_fd().try_clone_to_owned()
}
