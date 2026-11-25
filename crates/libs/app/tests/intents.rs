#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, View};
use bande_compositor::Compositor;
use bande_intents::{
    Client, Describe, Dict, Domain, Find, Found, Intent, Limits, Name, Owner, Ref, Refusal, Store,
    Thing, Wanted,
};
use bande_l10n::text;
use bande_test_wayland::{Frames, headless};
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

const PATIENCE: Duration = Duration::from_secs(10);
const ONLY: &str = "pages.page:only";

struct Pane;

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: window.appearance().palette().sheet,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, _event: WindowEvent) -> bool {
        false
    }

    fn set_appearance(&mut self, _appearance: Appearance) {}

    fn animating(&self) -> bool {
        false
    }
}

#[derive(Thing)]
#[thing(id = "pages.page", title = text!("Page"))]
struct Page {
    reference: Ref<Self>,
    title: Name,
}

#[derive(Intent)]
#[intent(id = "pages.find", title = text!("Find Pages"), effects = [Read])]
struct FindPages {
    current: bool,
}

struct Pages;

impl Store for Pages {
    fn look(&self, _reference: &str, _properties: &[String]) -> Result<String, Refusal> {
        Ok(String::new())
    }
}

impl Find<Pages> for FindPages {
    type Thing = Page;

    fn find(self, _: &Pages, _: &Limits) -> Result<Found<Page>, Refusal> {
        Ok(Found::new(vec![Ref::new("only"), Ref::new("other")]))
    }
}

impl Describe<Page> for Pages {
    fn describe(&self, page: &Ref<Page>, _: &Wanted, _: &Limits) -> Result<Page, Refusal> {
        Ok(Page {
            reference: page.clone(),
            title: Name::new("Page"),
        })
    }
}

struct Example;

fn open(cx: &mut Cx<Example>) {
    cx.open(Kind::Main, "Example", (200, 120), |_| Pane)
        .expect("window");
}

impl Program for Example {
    type View = Pane;

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, Error> {
        Ok(Self)
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        open(cx);
    }

    fn intents(_: &bande_app::MainLoop<Self>) -> Option<Owner> {
        Some(
            Owner::new(INFO.id, "/usr/bin/bande-pages")
                .with(Domain::new(|_| Ok(Pages)).find::<FindPages>()),
        )
    }

    fn current(&self, cx: &Cx<Self>, kind: &str) -> Vec<String> {
        match kind == "pages.page" && cx.windows().next().is_some() {
            true => vec![ONLY.to_owned()],
            false => Vec::new(),
        }
    }
}

static INFO: Info = Info {
    id: "org.bande.Pages",
    package: "bande-pages",
    version: "1.2.3",
    name: || "Pages".to_owned(),
    keys: Keys::Standard,
    settings: None,
};

fn found(current: bool) -> Result<usize, Refusal> {
    let client = Client::new(&Connection::session()?);
    let arguments = Dict::from([(
        "current".to_owned(),
        Value::from(current).try_into().expect("an argument"),
    )]);
    client
        .find(INFO.id, "pages.find", &arguments, "", 10)
        .map(|page| page.things.len())
}

fn step(frames: &mut Frames, server: &mut Compositor) {
    let mut stepped = false;
    frames.run_until(server, || std::mem::replace(&mut stepped, true));
}

#[test]
fn an_app_serves_its_intents_without_a_window_until_it_is_opened() {
    let mut server = headless(640, 480);
    let mut frames = Frames::start();
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, vec!["--intents".to_owned()], client_end)
            .map_err(|error| error.to_string())
    });
    let until = Instant::now() + PATIENCE;
    while found(false) != Ok(2) {
        assert!(Instant::now() < until, "the app never answered");
        step(&mut frames, &mut server);
    }
    assert_eq!(found(true), Ok(0), "nothing is shown");
    assert_eq!(server.windows().len(), 0, "it has no window");

    Connection::session()
        .and_then(|session| {
            session.call_method(
                Some(INFO.id),
                "/org/bande/Pages",
                Some("org.freedesktop.Application"),
                "Activate",
                &(HashMap::<&str, Value<'_>>::new(),),
            )
        })
        .expect("the app is opened");
    let until = Instant::now() + PATIENCE;
    while server.windows().is_empty() {
        assert!(Instant::now() < until, "the window never opened");
        step(&mut frames, &mut server);
    }
    let asked = thread::spawn(|| found(true));
    while !asked.is_finished() {
        assert!(Instant::now() < until + PATIENCE, "no answer");
        step(&mut frames, &mut server);
    }
    assert_eq!(
        asked.join().expect("an answer"),
        Ok(1),
        "its window shows one"
    );
}
