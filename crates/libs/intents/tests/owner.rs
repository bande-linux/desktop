#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bande_intents::client::text;
use bande_intents::{
    Access, Client, Describe, Dict, Domain, Done, Effect, Files, Find, Found, Intent, Job, Limits,
    List, Manifests, Name, Opened, Owner, Perform, Plan, Reason, Ref, Refs, Refusal, RichText,
    Store, Text, Thing, Undo, Unit, Wanted, Words,
};
use bande_l10n::text;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

const BUS: &str = "org.bande.Test.Books";
const EACH: Duration = Duration::from_millis(40);
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Thing)]
#[thing(id = "books.book", title = text!("Book"))]
struct Book {
    reference: Ref<Self>,
    title: Name,
    #[thing(find, foreign)]
    review: Option<RichText>,
    trashed: Option<bool>,
    foreign: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Words)]
enum Turn {
    #[word("90")]
    Quarter,
    UpsideDown,
}

#[derive(Thing)]
#[thing(id = "books.cover", title = text!("Cover"), view_of = ["books.book"])]
struct Cover {
    reference: Ref<Self>,
    title: Name,
    turn: Option<Turn>,
}

#[derive(Intent)]
#[intent(id = "books.get_cover", title = text!("Get Cover"), effects = [Read])]
struct GetCover {
    cover: Ref<Cover>,
    turn: Option<Turn>,
}

#[derive(Intent)]
#[intent(id = "books.find", title = text!("Find Books"), effects = [Read, ReadForeign])]
struct FindBooks {
    text: Option<Text<256>>,
}

#[derive(Intent)]
#[intent(id = "books.get", title = text!("Get Books"), effects = [Read, ReadForeign])]
struct GetBooks {
    books: Refs<Book>,
    properties: List<Text<64>, 8>,
}

#[derive(Intent)]
#[intent(id = "books.trash", title = text!("Trash Books"), effects = [Change], undo)]
struct TrashBooks {
    books: Refs<Book>,
}

#[derive(Intent)]
#[intent(id = "books.restore", title = text!("Restore Books"), effects = [Change])]
struct RestoreBooks {
    books: Refs<Book>,
}

struct Shelf {
    trashed: Mutex<BTreeMap<String, bool>>,
}

impl Shelf {
    fn open() -> Self {
        Self {
            trashed: Mutex::new((0..5).map(|number| (number.to_string(), false)).collect()),
        }
    }

    fn set(&self, book: &Ref<Book>, trashed: bool) -> Result<(), Refusal> {
        match self.trashed.lock().unwrap().get_mut(book.token()) {
            Some(book) => {
                *book = trashed;
                Ok(())
            }
            None => Err(Refusal::not_found("no such book")
                .candidates(["books.book:1".to_owned(), "books.book:2".to_owned()])),
        }
    }
}

impl Store for Shelf {
    fn look(&self, reference: &str, _properties: &[String]) -> Result<String, Refusal> {
        let token = reference.split(':').next_back().unwrap();
        Ok(format!("{:?}", self.trashed.lock().unwrap().get(token)))
    }
}

impl Find<Shelf> for FindBooks {
    type Thing = Book;

    fn find(self, shelf: &Shelf, limits: &Limits) -> Result<Found<Book>, Refusal> {
        let wanted = self.text.map(|text| text.0);
        Ok(Found::new(
            shelf
                .trashed
                .lock()
                .unwrap()
                .keys()
                .filter(|token| wanted.as_ref().is_none_or(|wanted| wanted == *token))
                .map(|token| Ref::new(token))
                .filter(|book: &Ref<Book>| limits.covers("books", book.as_str()))
                .collect(),
        ))
    }
}

impl Describe<Book> for Shelf {
    fn describe(
        &self,
        book: &Ref<Book>,
        wanted: &Wanted,
        limits: &Limits,
    ) -> Result<Book, Refusal> {
        let trashed = self.trashed.lock().unwrap().get(book.token()).copied();
        let trashed = trashed
            .filter(|_| limits.covers("books", book.as_str()))
            .ok_or_else(|| Refusal::not_found("no such book"))?;
        Ok(Book {
            reference: book.clone(),
            title: Name::new(&format!("Book {}", book.token())),
            review: wanted
                .has("review")
                .then(|| RichText("Buy it at once".to_owned())),
            trashed: Some(trashed),
            foreign: match book.token() {
                "4" => vec!["title"],
                _ => Vec::new(),
            },
        })
    }
}

impl Files for Shelf {
    fn open(
        &self,
        reference: &str,
        _: Access,
        limits: &Limits,
    ) -> Result<(Opened, Vec<Effect>), Refusal> {
        if !limits.covers("books", reference) {
            return Err(Refusal::not_found("no such book"));
        }
        let file =
            std::fs::File::open("/dev/null").map_err(|error| Refusal::failed(error.to_string()))?;
        Ok((
            Opened {
                file: file.into(),
                reference: reference.to_owned(),
                name: "book".to_owned(),
                media_type: "text/plain".to_owned(),
                size: 7,
            },
            vec![Effect::ReadForeign],
        ))
    }
}

