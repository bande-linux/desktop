use std::io;
use std::path::{Path, PathBuf};
use std::thread;

use bande_inbox::Inbox;
use bande_toolkit::Image;
use bande_wallpaper::{Choice, Variant, backdrop, read, render};
use bande_xdg::data_dirs;

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
            let fit = choice.fit;
            let draw = move |path: &Path, look: Look| match look {
                Look::Blurred => backdrop(path, fit, size.0, size.1),
                Look::Sharp => render(path, fit, size.0, size.1),
            };
            for look in looks {
                let image = match draw(&path, look) {
                    Ok(picture) => {
                        let path = path.clone();
                        Image::redrawable(picture.width, picture.height, picture.rgba, move || {
                            draw(&path, look).ok().map(|picture| picture.rgba)
                        })
                    }
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
