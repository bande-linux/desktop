use std::sync::Arc;

use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

pub const AGENT_PATH: &str = "/org/unbound/bluetooth/agent";
const CAPABILITY: &str = "KeyboardDisplay";

pub trait PairingPrompt: Send + Sync {
    fn confirm_passkey(&self, device: &str, passkey: u32) -> bool;

    fn show_code(&self, device: &str, code: &str);

    fn ask_pin_code(&self, device: &str) -> Option<String>;

    fn authorize(&self, device: &str) -> bool;

    fn dismiss(&self);
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
}

#[zbus::proxy(
    interface = "org.bluez.AgentManager1",
    default_service = "org.bluez",
    default_path = "/org/bluez"
)]
trait AgentManager {
    fn register_agent(&self, agent: &ObjectPath<'_>, capability: &str) -> zbus::Result<()>;

    fn request_default_agent(&self, agent: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
trait DeviceName {
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;
}

struct Agent {
    bus: zbus::Connection,
    prompt: Arc<dyn PairingPrompt>,
}

impl Agent {
    async fn name(&self, device: &OwnedObjectPath) -> String {
        let alias = async {
            DeviceNameProxy::builder(&self.bus)
                .path(device.clone())?
                .build()
                .await?
                .alias()
                .await
        };
        alias.await.unwrap_or_else(|_| device.to_string())
    }

    async fn ask<T: Send + 'static>(
        &self,
        device: &OwnedObjectPath,
        question: impl FnOnce(&dyn PairingPrompt, &str) -> T + Send + 'static,
    ) -> T {
        let name = self.name(device).await;
        let prompt = Arc::clone(&self.prompt);
        blocking::unblock(move || question(prompt.as_ref(), &name)).await
    }

    fn approve(approved: bool) -> Result<(), AgentError> {
        if approved {
            Ok(())
        } else {
            Err(AgentError::Rejected("the pairing was declined".into()))
        }
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {
        self.prompt.dismiss();
    }

    async fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> {
        self.ask(&device, |prompt, name| prompt.ask_pin_code(name))
            .await
            .ok_or_else(|| AgentError::Rejected("no PIN code was entered".into()))
    }

    async fn display_pin_code(&self, device: OwnedObjectPath, pin_code: String) {
        self.ask(&device, move |prompt, name| {
            prompt.show_code(name, &pin_code)
        })
        .await;
    }

    async fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> {
        self.ask(&device, |prompt, name| prompt.ask_pin_code(name))
            .await
            .and_then(|code| code.trim().parse().ok())
            .filter(|passkey| *passkey <= 999_999)
            .ok_or_else(|| AgentError::Rejected("no passkey was entered".into()))
    }

    async fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) {
        self.ask(&device, move |prompt, name| {
            prompt.show_code(name, &format!("{passkey:06}"))
        })
        .await;
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        Self::approve(
            self.ask(&device, move |prompt, name| {
                prompt.confirm_passkey(name, passkey)
            })
            .await,
        )
    }

    async fn request_authorization(&self, device: OwnedObjectPath) -> Result<(), AgentError> {
        Self::approve(
            self.ask(&device, |prompt, name| prompt.authorize(name))
                .await,
        )
    }

    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        _uuid: String,
    ) -> Result<(), AgentError> {
        Self::approve(
            self.ask(&device, |prompt, name| prompt.authorize(name))
                .await,
        )
    }

    fn cancel(&self) {
        self.prompt.dismiss();
    }
}

pub fn register_agent(system: &Connection, prompt: Arc<dyn PairingPrompt>) -> zbus::Result<()> {
    system.object_server().at(
        AGENT_PATH,
        Agent {
            bus: system.inner().clone(),
            prompt,
        },
    )?;
    let manager = AgentManagerProxyBlocking::new(system)?;
    let path = ObjectPath::try_from(AGENT_PATH)?;
    manager.register_agent(&path, CAPABILITY)?;
    manager.request_default_agent(&path)
}
