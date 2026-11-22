use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use bande_inbox::Inbox;
use bande_toolkit::Image;
use bande_wallpaper::{Choice, Fit, Variant, backdrop, read, render};
use bande_xdg::data_dirs;

#[derive(Debug, Clone)]
pub enum BackdropSource {
    Session,
    Picture(Option<Arc<File>>, Fit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Blurred,
    Sharp,
}

fn opened_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
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
            let (choice, picture) = match source {
                BackdropSource::Session => (
                    bande_bus::session()
                        .map(|session| read(&session))
                        .inspect_err(|error| tracing::warn!("no session bus: {error}"))
                        .unwrap_or_default(),
                    None,
                ),
                BackdropSource::Picture(picture, fit) => {
                    let path = picture.as_deref().map(opened_path);
                    let choice = Choice {
                        light: path.clone(),
                        dark: path,
                        fit,
                    };
                    (choice, picture)
                }
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
                    Ok(drawn) => {
                        let (path, picture) = (path.clone(), picture.clone());
                        Image::redrawable(drawn.width, drawn.height, drawn.rgba, move || {
                            let path = picture.as_deref().map_or_else(|| path.clone(), opened_path);
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

pub fn load_pictures(logins: Vec<String>, size: u32) -> io::Result<Inbox<(usize, Image)>> {
    let received = Inbox::new()?;
    let sender = received.outbox();
    thread::Builder::new()
        .name("picture".to_owned())
        .spawn(move || {
            let system = match bande_bus::system() {
                Ok(system) => system,
                Err(error) => {
                    tracing::warn!("no account pictures without the system bus: {error}");
                    return;
                }
            };
            for (index, login) in logins.iter().enumerate() {
                let Some(path) = bande_status::accounts::picture(&system, login) else {
                    continue;
                };
                let image = match render(&path, Fit::Fill, size, size) {
                    Ok(picture) => Image::from_rgba(picture.width, picture.height, picture.rgba),
                    Err(error) => {
                        tracing::warn!("cannot show {}: {error}", path.display());
                        None
                    }
                };
                if let Some(image) = image
                    && !sender.send((index, image))
                {
                    return;
                }
            }
        })?;
    Ok(received)
}
