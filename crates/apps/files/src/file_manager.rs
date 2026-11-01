use bande_inbox::Outbox;
use zbus::blocking::Connection;

const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Folders,
    Items,
    Properties,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowRequest {
    pub show: Show,
    pub uris: Vec<String>,
    pub token: Option<String>,
}

struct FileManager {
    requests: Outbox<ShowRequest>,
}

impl FileManager {
    fn send(&self, show: Show, uris: Vec<String>, startup_id: String) {
        let token = Some(startup_id).filter(|token| !token.is_empty());
        self.requests.send(ShowRequest { show, uris, token });
    }
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.send(Show::Folders, uris, startup_id);
    }

    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.send(Show::Items, uris, startup_id);
    }

    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.send(Show::Properties, uris, startup_id);
    }
}

pub fn serve_file_manager(session: &Connection, requests: Outbox<ShowRequest>) -> zbus::Result<()> {
    session.object_server().at(PATH, FileManager { requests })?;
    session.request_name(NAME)
}
