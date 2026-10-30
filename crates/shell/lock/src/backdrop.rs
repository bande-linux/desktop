use std::io;
use std::path::PathBuf;
use std::thread;

use unbound_inbox::Inbox;
use unbound_toolkit::Image;
use unbound_wallpaper::{Choice, Variant, backdrop, read, render};
use unbound_xdg::data_dirs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackdropSource {
    Session,
    Picture(Option<PathBuf>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Blurred,
    Sharp,
}

pub fn load_backdrop(
    source: BackdropSource,
    variant: Variant,
    size: (u32, u32),
    looks: Vec<Look>,
) -> io::Result<Inbox<(Look, Image)>> {
    let received = Inbox::new()?;
    let sender = received.outbox();
    thread::Builder::new()
        .name("backdrop".to_owned())
        .spawn(move || {
            let choice = match source {
                BackdropSource::Session => zbus::blocking::Connection::session()
                    .map(|session| read(&session))
                    .unwrap_or_default(),
                BackdropSource::Picture(path) => Choice {
                    light: path.clone(),
                    dark: path,
                    ..Choice::default()
                },
            };
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let Some(path) = choice.path(variant, home.as_deref(), &data_dirs()) else {
                return;
            };
            for look in looks {
                let drawn = match look {
                    Look::Blurred => backdrop(&path, choice.fit, size.0, size.1),
                    Look::Sharp => render(&path, choice.fit, size.0, size.1),
                };
                let image = match drawn {
                    Ok(picture) => Image::from_rgba(picture.width, picture.height, picture.rgba),
                    Err(error) => {
                        tracing::warn!("cannot show the wallpaper {}: {error}", path.display());
                        None
                    }
                };
                if let Some(image) = image
                    && !sender.send((look, image))
                {
                    return;
                }
            }
        })?;
    Ok(received)
}
