use std::collections::HashMap;
use std::ffi::OsString;
use std::os::fd::BorrowedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use bande_inbox::{Inbox, Outbox};
use bande_toolkit::ACTIVATION_TOKEN_VARIABLE;
use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

use crate::Error;

const INTERFACE: &str = "org.freedesktop.Application";
const TOKEN: &str = "activation-token";
const DIRECTORY: &str = "x-bande-directory";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) arguments: Vec<String>,
    pub(crate) directory: Option<PathBuf>,
    pub(crate) token: Option<String>,
}

pub(crate) enum Claim {
    First(Instance),
    Running,
}

pub(crate) struct Instance {
    requests: Inbox<Request>,
}

struct Application {
    requests: Outbox<Request>,
}

fn request(arguments: Vec<String>, platform_data: &HashMap<String, OwnedValue>) -> Request {
    let token = platform_data
        .get(TOKEN)
        .and_then(|token| String::try_from(token.try_clone().ok()?).ok());
    let directory = platform_data
        .get(DIRECTORY)
        .and_then(|directory| Vec::<u8>::try_from(directory.try_clone().ok()?).ok())
        .map(|bytes| PathBuf::from(OsString::from_vec(bytes)));
    Request {
        arguments,
        directory,
        token,
    }
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        self.requests.send(request(Vec::new(), &platform_data));
    }

    fn open(&self, uris: Vec<String>, platform_data: HashMap<String, OwnedValue>) {
        self.requests.send(request(uris, &platform_data));
    }
}

pub(crate) fn object_path(id: &str) -> String {
    format!("/{}", id.replace('.', "/").replace('-', "_"))
}

pub(crate) fn claim(session: &Connection, id: &str) -> Result<Claim, Error> {
    let requests = Inbox::new()?;
    let path = object_path(id);
    session.object_server().at(
        path.as_str(),
        Application {
            requests: requests.outbox(),
        },
    )?;
    match session.request_name_with_flags(id, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
            Ok(Claim::First(Instance { requests }))
        }
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            session
                .object_server()
                .remove::<Application, _>(path.as_str())?;
            Ok(Claim::Running)
        }
        Err(error) => Err(error.into()),
    }
}

impl Request {
    pub(crate) fn from_launch(arguments: Vec<String>) -> Result<Self, Error> {
        let directory = match arguments.is_empty() {
            true => None,
            false => Some(std::env::current_dir()?),
        };
        Ok(Self {
            arguments,
            directory,
            token: std::env::var(ACTIVATION_TOKEN_VARIABLE).ok(),
        })
    }
}

pub(crate) fn forward(session: &Connection, id: &str, request: Request) -> Result<(), Error> {
    let mut platform_data: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(token) = request.token {
        platform_data.insert(TOKEN, Value::from(token));
    }
    if let Some(directory) = &request.directory {
        platform_data.insert(
            DIRECTORY,
            Value::from(directory.as_os_str().as_bytes().to_vec()),
        );
    }
    let path = object_path(id);
    match request.arguments.is_empty() {
        true => session.call_method(
            Some(id),
            path.as_str(),
            Some(INTERFACE),
            "Activate",
            &(platform_data,),
        )?,
        false => session.call_method(
            Some(id),
            path.as_str(),
            Some(INTERFACE),
            "Open",
            &(request.arguments, platform_data),
        )?,
    };
    Ok(())
}

impl Instance {
    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.requests.fd()
    }

    pub(crate) fn take_requests(&mut self) -> Vec<Request> {
        self.requests.drain()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use bande_app_menu::{AppMenu, Entry, MENU_PATH};
    use bande_test_bus::SessionBus;
    use zbus::blocking::fdo::DBusProxy;

    use super::{Claim, Instance, Request, claim, forward, object_path};

    const ID: &str = "org.bande.Example";

    fn wait_for_requests(instance: &mut Instance) -> Vec<Request> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let requests = instance.take_requests();
            if !requests.is_empty() || Instant::now() > deadline {
                return requests;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_object_path_follows_the_app_id_as_the_specification_says() {
        assert_eq!(object_path("org.bande.Files"), "/org/bande/Files");
        assert_eq!(object_path("org.bande.my-app"), "/org/bande/my_app");
    }

    #[test]
    fn a_second_launch_hands_its_arguments_to_the_first_which_keeps_the_name_and_menu() {
        let bus = SessionBus::start().expect("bus");
        let first = bus.connect().expect("first");
        let Claim::First(mut instance) = claim(&first, ID).expect("claim") else {
            panic!("the first launch is not the first instance");
        };
        let _menu = AppMenu::serve(first.clone(), vec![Entry::item("Example", "")]).expect("menu");

        let second = bus.connect().expect("second");
        assert!(matches!(claim(&second, ID).expect("claim"), Claim::Running));
        let open = Request {
            arguments: vec!["notes.txt".to_owned()],
            directory: Some(PathBuf::from("/home/example")),
            token: Some("token-1".to_owned()),
        };
        let activate = Request {
            arguments: Vec::new(),
            directory: None,
            token: None,
        };
        forward(&second, ID, open.clone()).expect("forward");
        forward(&second, ID, activate.clone()).expect("forward");

        assert_eq!(wait_for_requests(&mut instance), [open, activate]);
        let client = bus.connect().expect("client");
        let owner = DBusProxy::new(&client)
            .expect("proxy")
            .get_name_owner(ID.try_into().expect("name"))
            .expect("owner");
        assert_eq!(
            Some(owner.as_ref()),
            first.unique_name().map(|name| name.as_ref())
        );
        client
            .call_method(
                Some(ID),
                MENU_PATH,
                Some("com.canonical.dbusmenu"),
                "GetLayout",
                &(0, -1, Vec::<String>::new()),
            )
            .expect("the menu is still served");
    }
}
