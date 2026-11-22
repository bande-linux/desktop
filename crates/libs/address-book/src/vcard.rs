use bande_content_lines::{
    Component, Property, escape, parse, split_components, split_list, unescape,
};
use base64::Engine;

const MAX_CARD: usize = 8 << 20;
const MAX_PROPERTIES: usize = 4096;
const MAX_CARDS: usize = 100_000;
const MAX_PHOTO: usize = 4 << 20;
const MAX_VALUES: usize = 256;
const PRODID: &str = "-//Bande//Contacts//EN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V3,
    V4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Label {
    Home,
    Work,
    Mobile,
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Labelled {
    pub label: Label,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Name {
    pub family: String,
    pub given: String,
    pub additional: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Address {
    pub label: Label,
    pub street: String,
    pub locality: String,
    pub region: String,
    pub code: String,
    pub country: String,
}

impl Address {
    pub fn is_empty(&self) -> bool {
        [
            &self.street,
            &self.locality,
            &self.region,
            &self.code,
            &self.country,
        ]
        .iter()
        .all(|part| part.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Birthday {
    pub year: Option<i16>,
    pub month: i8,
    pub day: i8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Individual,
    Group,
    Other,
}

/// What the Contacts app edits; everything else in a card is kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contact {
    pub name: Name,
    pub formatted: String,
    pub organisation: String,
    pub title: String,
    pub emails: Vec<Labelled>,
    pub phones: Vec<Labelled>,
    pub addresses: Vec<Address>,
    pub birthday: Option<Birthday>,
    pub note: String,
    pub photo: Option<Photo>,
    pub categories: Vec<String>,
}

/// One vCard (RFC 6350 4.0 or RFC 2426 3.0), with every property it came with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    component: Component,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub cards: Vec<Card>,
    pub skipped: usize,
}

fn decode_quoted_printable(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'=')
            .then(|| value.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    out
}

fn text_from(bytes: Vec<u8>, charset: Option<&str>) -> String {
    let latin = charset.is_some_and(|charset| {
        ["iso-8859-1", "latin1", "windows-1252"]
            .iter()
            .any(|name| charset.eq_ignore_ascii_case(name))
    });
    match latin {
        true => bytes.iter().map(|byte| char::from(*byte)).collect(),
        false => String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn joined_soft_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut continuing = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let quoted = continuing || {
            let head = line.split(':').next().unwrap_or_default();
            head.to_ascii_uppercase().contains("QUOTED-PRINTABLE")
        };
        match quoted && line.ends_with('=') {
            true => {
                out.push_str(&line[..line.len() - 1]);
                continuing = true;
            }
            false => {
                out.push_str(line);
                out.push_str("\r\n");
                continuing = false;
            }
        }
    }
    out
}

fn chunks(text: &str) -> (Vec<String>, usize) {
    let mut found = Vec::new();
    let mut skipped = 0;
    let mut current: Option<String> = None;
    let mut depth = 0usize;
    for line in text.split("\r\n") {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            depth += 1;
            if depth == 1 {
                current = Some(String::new());
            }
        }
        if let Some(card) = current.as_mut() {
            card.push_str(line);
            card.push_str("\r\n");
            if card.len() > MAX_CARD {
                current = None;
                skipped += 1;
            }
        }
        if upper == "END:VCARD" && depth > 0 {
            depth -= 1;
            if depth == 0
                && let Some(card) = current.take()
            {
                match found.len() < MAX_CARDS {
                    true => found.push(card),
                    false => skipped += 1,
                }
            }
        }
    }
    (found, skipped)
}

fn normalised(mut property: Property) -> Property {
    let quoted =
        property.params.iter().any(|(name, value)| {
            name == "ENCODING" && value.eq_ignore_ascii_case("QUOTED-PRINTABLE")
        }) || property
            .params
            .iter()
            .any(|(name, value)| name == "TYPE" && value.eq_ignore_ascii_case("QUOTED-PRINTABLE"));
    if quoted {
        let charset = property.param("CHARSET").map(str::to_owned);
        property.value = text_from(decode_quoted_printable(&property.value), charset.as_deref());
        property.params.retain(|(name, value)| {
            !(name == "ENCODING"
                || name == "CHARSET"
                || value.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
        });
    }
    property
}

/// The cards in a `.vcf` text, each read on its own: a card that is malformed or over the
/// limits is skipped and counted, and the others are kept.
pub fn parse_cards(text: &str) -> Parsed {
    let (found, mut skipped) = chunks(&joined_soft_breaks(text));
    let mut cards = Vec::new();
    for chunk in found {
        match parse(&chunk)
            .ok()
            .and_then(|roots| roots.into_iter().next())
        {
            Some(component)
                if component.name == "VCARD" && component.properties.len() <= MAX_PROPERTIES =>
            {
                let mut component = Component {
                    properties: component.properties.into_iter().map(normalised).collect(),
                    ..component
                };
                component.components.clear();
                cards.push(Card { component });
            }
            _ => skipped += 1,
        }
    }
    Parsed { cards, skipped }
}

fn types(property: &Property) -> impl Iterator<Item = String> + '_ {
    property
        .params
        .iter()
        .filter(|(name, _)| name == "TYPE")
        .flat_map(|(_, value)| value.split(','))
        .map(|kind| kind.trim().to_ascii_lowercase())
}

fn label_of(property: &Property) -> Label {
    let mut label = Label::Other;
    for kind in types(property) {
        label = match kind.as_str() {
            "cell" | "mobile" => return Label::Mobile,
            "home" => Label::Home,
            "work" => Label::Work,
            _ => label,
        };
    }
    label
}

fn preferred(property: &Property) -> bool {
    types(property).any(|kind| kind == "pref") || property.param("PREF") == Some("1")
}

fn label_type(label: Label, phone: bool) -> Option<&'static str> {
    match label {
        Label::Home => Some("home"),
        Label::Work => Some("work"),
        Label::Mobile if phone => Some("cell"),
        Label::Mobile | Label::Other => None,
    }
}

fn component_text(parts: &[&str], index: usize) -> String {
    parts
        .get(index)
        .map(|part| {
            split_list(part)
                .into_iter()
                .map(unescape)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn structured(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| escape(part))
        .collect::<Vec<_>>()
        .join(";")
}

fn parse_birthday(value: &str) -> Option<Birthday> {
    let digits: String = value
        .split('T')
        .next()?
        .chars()
        .filter(|character| *character != '-')
        .collect();
    let number = |text: &str| text.parse::<i16>().ok();
    let (year, rest) = match (value.starts_with("--"), digits.len()) {
        (true, 4) => (None, digits.as_str()),
        (false, 8) => (Some(number(&digits[..4])?), &digits[4..]),
        _ => return None,
    };
    let month = i8::try_from(number(rest.get(..2)?)?).ok()?;
    let day = i8::try_from(number(rest.get(2..4)?)?).ok()?;
    let valid = (1..=12).contains(&month) && (1..=31).contains(&day);
    valid.then_some(Birthday { year, month, day })
}

fn birthday_text(birthday: Birthday) -> String {
    match birthday.year {
        Some(year) => format!("{year:04}-{:02}-{:02}", birthday.month, birthday.day),
        None => format!("--{:02}{:02}", birthday.month, birthday.day),
    }
}

fn decode_photo(property: &Property) -> Option<Photo> {
    let value = property.value.trim();
    let (mime, data) = match value.strip_prefix("data:") {
        Some(uri) => {
            let (head, data) = uri.split_once(',')?;
            let mime = head.strip_suffix(";base64")?;
            (mime.to_ascii_lowercase(), data)
        }
        None => {
            let base64 =
                |text: &str| text.eq_ignore_ascii_case("b") || text.eq_ignore_ascii_case("base64");
            if !property.param("ENCODING").is_some_and(base64)
                && !types(property).any(|kind| base64(&kind))
            {
                return None;
            }
            let kind = types(property)
                .find(|kind| ["jpeg", "jpg", "png", "gif"].contains(&kind.as_str()))
                .unwrap_or_else(|| "jpeg".to_owned());
            (format!("image/{}", kind.replace("jpg", "jpeg")), value)
        }
    };
    if data.len() > MAX_PHOTO / 3 * 4 + 4 {
        return None;
    }
    let cleaned: String = data
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    Some(Photo { mime, bytes })
}

impl Card {
    /// A new card, in vCard 3.0 that every CardDAV server takes (RFC 6352).
    pub fn new(uid: &str, contact: &Contact) -> Self {
        let mut component = Component::new("VCARD");
        component.push(Property::new("VERSION", "3.0"));
        component.push(Property::new("PRODID", PRODID));
        component.push(Property::new("UID", escape(uid)));
        let mut card = Self { component };
        card.write_name(contact);
        card.apply(&Contact::default(), contact);
        card
    }

    pub fn version(&self) -> Version {
        match self.component.value("VERSION").map(str::trim) {
            Some("4.0") => Version::V4,
            _ => Version::V3,
        }
    }

    fn text(&self, name: &str) -> String {
        self.component
            .value(name)
            .map(|value| unescape(value).trim().to_owned())
            .unwrap_or_default()
    }

    pub fn set_uid(&mut self, uid: &str) {
        self.replace(&["UID"], vec![Property::new("UID", escape(uid))]);
    }

    pub fn uid(&self) -> String {
        let uid = self.text("UID");
        uid.strip_prefix("urn:uuid:").unwrap_or(&uid).to_owned()
    }

    pub fn kind(&self) -> Kind {
        let kind = match self.text("KIND") {
            kind if kind.is_empty() => self.text("X-ADDRESSBOOKSERVER-KIND"),
            kind => kind,
        };
        match kind.to_ascii_lowercase().as_str() {
            "" | "individual" => Kind::Individual,
            "group" => Kind::Group,
            _ => Kind::Other,
        }
    }

    /// The UIDs of a group card's members (`MEMBER`, or the de-facto
    /// `X-ADDRESSBOOKSERVER-MEMBER` of vCard 3.0 servers).
    pub fn members(&self) -> Vec<String> {
        self.component
            .properties
            .iter()
            .filter(|property| {
                property.name == "MEMBER" || property.name == "X-ADDRESSBOOKSERVER-MEMBER"
            })
            .map(|property| {
                let value = unescape(&property.value);
                value
                    .strip_prefix("urn:uuid:")
                    .unwrap_or(&value)
                    .trim()
                    .to_owned()
            })
            .take(MAX_PROPERTIES)
            .collect()
    }

    /// Sets the members of a group card to `uids`, in the property the card already uses.
    pub fn set_members(&mut self, uids: &[String]) {
        let name = match self.version() {
            Version::V4 => "MEMBER",
            Version::V3 => "X-ADDRESSBOOKSERVER-MEMBER",
        };
        self.component.properties.retain(|property| {
            property.name != "MEMBER" && property.name != "X-ADDRESSBOOKSERVER-MEMBER"
        });
        for uid in uids {
            self.component
                .push(Property::new(name, format!("urn:uuid:{}", escape(uid))));
        }
    }

    /// A new group card with `name`, in the form a vCard 3.0 server keeps.
    pub fn new_group(uid: &str, name: &str) -> Self {
        let mut component = Component::new("VCARD");
        component.push(Property::new("VERSION", "3.0"));
        component.push(Property::new("PRODID", PRODID));
        component.push(Property::new("UID", escape(uid)));
        component.push(Property::new("X-ADDRESSBOOKSERVER-KIND", "group"));
        component.push(Property::new("FN", escape(name)));
        component.push(Property::new("N", structured(&[name, "", "", "", ""])));
        Self { component }
    }

    pub fn contact(&self) -> Contact {
        let parts = |name: &str| {
            self.component
                .value(name)
                .map(|value| {
                    split_components(value)
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default()
        };
        let n = parts("N");
        let n: Vec<&str> = n.iter().map(String::as_str).collect();
        let org = parts("ORG");
        let org: Vec<&str> = org.iter().map(String::as_str).collect();
        let labelled = |name: &str, strip: &str| {
            let mut found: Vec<(bool, Labelled)> = self
                .component
                .all(name)
                .take(MAX_VALUES)
                .map(|property| {
                    let value = unescape(&property.value);
                    let value = value
                        .strip_prefix(strip)
                        .unwrap_or(&value)
                        .trim()
                        .to_owned();
                    (
                        preferred(property),
                        Labelled {
                            label: label_of(property),
                            value,
                        },
                    )
                })
                .filter(|(_, labelled)| !labelled.value.is_empty())
                .collect();
            found.sort_by_key(|(preferred, _)| !preferred);
            found.into_iter().map(|(_, labelled)| labelled).collect()
        };
        Contact {
            name: Name {
                family: component_text(&n, 0),
                given: component_text(&n, 1),
                additional: component_text(&n, 2),
                prefix: component_text(&n, 3),
                suffix: component_text(&n, 4),
            },
            formatted: self.text("FN"),
            organisation: [component_text(&org, 0), component_text(&org, 1)]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            title: self.text("TITLE"),
            emails: labelled("EMAIL", "mailto:"),
            phones: labelled("TEL", "tel:"),
            addresses: self
                .component
                .all("ADR")
                .take(MAX_VALUES)
                .map(|property| {
                    let parts = split_components(&property.value);
                    Address {
                        label: label_of(property),
                        street: [component_text(&parts, 1), component_text(&parts, 2)]
                            .into_iter()
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        locality: component_text(&parts, 3),
                        region: component_text(&parts, 4),
                        code: component_text(&parts, 5),
                        country: component_text(&parts, 6),
                    }
                })
                .filter(|address| !address.is_empty())
                .collect(),
            birthday: self.component.property("BDAY").and_then(|property| {
                let omitted = property.param("X-APPLE-OMIT-YEAR");
                parse_birthday(property.value.trim()).map(|birthday| Birthday {
                    year: birthday
                        .year
                        .filter(|year| omitted.is_none_or(|omitted| omitted != year.to_string())),
                    ..birthday
                })
            }),
            note: self.text("NOTE"),
            photo: self.component.property("PHOTO").and_then(decode_photo),
            categories: self
                .component
                .all("CATEGORIES")
                .flat_map(|property| split_list(&property.value))
                .map(|item| unescape(item).trim().to_owned())
                .filter(|item| !item.is_empty())
                .take(MAX_VALUES)
                .collect(),
        }
    }

    fn replace(&mut self, names: &[&str], new: Vec<Property>) {
        let properties = &mut self.component.properties;
        let ours = |property: &Property| names.contains(&property.name.as_str());
        let groups: Vec<String> = properties
            .iter()
            .filter(|property| ours(property))
            .filter_map(|property| property.group.clone())
            .collect();
        let doomed = |property: &Property| {
            ours(property)
                || (property.name == "X-ABLABEL"
                    && property
                        .group
                        .as_ref()
                        .is_some_and(|group| groups.contains(group)))
        };
        let at = properties.iter().position(ours).map(|at| {
            properties[..at]
                .iter()
                .filter(|property| !doomed(property))
                .count()
        });
        properties.retain(|property| !doomed(property));
        let at = at.unwrap_or(properties.len());
        properties.splice(at..at, new);
    }

    fn write_name(&mut self, contact: &Contact) {
        let name = &contact.name;
        let formatted = match contact.formatted.trim() {
            "" => display_name(contact),
            formatted => formatted.to_owned(),
        };
        self.replace(&["FN"], vec![Property::new("FN", escape(&formatted))]);
        let parts = [
            &name.family,
            &name.given,
            &name.additional,
            &name.prefix,
            &name.suffix,
        ]
        .map(|part| part.trim());
        self.replace(&["N"], vec![Property::new("N", structured(&parts))]);
    }

    fn labelled_properties(&self, name: &str, values: &[Labelled], phone: bool) -> Vec<Property> {
        values
            .iter()
            .filter(|labelled| !labelled.value.trim().is_empty())
            .map(|labelled| {
                let mut property = Property::new(name, escape(labelled.value.trim()));
                if let Some(kind) = label_type(labelled.label, phone) {
                    property = property.with_param("TYPE", kind);
                }
                match (phone, labelled.label) {
                    (true, Label::Mobile | Label::Home | Label::Work) => {}
                    (true, Label::Other) => property = property.with_param("TYPE", "voice"),
                    (false, _) if self.version() == Version::V3 => {
                        property = property.with_param("TYPE", "internet")
                    }
                    (false, _) => {}
                }
                property
            })
            .collect()
    }

    fn photo_property(&self, photo: &Photo) -> Property {
        let data = base64::engine::general_purpose::STANDARD.encode(&photo.bytes);
        match self.version() {
            Version::V4 => Property::new("PHOTO", format!("data:{};base64,{data}", photo.mime)),
            Version::V3 => {
                let kind = photo
                    .mime
                    .strip_prefix("image/")
                    .unwrap_or("jpeg")
                    .to_ascii_uppercase();
                Property::new("PHOTO", data)
                    .with_param("ENCODING", "b")
                    .with_param("TYPE", &kind)
            }
        }
    }

    /// Writes the fields of `contact` that differ from `before` (what [`Card::contact`] gave),
    /// so that the properties of a field left alone keep every parameter and group they had.
    pub fn apply(&mut self, before: &Contact, contact: &Contact) {
        if before.name != contact.name || before.formatted != contact.formatted {
            self.write_name(contact);
        }
        if before.organisation != contact.organisation {
            let new = match contact.organisation.trim() {
                "" => Vec::new(),
                organisation => vec![Property::new("ORG", structured(&[organisation]))],
            };
            self.replace(&["ORG"], new);
        }
        if before.title != contact.title {
            let new = match contact.title.trim() {
                "" => Vec::new(),
                title => vec![Property::new("TITLE", escape(title))],
            };
            self.replace(&["TITLE"], new);
        }
        if before.emails != contact.emails {
            let new = self.labelled_properties("EMAIL", &contact.emails, false);
            self.replace(&["EMAIL"], new);
        }
        if before.phones != contact.phones {
            let new = self.labelled_properties("TEL", &contact.phones, true);
            self.replace(&["TEL"], new);
        }
        if before.addresses != contact.addresses {
            let new = contact
                .addresses
                .iter()
                .filter(|address| !address.is_empty())
                .map(|address| {
                    let street = address.street.replace('\n', ", ");
                    let value = structured(&[
                        "",
                        "",
                        street.trim(),
                        address.locality.trim(),
                        address.region.trim(),
                        address.code.trim(),
                        address.country.trim(),
                    ]);
                    let property = Property::new("ADR", value);
                    match label_type(address.label, false) {
                        Some(kind) => property.with_param("TYPE", kind),
                        None => property,
                    }
                })
                .collect();
            self.replace(&["ADR", "LABEL"], new);
        }
        if before.birthday != contact.birthday {
            let new = contact
                .birthday
                .map(|birthday| Property::new("BDAY", birthday_text(birthday)))
                .into_iter()
                .collect();
            self.replace(&["BDAY"], new);
        }
        if before.note != contact.note {
            let new = match contact.note.trim() {
                "" => Vec::new(),
                note => vec![Property::new("NOTE", escape(note))],
            };
            self.replace(&["NOTE"], new);
        }
        if before.photo != contact.photo {
            let new = contact
                .photo
                .as_ref()
                .map(|photo| self.photo_property(photo))
                .into_iter()
                .collect();
            self.replace(&["PHOTO"], new);
        }
        if before.categories != contact.categories {
            let new = match contact.categories.is_empty() {
                true => Vec::new(),
                false => vec![Property::new(
                    "CATEGORIES",
                    contact
                        .categories
                        .iter()
                        .map(|category| escape(category))
                        .collect::<Vec<_>>()
                        .join(","),
                )],
            };
            self.replace(&["CATEGORIES"], new);
        }
        self.replace(&["REV"], vec![Property::new("REV", revision())]);
    }

    pub fn to_vcf(&self) -> String {
        self.component.to_text()
    }
}

fn revision() -> String {
    jiff::Timestamp::now()
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string()
}

/// The name to show for a contact: its formatted name, else its name's parts, its
/// organisation or its first address.
pub fn display_name(contact: &Contact) -> String {
    let formatted = contact.formatted.trim();
    if !formatted.is_empty() {
        return formatted.to_owned();
    }
    let name = &contact.name;
    let parts: Vec<&str> = [
        &name.prefix,
        &name.given,
        &name.additional,
        &name.family,
        &name.suffix,
    ]
    .iter()
    .map(|part| part.trim())
    .filter(|part| !part.is_empty())
    .collect();
    if !parts.is_empty() {
        return parts.join(" ");
    }
    [&contact.organisation]
        .into_iter()
        .chain(contact.emails.iter().map(|email| &email.value))
        .chain(contact.phones.iter().map(|phone| &phone.value))
        .map(|text| text.trim())
        .find(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The key a contact sorts and is indexed by: its family name first when it has one.
pub fn sort_key(contact: &Contact) -> String {
    let name = &contact.name;
    let key = match name.family.trim().is_empty() {
        true => display_name(contact),
        false => format!("{} {}", name.family.trim(), name.given.trim()),
    };
    key.to_lowercase()
}

/// The cards as one `.vcf` text.
pub fn write_cards<'a>(cards: impl IntoIterator<Item = &'a Card>) -> String {
    cards.into_iter().map(Card::to_vcf).collect()
}
