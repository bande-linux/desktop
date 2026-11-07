use bande_toolkit::ui::{
    Axis, CheckState, Checkbox, PopUpButton, SecureField, Stack, TextField, Widget,
};
use bande_volumes::{SavedServer, load_saved_servers};

use crate::L10N;
use crate::browser::Location;
use crate::controls::{Signal, sender};
use crate::prompt::{Kind, Prompt};
use crate::view::{FilesView, Request, Sheet};

const FIELD_SIZE: f32 = 13.0;
const STACK_GAP: f32 = 8.0;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerRequest {
    pub url: String,
    pub user: String,
    pub password: String,
    pub remember: bool,
    pub reconnect: bool,
    pub trusted_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAnswer {
    pub token: u64,
    pub request: ServerRequest,
    pub status: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerField {
    Address(String),
    Recent(usize),
    User(String),
    Password(String),
    Remember(bool),
    Reconnect(bool),
    Share(usize),
}

impl FilesView {
    pub(crate) fn ask_connect(&mut self) {
        self.ask_connect_to(ServerRequest::default());
    }

    fn ask_connect_to(&mut self, request: ServerRequest) {
        let saved = load_saved_servers();
        let send = sender(&self.signals);
        let address = TextField::new(&L10N.tr("server-address-hint"), &request.url, move |text| {
            send(Signal::Server(ServerField::Address(text.to_owned())));
        });
        let send = sender(&self.signals);
        let reconnect = Checkbox::new(
            &L10N.tr("server-reconnect"),
            match request.reconnect {
                true => CheckState::On,
                false => CheckState::Off,
            },
            move |on| send(Signal::Server(ServerField::Reconnect(on))),
        );
        let mut fields: Vec<Box<dyn Widget>> = vec![Box::new(address)];
        if !saved.is_empty() {
            let send = sender(&self.signals);
            let names: Vec<String> = std::iter::once(L10N.tr("server-recent"))
                .chain(saved.iter().map(|entry| entry.url.clone()))
                .collect();
            fields.push(Box::new(PopUpButton::new("", &names, 0, move |index| {
                if let Some(index) = index.checked_sub(1) {
                    send(Signal::Server(ServerField::Recent(index)));
                }
            })));
        }
        fields.push(Box::new(reconnect));
        let prompt = Prompt::new(&L10N.tr("server-connect-title"), "", self.choice())
            .accessory(Stack::new(Axis::Vertical, STACK_GAP, 0.0, fields))
            .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
            .button(&L10N.tr("server-connect"), "connect", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::ConnectServer { request, saved }, cancel);
    }

    pub(crate) fn connect_to(&mut self, url: String) {
        self.start_connecting(ServerRequest {
            url,
            ..ServerRequest::default()
        });
    }

    pub(crate) fn server_field(&mut self, field: ServerField) {
        match (&mut self.sheet, field) {
            (Some(Sheet::ConnectServer { request, .. }), ServerField::Address(text)) => {
                request.url = text;
            }
            (Some(Sheet::ConnectServer { request, saved }), ServerField::Recent(index)) => {
                if let Some(SavedServer {
                    url,
                    user,
                    reconnect,
                }) = saved.get(index).cloned()
                {
                    let request = ServerRequest {
                        url,
                        user,
                        reconnect,
                        ..std::mem::take(request)
                    };
                    self.dismiss_sheet();
                    self.ask_connect_to(request);
                }
            }
            (Some(Sheet::ConnectServer { request, .. }), ServerField::Reconnect(on)) => {
                request.reconnect = on;
            }
            (Some(Sheet::ServerPassword(request)), ServerField::User(text)) => {
                request.user = text;
            }
            (Some(Sheet::ServerPassword(request)), ServerField::Password(text)) => {
                request.password = text;
            }
            (Some(Sheet::ServerPassword(request)), ServerField::Remember(on)) => {
                request.remember = on;
            }
            (Some(Sheet::ServerShares { chosen, .. }), ServerField::Share(index)) => {
                *chosen = index;
            }
            _ => {}
        }
    }

    pub(crate) fn server_choice(&mut self, sheet: Sheet, action: &str) {
        match sheet {
            Sheet::ConnectServer { request, .. } if action == "connect" => {
                self.start_connecting(request);
            }
            Sheet::ServerPassword(request) if action == "connect" => {
                self.start_connecting(request);
            }
            Sheet::ServerShares {
                mut request,
                shares,
                chosen,
            } if action == "connect" => {
                if let Some(share) = shares.get(chosen) {
                    request.url = format!(
                        "{}/{}",
                        request.url.trim_end_matches('/'),
                        share.replace('%', "%25").replace(' ', "%20")
                    );
                    self.start_connecting(request);
                }
            }
            Sheet::ServerHostKey { mut request, key } if action == "connect" => {
                request.trusted_key = key;
                self.start_connecting(request);
            }
            Sheet::ServerInstall { request, packages } if action == "install" => {
                self.server_token += 1;
                self.requests.push(Request::InstallServerTools {
                    token: self.server_token,
                    request,
                    packages,
                });
                self.show_busy(&L10N.tr("server-installing"));
            }
            Sheet::ServerFailed(request) if action == "retry" => self.start_connecting(request),
            Sheet::ServerBusy => self.server_token += 1,
            _ => {}
        }
    }

    fn show_busy(&mut self, title: &str) {
        let prompt = Prompt::new(title, "", self.choice()).button(
            &L10N.tr("cancel"),
            "cancel",
            Kind::Cancel,
        );
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::ServerBusy, cancel);
    }

    fn start_connecting(&mut self, request: ServerRequest) {
        self.server_token += 1;
        let title = L10N.tr_args(
            "server-connecting",
            [("server", request.url.clone().into())],
        );
        self.requests.push(Request::ConnectServer {
            token: self.server_token,
            request,
        });
        self.show_busy(&title);
    }

    pub fn server_answered(&mut self, answer: ServerAnswer) {
        if answer.token != self.server_token {
            return;
        }
        if matches!(self.sheet, Some(Sheet::ServerBusy)) {
            self.dismiss_sheet();
        }
        let ServerAnswer {
            request,
            status,
            details,
            ..
        } = answer;
        let first = details.first().cloned().unwrap_or_default();
        match status.as_str() {
            "connected" => self.go(Location::Folder(first.into())),
            "password" => self.ask_server_password(request, &first, false),
            "refused" => self.ask_server_password(request, &first, true),
            "shares" => self.ask_share(request, details),
            "host-key" => self.ask_host_key(request, first),
            "host-key-changed" => self.server_failed(
                request,
                L10N.tr("server-key-changed"),
                L10N.tr("server-key-changed-detail"),
                false,
            ),
            "unreachable" => self.server_failed(
                request,
                L10N.tr("server-unreachable"),
                L10N.tr("server-unreachable-detail"),
                true,
            ),
            "no-share" => {
                self.server_failed(request, L10N.tr("server-no-share"), String::new(), false)
            }
            "invalid" => self.server_failed(
                request,
                L10N.tr("server-invalid"),
                L10N.tr("server-address-hint"),
                false,
            ),
            "missing" => self.ask_install(request, details),
            _ => self.server_failed(request, L10N.tr("server-failed"), first, true),
        }
    }

    fn ask_server_password(&mut self, request: ServerRequest, user: &str, refused: bool) {
        let request = ServerRequest {
            user: user.to_owned(),
            password: String::new(),
            remember: true,
            ..request
        };
        let send = sender(&self.signals);
        let name = TextField::new(&L10N.tr("server-user"), user, move |text| {
            send(Signal::Server(ServerField::User(text.to_owned())));
        });
        let edit = sender(&self.signals);
        let submit = sender(&self.signals);
        let password = SecureField::new(&L10N.tr("server-password"), FIELD_SIZE, move |_: &str| {
            submit(Signal::SheetDefault);
        })
        .on_edit(move |text: &str| edit(Signal::Server(ServerField::Password(text.to_owned()))));
        let send = sender(&self.signals);
        let remember = Checkbox::new(&L10N.tr("server-remember"), CheckState::On, move |on| {
            send(Signal::Server(ServerField::Remember(on)))
        });
        let focus = match user.is_empty() {
            true => name.id(),
            false => password.id(),
        };
        let title = L10N.tr_args(
            "server-password-title",
            [("server", request.url.clone().into())],
        );
        let message = match refused {
            true => L10N.tr("server-password-refused"),
            false => String::new(),
        };
        let prompt = Prompt::new(&title, &message, self.choice())
            .accessory(Stack::new(
                Axis::Vertical,
                STACK_GAP,
                0.0,
                vec![Box::new(name), Box::new(password), Box::new(remember)],
            ))
            .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
            .button(&L10N.tr("server-connect"), "connect", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.sheet_primary = prompt.primary_button();
        self.show_sheet(prompt, Sheet::ServerPassword(request), cancel);
        self.focus(focus);
    }

    fn ask_share(&mut self, request: ServerRequest, shares: Vec<String>) {
        if shares.is_empty() {
            self.server_failed(request, L10N.tr("server-no-shares"), String::new(), false);
            return;
        }
        let send = sender(&self.signals);
        let chooser = PopUpButton::new("", &shares, 0, move |index| {
            send(Signal::Server(ServerField::Share(index)));
        });
        let title = L10N.tr_args(
            "server-choose-share",
            [("server", request.url.clone().into())],
        );
        let prompt = Prompt::new(&title, "", self.choice())
            .accessory(chooser)
            .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
            .button(&L10N.tr("server-connect"), "connect", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(
            prompt,
            Sheet::ServerShares {
                request,
                shares,
                chosen: 0,
            },
            cancel,
        );
    }

    fn ask_host_key(&mut self, request: ServerRequest, key: String) {
        let title = L10N.tr_args("server-new-key", [("server", request.url.clone().into())]);
        let detail = L10N.tr_args("server-new-key-detail", [("key", key.clone().into())]);
        let prompt = Prompt::new(&title, &detail, self.choice())
            .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
            .button(&L10N.tr("server-connect"), "connect", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::ServerHostKey { request, key }, cancel);
    }

    fn ask_install(&mut self, request: ServerRequest, packages: Vec<String>) {
        let prompt = Prompt::new(
            &L10N.tr("server-needs-install"),
            &L10N.tr("server-needs-install-detail"),
            self.choice(),
        )
        .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
        .button(&L10N.tr("server-install"), "install", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::ServerInstall { request, packages }, cancel);
    }

    fn server_failed(
        &mut self,
        request: ServerRequest,
        title: String,
        detail: String,
        retry: bool,
    ) {
        let mut prompt = Prompt::new(&title, &detail, self.choice());
        prompt = match retry {
            true => prompt
                .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
                .button(&L10N.tr("try-again"), "retry", Kind::Primary),
            false => prompt.button(&L10N.tr("ok"), "ok", Kind::Primary),
        };
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::ServerFailed(request), cancel);
    }
}
