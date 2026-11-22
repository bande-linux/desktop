use bande_discovery::{Resolver, domain_of};

use crate::http::{
    Credentials, Error, Http, Response, join, origin, percent_decode, same_resource,
};
use crate::xml::{
    self, CALDAV, CALENDAR_COLOR_NS, CALSERVER, CARDDAV, DAV, Multistatus, parse_multistatus,
};

const MAX_REDIRECTS: usize = 5;
const MULTIGET_BATCH: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCalendar {
    pub url: String,
    pub name: String,
    pub color: Option<String>,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub url: String,
    pub etag: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    pub changed: Vec<(String, Option<String>)>,
    pub removed: Vec<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    Create,
    Match(String),
    None,
}

/// The protocol a [`Client`] speaks on top of WebDAV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    CalDav,
    CardDav,
}

impl Service {
    fn name(self) -> &'static str {
        match self {
            Self::CalDav => "caldav",
            Self::CardDav => "carddav",
        }
    }

    fn app(self) -> &'static str {
        match self {
            Self::CalDav => "calendar",
            Self::CardDav => "contacts",
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Self::CalDav => "text/calendar",
            Self::CardDav => "text/vcard",
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: Http,
    service: Service,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub principal: String,
    pub home: String,
    pub addresses: Vec<String>,
    pub calendars: Vec<RemoteCalendar>,
}

