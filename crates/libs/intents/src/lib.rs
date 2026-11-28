//! Bande Intents: typed operations that apps and services offer to other programs.
//!
//! `docs/architecture/intents.md` is the specification. A program that serves intents declares
//! its things and intents with the derives, implements [`Find`], [`Describe`] and [`Perform`] for
//! its [`Store`], lists them in a [`Domain`] and serves them with an [`Owner`]. A program that
//! calls them reads the installed [`Manifests`] and uses a [`Client`].

extern crate self as bande_intents;

mod callers;
pub mod client;
mod definition;
mod domain;
mod files;
mod guard;
mod job;
mod limits;
mod manifest;
mod owner;
pub mod phrases;
mod plan;
mod program;
mod receipts;
mod refusal;
mod value;

pub use bande_intents_macros::{Intent, Record, Thing, Words};
pub use client::{Client, Page, Performed, Prepared, Receipt};
pub use definition::{
    Definition, Effect, Intent, Label, Parameter, Phrase, Property, Returns, Thing,
    ThingDefinition, domain, reject_unknown,
};
pub use domain::{Domain, Things};
pub use files::{Access, Files, Opened, Written};
pub use limits::Limits;
pub use manifest::{MANIFESTS, Manifest, Manifests};
pub use owner::{Current, Owner, PATH, Serving};
pub use plan::{
    Computed, ComputedList, Describe, Done, Find, Found, Job, Output, Perform, Plan, Record, Row,
    RowKind, Store, Undo, Unit, Wanted,
};
pub use program::{WRITE_INTENTS, run};
pub use refusal::{Reason, Refusal};
pub use value::{
    Address, Between, DateOrDateTime, Dict, IN_SET, Kind, LISTED, Level, List, Location, Name,
    Picture, RICH_TEXT, Ref, Referent, Refs, RichText, Rows, SHOWN_TEXT, Text, Uri, Url, Value,
    Words, dict_to_json, give, reference_token, take, to_json,
};
