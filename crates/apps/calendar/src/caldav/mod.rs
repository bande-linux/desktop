mod client;
mod http;
mod schedule;
mod xml;

pub use client::{Client, Delta, Discovered, Precondition, RemoteCalendar, Resource, candidates};
pub use http::{Credentials, Error, join, same_resource};
pub use schedule::{Availability, Principal, Scheduling, UserType};