impl Describe<Cover> for Shelf {
    fn describe(&self, cover: &Ref<Cover>, wanted: &Wanted, _: &Limits) -> Result<Cover, Refusal> {
        let asked: Option<GetCover> = wanted.arguments()?;
        Ok(Cover {
            reference: cover.clone(),
            title: Name::new("Cover"),
            turn: asked.and_then(|asked| asked.turn),
        })
    }
}

fn plan(books: Refs<Book>) -> Plan<Vec<Ref<Book>>> {
    Plan {
        effects: vec![Effect::Change],
        summary: format!("{} books", books.0.len()),
        rows: Vec::new(),
        references: books.0.iter().map(Ref::to_string).collect(),
        prepared: books.0,
    }
}

impl Perform<Shelf> for TrashBooks {
    type Prepared = Vec<Ref<Book>>;
    type Output = Vec<Book>;

    fn prepare(self, _: &Shelf, _: &Limits) -> Result<Plan<Self::Prepared>, Refusal> {
        Ok(plan(self.books))
    }

    fn perform(
        books: Self::Prepared,
        shelf: &Shelf,
        job: &Job,
    ) -> Result<Done<Vec<Book>>, Refusal> {
        job.expect(Unit::Things, books.len() as u64);
        let mut done: Vec<Ref<Book>> = Vec::new();
        for book in books {
            if job.cancelled() {
                break;
            }
            let restore = RestoreBooks {
                books: Refs(vec![book.clone()]),
            };
            job.keep(Undo::by(restore)?.touching(&book, &["trashed"]))?;
            match shelf.set(&book, true) {
                Ok(()) => done.push(book),
                Err(refusal) => {
                    job.not_done()?;
                    job.failed(book.as_str(), refusal);
                }
            }
            std::thread::sleep(EACH);
            job.advance(1);
        }
        let wanted = Wanted::of(&["trashed"]);
        let things = done
            .iter()
            .map(|book| shelf.describe(book, &wanted, &Limits::default()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Done {
            output: things,
            undo: job.undo()?,
        })
    }
}

impl Perform<Shelf> for RestoreBooks {
    type Prepared = Vec<Ref<Book>>;
    type Output = ();

    fn prepare(self, _: &Shelf, _: &Limits) -> Result<Plan<Self::Prepared>, Refusal> {
        Ok(plan(self.books))
    }

    fn perform(books: Self::Prepared, shelf: &Shelf, _: &Job) -> Result<Done<()>, Refusal> {
        books.iter().try_for_each(|book| shelf.set(book, false))?;
        Ok(Done::new(()))
    }
}

fn owned(value: impl Into<Value<'static>>) -> OwnedValue {
    value.into().try_into().unwrap()
}

fn books(references: &[&str]) -> Dict {
    let references: Vec<String> = references.iter().map(|it| (*it).to_owned()).collect();
    Dict::from([("books".to_owned(), owned(references))])
}

fn granted(effects: &[&str], within: &[&str]) -> Dict {
    let words = |words: &[&str]| -> Value<'static> {
        words
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<String>>()
            .into()
    };
    let grant = HashMap::from([("effects", words(effects)), ("within", words(within))]);
    let domains = HashMap::from([("books", grant)]);
    Dict::from([("domains".to_owned(), owned(Value::from(domains)))])
}

fn trashed(things: &[Dict]) -> usize {
    things
        .iter()
        .filter(|thing| matches!(&*thing["trashed"], Value::Bool(true)))
        .count()
}

