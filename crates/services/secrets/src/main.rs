use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use bande_secrets::dialog::PROGRAM;
use bande_secrets::service::Daemon;
use bande_secrets::store::Keychain;
use bande_secrets::{lock, network, pam};
use rustix::process::{DumpableBehavior, Resource, Rlimit, set_dumpable_behavior, setrlimit};

fn harden() -> rustix::io::Result<()> {
    set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )
}

async fn serve() -> Result<(), Box<dyn Error>> {
    let data_home = bande_xdg::data_home().ok_or("no home folder")?;
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("no XDG_RUNTIME_DIR")?;
    let listener = pam::listener(&runtime_dir)?;
    let daemon = Daemon::new(Keychain::load(data_home).await, PathBuf::from(PROGRAM));
    let connection = zbus::connection::Builder::session()?.build().await?;
    daemon.serve(&connection).await?;
    match zbus::connection::Builder::system()?.build().await {
        Ok(system) => {
            let executor = connection.executor();
            let (agent, locks) = (daemon.clone(), daemon.clone());
            let watched = system.clone();
            executor
                .spawn(
                    async move {
                        if let Err(error) = network::serve(agent, system).await {
                            tracing::warn!("the network secret agent stopped: {error}");
                        }
                    },
                    "bande-secrets network",
                )
                .detach();
            executor
                .spawn(
                    async move {
                        if let Err(error) = lock::revoke_grants_on_lock(locks, watched).await {
                            tracing::warn!("cannot follow the lock screen: {error}");
                        }
                    },
                    "bande-secrets lock",
                )
                .detach();
        }
        Err(error) => tracing::warn!("no system bus for the network secret agent: {error}"),
    }
    pam::serve(daemon, connection, listener).await?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-secrets");
    if let Err(error) = harden() {
        tracing::error!("cannot keep secrets out of core dumps: {error}");
        return ExitCode::FAILURE;
    }
    match async_io::block_on(serve()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