fn has_scheme(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

fn well_known(service: Service, url: &str) -> String {
    format!("{}/.well-known/{}", origin(url), service.name())
}

/// The URLs to try for a server the user typed: as typed, its `/.well-known/` URL and its root.
pub fn candidates(service: Service, server: &str) -> Vec<String> {
    let server = server.trim();
    let mut found = match (server.is_empty(), has_scheme(server)) {
        (true, _) => return Vec::new(),
        (false, true) => vec![server.to_owned()],
        (false, false) if server.contains('/') => vec![format!("https://{server}")],
        (false, false) => Vec::new(),
    };
    let base = match has_scheme(server) {
        true => server.to_owned(),
        false => format!("https://{}", server.split('/').next().unwrap_or(server)),
    };
    for url in [well_known(service, &base), format!("{}/", origin(&base))] {
        if !found.contains(&url) {
            found.push(url);
        }
    }
    found
}

fn contexts(service: Service, domain: &str, resolver: &Resolver) -> Vec<String> {
    let mut found = Vec::new();
    for (secure, scheme, default_port) in [("s", "https", 443), ("", "http", 80)] {
        let name = format!("_{}{secure}._tcp.{domain}", service.name());
        let path = resolver
            .texts(&name)
            .iter()
            .find_map(|text| text.strip_prefix("path="))
            .filter(|path| path.starts_with('/'))
            .map(str::to_owned);
        for target in resolver.services(&name) {
            let port = match target.port == default_port {
                true => String::new(),
                false => format!(":{}", target.port),
            };
            let path = path
                .clone()
                .unwrap_or_else(|| format!("/.well-known/{}", service.name()));
            found.push(format!("{scheme}://{}{port}{path}", target.host));
        }
    }
    found.push(format!("https://{domain}/.well-known/{}", service.name()));
    found
}

fn rank(error: &Error) -> u8 {
    match error {
        Error::Unauthorized => 0,
        Error::Unsupported(_) => 1,
        Error::Insecure(_) => 2,
        Error::Certificate(_) => 3,
        _ => 4,
    }
}

impl Client {
    pub fn new(service: Service, credentials: Option<&Credentials>) -> Self {
        Self {
            http: Http::new(service.app(), credentials),
            service,
        }
    }

    fn follow(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<(String, Response), Error> {
        let mut url = url.to_owned();
        for _ in 0..MAX_REDIRECTS {
            let response = self.http.send(method, &url, headers, body)?;
            match (&response.location, response.status) {
                (Some(location), 301 | 302 | 303 | 307 | 308) => url = join(&url, location),
                _ => return Ok((url, response)),
            }
        }
        Err(Error::Network(format!("too many redirects from {url}")))
    }

    pub(crate) fn multistatus(
        &self,
        method: &str,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<(String, Multistatus), Error> {
        let (url, response) = self.follow(method, url, &[("Depth", depth)], Some(body))?;
        if response.status != 207 {
            return Err(Error::Status {
                status: response.status,
                url,
            });
        }
        Ok((url.clone(), parse_multistatus(&response.body)?))
    }

    fn first_href(
        &self,
        url: &str,
        namespace: &str,
        name: &str,
        prop: &str,
    ) -> Result<Option<(String, String)>, Error> {
        let (url, status) = self.multistatus("PROPFIND", url, "0", &xml::propfind(&[prop]))?;
        Ok(status
            .entries
            .iter()
            .find_map(|entry| entry.prop(namespace, name)?.hrefs().into_iter().next())
            .map(|href| (url.clone(), join(&url, &href))))
    }

    /// The context URL that the domain of `address` points to for `service` (RFC 6764),
    /// found without sending credentials.
    pub fn find(service: Service, address: &str, resolver: &Resolver) -> Option<String> {
        let probe = Self::new(service, None);
        contexts(service, &domain_of(address)?, resolver)
            .into_iter()
            .find_map(|url| probe.probe(&url))
    }

    fn probe(&self, url: &str) -> Option<String> {
        let mut url = url.to_owned();
        let body = xml::propfind(&["d:current-user-principal"]);
        for _ in 0..MAX_REDIRECTS {
            let response = match self
                .http
                .send("PROPFIND", &url, &[("Depth", "0")], Some(&body))
            {
                Err(Error::Unauthorized) => return Some(url),
                Err(error) => {
                    tracing::debug!("no {} server at {url}: {error}", self.service.name());
                    return None;
                }
                Ok(response) => response,
            };
            match (&response.location, response.status) {
                (Some(location), 301 | 302 | 303 | 307 | 308) => url = join(&url, location),
                (_, 207) => return Some(url),
                (_, status) => {
                    tracing::debug!("no {} server at {url}: {status}", self.service.name());
                    return None;
                }
            }
        }
        None
    }

    pub(crate) fn discover_with<T>(
        &self,
        server: &str,
        at: impl Fn(&str) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut best = Error::NotFound(server.to_owned());
        for candidate in candidates(self.service, server) {
            match at(&candidate) {
                Ok(found) => return Ok(found),
                Err(error) => {
                    tracing::debug!("nothing at {candidate}: {error}");
                    if rank(&error) <= rank(&best) {
                        best = error;
                    }
                }
            }
        }
        Err(best)
    }

    pub(crate) fn principal_and_home(
        &self,
        url: &str,
        (namespace, name, prop): (&str, &str, &str),
    ) -> Result<(String, String), Error> {
        let principal = self.first_href(
            url,
            DAV,
            "current-user-principal",
            "d:current-user-principal",
        )?;
        let (url, principal) = match principal {
            Some(found) => found,
            None => (url.to_owned(), url.to_owned()),
        };
        let home = self
            .first_href(&principal, namespace, name, prop)?
            .map(|(_, home)| home);
        Ok((principal, home.unwrap_or(url)))
    }

    /// Signs in at `server` and follows `current-user-principal` → `calendar-home-set` → the
    /// calendars there.
    pub fn discover_calendars(&self, server: &str) -> Result<Discovered, Error> {
        self.discover_with(server, |url| self.calendars_at(url))
    }

    fn calendars_at(&self, url: &str) -> Result<Discovered, Error> {
        let (principal, home) =
            self.principal_and_home(url, (CALDAV, "calendar-home-set", "c:calendar-home-set"))?;
        let calendars = self.calendars(&home)?;
        if calendars.is_empty() {
            return Err(Error::NotFound(home));
        }
        let addresses = self
            .multistatus(
                "PROPFIND",
                &principal,
                "0",
                &xml::propfind(&["c:calendar-user-address-set"]),
            )
            .map(|(_, status)| {
                status
                    .entries
                    .iter()
                    .find_map(|entry| entry.prop(CALDAV, "calendar-user-address-set"))
                    .map(|set| set.hrefs())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
            .iter()
            .filter_map(|href| {
                href.get(..7)
                    .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
                    .and_then(|_| href.get(7..))
                    .map(str::to_owned)
            })
            .collect();
        Ok(Discovered {
            principal,
            home,
            addresses,
            calendars,
        })
    }

    pub fn calendars(&self, home: &str) -> Result<Vec<RemoteCalendar>, Error> {
        let body = xml::propfind(&[
            "d:resourcetype",
            "d:displayname",
            "ic:calendar-color",
            "cs:getctag",
            "d:sync-token",
            "d:current-user-privilege-set",
            "c:supported-calendar-component-set",
        ]);
        let (url, status) = self.multistatus("PROPFIND", home, "1", &body)?;
        Ok(status
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .prop(DAV, "resourcetype")
                    .is_some_and(|kind| kind.child(CALDAV, "calendar").is_some())
            })
            .filter(|entry| {
                entry
                    .prop(CALDAV, "supported-calendar-component-set")
                    .is_none_or(|set| {
                        set.children.is_empty()
                            || set.children.iter().any(|component| {
                                component
                                    .attribute("name")
                                    .is_some_and(|name| name.eq_ignore_ascii_case("VEVENT"))
                            })
                    })
            })
            .map(|entry| {
                let calendar_url = join(&url, &entry.href);
                let read_only = entry.read_only();
                RemoteCalendar {
                    name: entry
                        .text(DAV, "displayname")
                        .map(str::to_owned)
                        .unwrap_or_else(|| last_segment(&calendar_url)),
                    color: entry
                        .text(CALENDAR_COLOR_NS, "calendar-color")
                        .map(str::to_owned),
                    ctag: entry.text(CALSERVER, "getctag").map(str::to_owned),
                    sync_token: entry.text(DAV, "sync-token").map(str::to_owned),
                    read_only,
                    url: calendar_url,
                }
            })
            .collect())
    }

    pub fn collection_tags(
        &self,
        calendar: &str,
    ) -> Result<(Option<String>, Option<String>), Error> {
        let (_, status) = self.multistatus(
            "PROPFIND",
            calendar,
            "0",
            &xml::propfind(&["cs:getctag", "d:sync-token"]),
        )?;
        let entry = status.entries.first();
        Ok((
            entry
                .and_then(|entry| entry.text(CALSERVER, "getctag"))
                .map(str::to_owned),
            entry
                .and_then(|entry| entry.text(DAV, "sync-token"))
                .map(str::to_owned),
        ))
    }

    pub fn sync_collection(&self, calendar: &str, token: Option<&str>) -> Result<Delta, Error> {
        let (url, status) =
            self.multistatus("REPORT", calendar, "1", &xml::sync_collection(token))?;
        let mut delta = Delta {
            token: status.sync_token.clone(),
            ..Delta::default()
        };
        for entry in status.entries {
            let href = join(&url, &entry.href);
            if same_resource(&href, &url) {
                continue;
            }
            match entry.status {
                Some(404) => delta.removed.push(href),
                _ => delta
                    .changed
                    .push((href, entry.text(DAV, "getetag").map(str::to_owned))),
            }
        }
        Ok(delta)
    }

    pub fn etags(&self, collection: &str) -> Result<Vec<(String, Option<String>)>, Error> {
        let (method, body) = match self.service {
            Service::CalDav => ("REPORT", xml::calendar_query_events()),
            Service::CardDav => ("PROPFIND", xml::propfind(&["d:resourcetype", "d:getetag"])),
        };
        let (url, status) = self.multistatus(method, collection, "1", &body)?;
        Ok(status
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .prop(DAV, "resourcetype")
                    .is_none_or(|kind| kind.child(DAV, "collection").is_none())
            })
            .map(|entry| {
                (
                    join(&url, &entry.href),
                    entry.text(DAV, "getetag").map(str::to_owned),
                )
            })
            .filter(|(href, _)| !same_resource(href, &url))
            .collect())
    }

    pub fn multiget(&self, collection: &str, urls: &[String]) -> Result<Vec<Resource>, Error> {
        let (namespace, name) = match self.service {
            Service::CalDav => (CALDAV, "calendar-data"),
            Service::CardDav => (CARDDAV, "address-data"),
        };
        let mut found = Vec::new();
        for batch in urls.chunks(MULTIGET_BATCH) {
            let hrefs: Vec<String> = batch.iter().map(|url| path_of(url).to_owned()).collect();
            let body = xml::multiget(self.service, &hrefs);
            let (url, status) = self.multistatus("REPORT", collection, "1", &body)?;
            for entry in status.entries {
                if let Some(data) = entry.text(namespace, name) {
                    found.push(Resource {
                        url: join(&url, &entry.href),
                        etag: entry.text(DAV, "getetag").map(str::to_owned),
                        data: data.to_owned(),
                    });
                }
            }
        }
        Ok(found)
    }

    pub fn get(&self, url: &str) -> Result<Resource, Error> {
        let (url, response) =
            self.follow("GET", url, &[("Accept", self.service.media_type())], None)?;
        match response.success() {
            true => Ok(Resource {
                url,
                etag: response.etag,
                data: response.body,
            }),
            false => Err(Error::Status {
                status: response.status,
                url,
            }),
        }
    }

    pub fn put(
        &self,
        url: &str,
        data: &str,
        precondition: &Precondition,
    ) -> Result<Option<String>, Error> {
        let content_type = format!("{}; charset=utf-8", self.service.media_type());
        let mut headers = vec![("Content-Type", content_type.as_str())];
        match precondition {
            Precondition::Create => headers.push(("If-None-Match", "*")),
            Precondition::Match(etag) => headers.push(("If-Match", etag.as_str())),
            Precondition::None => {}
        }
        let response = self.http.send("PUT", url, &headers, Some(data))?;
        match response.status {
            412 => Err(Error::PreconditionFailed),
            status if (200..300).contains(&status) => Ok(response.etag),
            status => Err(Error::Status {
                status,
                url: url.to_owned(),
            }),
        }
    }

    pub fn delete(&self, url: &str, etag: Option<&str>) -> Result<(), Error> {
        let headers: Vec<(&str, &str)> = etag.map(|etag| ("If-Match", etag)).into_iter().collect();
        let response = self.http.send("DELETE", url, &headers, None)?;
        match response.status {
            412 => Err(Error::PreconditionFailed),
            404 => Ok(()),
            status if (200..300).contains(&status) => Ok(()),
            status => Err(Error::Status {
                status,
                url: url.to_owned(),
            }),
        }
    }

    fn set_property(
        &self,
        url: &str,
        namespace: &str,
        name: &str,
        element: &str,
        value: &str,
    ) -> Result<(), Error> {
        let (url, status) =
            self.multistatus("PROPPATCH", url, "0", &xml::set_property(element, value))?;
        match status
            .entries
            .iter()
            .any(|entry| entry.prop(namespace, name).is_some())
        {
            true => Ok(()),
            false => Err(Error::Status { status: 403, url }),
        }
    }

    pub fn set_color(&self, url: &str, color: &str) -> Result<(), Error> {
        self.set_property(
            url,
            CALENDAR_COLOR_NS,
            "calendar-color",
            "ic:calendar-color",
            color,
        )
    }

    pub fn rename(&self, url: &str, name: &str) -> Result<(), Error> {
        self.set_property(url, DAV, "displayname", "d:displayname", name)
    }

    pub fn make_calendar(&self, url: &str, name: &str, color: &str) -> Result<(), Error> {
        let response =
            self.http
                .send("MKCALENDAR", url, &[], Some(&xml::mkcalendar(name, color)))?;
        match response.success() {
            true => Ok(()),
            false => Err(Error::Status {
                status: response.status,
                url: url.to_owned(),
            }),
        }
    }
}

fn path_of(url: &str) -> &str {
    &url[origin(url).len()..]
}

pub(crate) fn last_segment(url: &str) -> String {
    percent_decode(url.trim_end_matches('/').rsplit('/').next().unwrap_or(url))
}
