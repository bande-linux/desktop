use bande_graphics::{Point, Rect, Shape};
use bande_notify::client::NotificationCenter;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const PATH: &str = "/StatusNotifierItem";
const ICON_SIZE: i32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Sharing,
    Recording,
}

struct Item {
    title: String,
    activity: Activity,
    stop: Box<dyn Fn() + Send + Sync>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        (self.stop)();
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        (self.stop)();
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        "SystemServices"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        match self.activity {
            Activity::Sharing => "bande-screen-sharing",
            Activity::Recording => "bande-screen-recording",
        }
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        &self.title
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(ICON_SIZE, ICON_SIZE, icon(self.activity))]
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::default()
    }
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher",
    gen_async = false,
    blocking_name = "Watcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

pub struct Indicator {
    _connection: Connection,
}

impl Indicator {
    pub fn show(
        activity: Activity,
        title: &str,
        stop: impl Fn() + Send + Sync + 'static,
    ) -> zbus::Result<Self> {
        let connection = Connection::session()?;
        connection.object_server().at(
            PATH,
            Item {
                title: title.to_owned(),
                activity,
                stop: Box::new(stop),
            },
        )?;
        Watcher::new(&connection)?.register_status_notifier_item(PATH)?;
        if let Err(error) =
            NotificationCenter::new(&connection).and_then(|center| center.quiet_while_connected())
        {
            tracing::warn!("cannot keep notification banners away from the shared screen: {error}");
        }
        Ok(Self {
            _connection: connection,
        })
    }
}

fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn rounded_box(x: f32, y: f32, half: (f32, f32), radius: f32) -> f32 {
    let rect = Rect::new(-half.0, -half.1, 2.0 * half.0, 2.0 * half.1);
    Shape::rounded(rect, radius).distance(Point::new(x, y))
}

pub fn icon(activity: Activity) -> Vec<u8> {
    let center = ICON_SIZE as f32 / 2.0;
    let (fill, mark): ([u8; 3], fn(f32, f32) -> f32) = match activity {
        Activity::Recording => ([0xff, 0x3b, 0x30], |x, y| {
            rounded_box(x, y, (5.0, 5.0), 1.5)
        }),
        Activity::Sharing => ([0x0a, 0x84, 0xff], |x, y| {
            rounded_box(x, y + 1.0, (8.0, 5.5), 1.5)
                .max(-rounded_box(x, y + 1.0, (6.0, 3.5), 0.5))
                .min(rounded_box(x, y - 7.0, (4.0, 1.0), 0.5))
        }),
    };
    (0..ICON_SIZE * ICON_SIZE)
        .flat_map(|index| {
            let x = (index % ICON_SIZE) as f32 + 0.5 - center;
            let y = (index / ICON_SIZE) as f32 + 0.5 - center;
            let disc = coverage(x.hypot(y) - (center - 1.0));
            let white = coverage(mark(x, y)) * disc;
            let alpha = disc;
            let channel =
                |color: u8| (f32::from(color) * (1.0 - white) + 255.0 * white).round() as u8;
            [
                (alpha * 255.0).round() as u8,
                channel(fill[0]),
                channel(fill[1]),
                channel(fill[2]),
            ]
        })
        .collect()
}
