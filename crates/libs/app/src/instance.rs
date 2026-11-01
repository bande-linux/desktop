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
