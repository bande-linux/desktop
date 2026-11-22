mod access;
mod compose_view;
mod format;
mod message_view;
mod paint;
mod reader;
mod reader_pane;
mod settings_view;
mod sidebar;
mod symbols;
mod toolbar;
mod view;

use bande_mail_core::sheet as setup_sheet;

pub use compose_view::{COMPOSE_SIZE, ComposeRequest, ComposeView};
pub use message_view::{MESSAGE_SIZE, MessageView};
pub use settings_view::{SETTINGS_SIZE, SettingsView};
pub use setup_sheet::Mode;
pub use sidebar::{Place, Row};
pub use view::{Hit, LIST_WIDTH, MailView, Request, SHORTCUTS, SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
