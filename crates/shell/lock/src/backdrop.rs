use std::io;
use std::path::PathBuf;
use std::thread;

use unbound_toolkit::Image;
use unbound_wallpaper::{Choice, Inbox, Variant, backdrop, inbox, read};
use unbound_xdg::data_dirs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdropSource {
    Session,
    Default,
}

pub fn load_backdrop(
    source: BackdropSource,
    variant: Variant,
    size: (u32, u32),
) -> io::Result<Inbox<Image>> {
    let (received, sender) = inbox()?;
    thread::Builder::new()
        .name("backdrop".to_owned())
        .spawn(move || {
            let choice = match source {
                BackdropSource::Session => zbus::blocking::Connection::session()
                    .map(|session| read(&session))
                    .unwrap_or_default(),
                BackdropSource::Default => Choice::default(),
            };
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let picture = choice
                .path(variant, home.as_deref(), &data_dirs())
                .and_then(|path| match backdrop(&path, choice.fit, size.0, size.1) {
                    Ok(picture) => Image::from_rgba(picture.width, picture.height, picture.rgba),
                    Err(error) => {
                        tracing::warn!("cannot show the wallpaper {}: {error}", path.display());
                        None
                    }
                });
            if let Some(image) = picture {
                sender.send(image);
            }
        })?;
    Ok(received)
}
