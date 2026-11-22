use std::path::PathBuf;

use bande_status::pointing::{PointingDevices, is_input_device};

use bande_watch::{Changes, FolderWatch};

use crate::follow::{Update, Updates};

pub fn follow_pointing(udev_data: PathBuf, updates: Updates) -> std::io::Result<FolderWatch> {
    let folder = udev_data.clone();
    let mut known = PointingDevices::scan(&udev_data);
    let watch = FolderWatch::spawn(Changes::Any, move |paths| {
        if !paths.iter().any(|path| is_input_device(path)) {
            return;
        }
        let found = PointingDevices::scan(&udev_data);
        if found != known {
            known = found;
            updates.send(Update::Pointing(found));
        }
    })?;
    watch.watch(&folder)?;
    Ok(watch)
}