#[test]
fn many_things_are_found_in_pages_changed_as_a_set_stopped_and_undone() {
    let folder = tempfile::tempdir().unwrap();
    let owner = Owner::new(BUS, "/usr/bin/bande-books").with(
        Domain::new(|_| Ok(Shelf::open()))
            .find::<FindBooks>()
            .get::<GetBooks, Book>()
            .get::<GetCover, Cover>()
            .perform::<TrashBooks>()
            .private::<RestoreBooks>()
            .files(),
    );
    owner.write_manifest(folder.path()).unwrap();
    let manifests = Manifests::read(folder.path());
    let serving = Connection::session().unwrap();
    let served = owner.serve(&serving).unwrap();
    serving.request_name(BUS).unwrap();

    let connection = Connection::session().unwrap();
    let client = Client::new(&connection);
    let first = client.find(BUS, "books.find", &Dict::new(), "", 2).unwrap();
    let second = client
        .find(BUS, "books.find", &Dict::new(), &first.cursor, 5)
        .unwrap();
    assert_eq!((first.things.len(), second.things.len()), (2, 3));
    assert_eq!(
        (second.cursor.as_str(), first.set),
        ("", second.set.clone())
    );
    assert_eq!(text(&first.things[0], "review"), "Buy it at once");
    let other = Client::new(&Connection::session().unwrap());
    let taken = other.prepare(BUS, "books.trash", &books(&[&second.set]));
    assert_eq!(taken.unwrap_err().reason, Reason::Expired);

    let prepared = client
        .prepare(BUS, "books.trash", &books(&[&second.set]))
        .unwrap();
    let (job, state, _): (zbus::zvariant::OwnedObjectPath, String, Dict) = connection
        .call_method(
            Some(BUS),
            bande_intents::PATH,
            Some("org.bande.Intents"),
            "Perform",
            &prepared.plan,
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert_eq!(state, "running");
    let on_job = |connection: &Connection, member: &str| {
        let (interface, body) = match member {
            "Cancel" => ("org.bande.Intents.Job", None),
            property => ("org.freedesktop.DBus.Properties", Some(property.to_owned())),
        };
        match body {
            None => connection.call_method(Some(BUS), &job, Some(interface), member, &()),
            Some(property) => connection.call_method(
                Some(BUS),
                &job,
                Some(interface),
                "Get",
                &("org.bande.Intents.Job", property),
            ),
        }
    };
    let stranger = Connection::session().unwrap();
    assert!(on_job(&stranger, "Cancel").is_err());
    assert!(on_job(&stranger, "Result").is_err());
    on_job(&connection, "Cancel").unwrap();
    let until = Instant::now() + PATIENCE;
    let state = || -> String {
        let reply = on_job(&connection, "State").unwrap();
        let value: OwnedValue = reply.body().deserialize().unwrap();
        value.try_into().unwrap()
    };
    while state() == "running" {
        assert!(Instant::now() < until, "the job ends");
        std::thread::sleep(EACH);
    }
    assert_eq!(state(), "cancelled");
    let all = client
        .get(
            BUS,
            "books.get",
            std::slice::from_ref(&second.set),
            &["trashed".to_owned()],
            &Dict::new(),
        )
        .unwrap();
    assert!(
        (1..5).contains(&trashed(&all)),
        "each book is whole: {all:?}"
    );

    let receipts = client.all_receipts(&manifests);
    assert_eq!(receipts.len(), 1, "what was done before it stopped");
    client.undo(&manifests, &receipts[0].receipt).unwrap();
    let all = client
        .get(
            BUS,
            "books.get",
            std::slice::from_ref(&second.set),
            &["trashed".to_owned()],
            &Dict::new(),
        )
        .unwrap();
    assert_eq!(trashed(&all), 0);
    assert_eq!(client.all_receipts(&manifests), []);

    let prepared = client
        .prepare(
            BUS,
            "books.trash",
            &books(&["books.book:0", "books.book:9"]),
        )
        .unwrap();
    let performed = client.perform(prepared).unwrap().json();
    assert_eq!(performed["failed"][0]["reference"], "books.book:9");
    assert_eq!(performed["failed"][0]["error"], "not_found");
    assert_eq!(performed["things"].as_array().unwrap().len(), 1);

    let reader = Client::new(&connection).limited(granted(&["read"], &["books.book:1"]));
    let found = reader.find(BUS, "books.find", &Dict::new(), "", 9).unwrap();
    assert_eq!(found.things.len(), 1, "only what it may reach");
    assert_eq!(text(&found.things[0], "review"), "", "nothing foreign");
    let refused = |properties: &[String], book: &str| {
        reader
            .get(
                BUS,
                "books.get",
                &[book.to_owned()],
                properties,
                &Dict::new(),
            )
            .unwrap_err()
    };
    let foreign = refused(&["review".to_owned()], "books.book:1");
    assert_eq!(foreign.reason, Reason::NotGranted);
    assert_eq!(refused(&[], "books.book:2").reason, Reason::NotFound);
    let foreign_title = Client::new(&connection).limited(granted(&["read"], &["books.book:4"]));
    let found = foreign_title
        .find(BUS, "books.find", &Dict::new(), "", 9)
        .unwrap();
    assert_eq!(text(&found.things[0], "title"), "", "foreign in this one");
    let turned = |turn: &str| {
        let arguments = Dict::from([("turn".to_owned(), owned(turn.to_owned()))]);
        client.get(
            BUS,
            "books.get_cover",
            &["books.book:1".to_owned()],
            &[],
            &arguments,
        )
    };
    let cover = turned("90").unwrap();
    assert_eq!(text(&cover[0], "type"), "books.cover");
    assert_eq!(text(&cover[0], "reference"), "books.book:1");
    assert_eq!(text(&cover[0], "turn"), "90");
    assert_eq!(turned("45").unwrap_err().reason, Reason::InvalidArgument);
    let open = |client: &Client, book: &str| {
        client.open_files(&manifests, &[book.to_owned()], Access::Read)
    };
    let (opened, effects) = open(&client, "books.book:1").unwrap();
    assert_eq!((opened[0].name.as_str(), opened[0].size), ("book", 7));
    assert_eq!(effects, [Effect::ReadForeign]);
    let refused = open(&reader, "books.book:2").unwrap_err();
    assert_eq!(refused.reason, Reason::NotFound);
    drop((client, reader, other, connection, stranger));
    served.finish();
}
