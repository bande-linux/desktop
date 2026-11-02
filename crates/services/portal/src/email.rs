use std::collections::HashMap;
use std::path::Path;

use bande_xdg::percent_encode;
use zbus::zvariant::{ObjectPath, OwnedValue};

use crate::file_chooser::option;
use crate::request::{OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub attachments: Vec<String>,
}

fn encode(text: &str) -> String {
    percent_encode(Path::new(text)).replace('/', "%2F")
}

pub fn mailto(message: &Message) -> String {
    let addresses = |list: &[String]| {
        list.iter()
            .map(|address| encode(address).replace("%40", "@"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut fields = Vec::new();
    for (key, list) in [("cc", &message.cc), ("bcc", &message.bcc)] {
        if !list.is_empty() {
            fields.push(format!("{key}={}", addresses(list)));
        }
    }
    for (key, value) in [("subject", &message.subject), ("body", &message.body)] {
        if let Some(value) = value {
            fields.push(format!("{key}={}", encode(value)));
        }
    }
    for attachment in &message.attachments {
        fields.push(format!("attachment={}", encode(attachment)));
    }
    let mut uri = format!("mailto:{}", addresses(&message.to));
    if !fields.is_empty() {
        uri.push('?');
        uri.push_str(&fields.join("&"));
    }
    uri
}

fn message(options: &Options) -> Message {
    let list = |key: &str| option::<Vec<String>>(options, key).unwrap_or_default();
    let mut to = list("addresses");
    if let Some(address) = option::<String>(options, "address") {
        to.insert(0, address);
    }
    Message {
        to,
        cc: list("cc"),
        bcc: list("bcc"),
        subject: option(options, "subject"),
        body: option(options, "body"),
        attachments: list("attachments"),
    }
}

pub struct Email;

#[zbus::interface(name = "org.freedesktop.impl.portal.Email")]
impl Email {
    async fn compose_email(
        &self,
        _handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        options: Options,
    ) -> (u32, Results) {
        let uri = mailto(&message(&options));
        let token: Option<String> = option(&options, "activation_token");
        let opened = blocking::unblock(move || bande_xdg::open(&uri, token.as_deref())).await;
        match opened {
            Ok(()) => (SUCCESS, Results::new()),
            Err(error) => {
                tracing::warn!("cannot write the email {app_id} asked for: {error}");
                (OTHER, Results::new())
            }
        }
    }
}
