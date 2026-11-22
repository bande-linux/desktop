use std::collections::HashMap;
use std::ops::Range;

use bande_l10n::{text, tr};

use crate::crypto::{Sealed, Security, Standard, Verdict};
use crate::html::{Block, BlockKind, Document};
use crate::message::{Address, Content};
use crate::picture::{Picture, decode};
use crate::settings::MailSettings;
use crate::store::{MessageId, Summary};
use crate::ui::format::{full_date, initials, list_date};
use crate::ui::symbols::{Symbol, Symbols};
use bande_datetime::DateFormats;
use bande_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;

const MARGIN: f32 = 28.0;
const AVATAR: f32 = 40.0;
const AVATAR_GAP: f32 = 12.0;
const SMALL_AVATAR: f32 = 28.0;
const ACTION_SIZE: f32 = 28.0;
const FOCUS_BAR: f32 = 3.0;
const QUOTE_TOGGLE_HEIGHT: f32 = 24.0;
const MAX_ATTRIBUTION: usize = 300;
const LINE_GAP: f32 = 3.0;
const SECTION_GAP: f32 = 16.0;
const BLOCK_GAP: f32 = 10.0;
const QUOTE_INDENT: f32 = 14.0;
const QUOTE_BAR: f32 = 2.0;
const ITEM_INDENT: f32 = 12.0;
const BANNER_HEIGHT: f32 = 34.0;
const CHIP_HEIGHT: f32 = 30.0;
const CHIP_GAP: f32 = 8.0;
const CHIP_PADDING: f32 = 10.0;
const CHIP_MAX: f32 = 260.0;
const SYMBOL_SIZE: f32 = 16.0;
const SYMBOL_PIXELS: u32 = 32;
const BOLD: f32 = 700.0;
const MAX_REMOTE: usize = 20;
const MAX_INLINE_PIXELS: u64 = 24_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    Blocked,
    Loading,
    Loaded,
}

pub struct Reading {
    pub content: Content,
    pub document: Document,
    pub downloaded: bool,
    pub pictures: HashMap<String, Image>,
    pub remote: Remote,
}

fn image_of(picture: Picture) -> Option<Image> {
    Image::from_rgba(picture.width, picture.height, picture.rgba)
}

impl Reading {
    pub fn new(content: Content, downloaded: bool, settings: &MailSettings) -> Self {
        let plain = content.plain.is_some() && (settings.plain_text || content.html.is_none());
        let mut document = content.document(settings.plain_text);
        if plain && settings.fixed_width {
            for block in &mut document.blocks {
                if block.kind == BlockKind::Paragraph {
                    block.kind = BlockKind::Preformatted;
                }
            }
        }
        let mut pictures = HashMap::new();
        let mut pixels = 0u64;
        for source in document
            .blocks
            .iter()
            .filter_map(|block| block.image.as_deref())
        {
            if pictures.len() >= MAX_REMOTE || pixels >= MAX_INLINE_PIXELS {
                break;
            }
            if pictures.contains_key(source) {
                continue;
            }
            let Some(picture) = content
                .inline_part(source)
                .and_then(|part| decode(&part.data))
            else {
                continue;
            };
            pixels += u64::from(picture.width) * u64::from(picture.height);
            if let Some(image) = image_of(picture) {
                pictures.insert(source.to_owned(), image);
            }
        }
        Self {
            content,
            document,
            downloaded,
            pictures,
            remote: Remote::Blocked,
        }
    }

    pub fn remote_sources(&self) -> Vec<String> {
        let mut sources = Vec::new();
        for source in self
            .document
            .blocks
            .iter()
            .filter_map(|block| block.image.as_deref())
        {
            let remote = source
                .get(..8)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
            if remote && !sources.iter().any(|known| known == source) && sources.len() < MAX_REMOTE
            {
                sources.push(source.to_owned());
            }
        }
        sources
    }

    pub fn add_picture(&mut self, source: String, picture: Picture) {
        if let Some(image) = image_of(picture) {
            self.pictures.insert(source, image);
        }
    }

    fn shown_inline(&self, index: usize) -> bool {
        self.content
            .inline
            .iter()
            .any(|(id, part)| *part == index && self.pictures.contains_key(&format!("cid:{id}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Respond {
    Reply,
    ReplyAll,
    Forward,
}

pub struct Entry {
    pub summary: Summary,
    pub reading: Option<Reading>,
    pub quotes: bool,
}

pub struct Thread {
    pub entries: Vec<Entry>,
    pub focus: MessageId,
    pub reveal: bool,
}

impl Thread {
    pub fn entry(&self, id: MessageId) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.summary.id == id)
    }

    pub fn entry_mut(&mut self, id: MessageId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.summary.id == id)
    }

    pub fn focused(&self) -> Option<(&Summary, &Reading)> {
        self.entry(self.focus)
            .and_then(|entry| Some((&entry.summary, entry.reading.as_ref()?)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Card(MessageId),
    Expand(MessageId),
    Quotes(MessageId),
    Respond(MessageId, Respond),
    Address(Address),
    Link(String, Option<String>, String),
    Attachment(MessageId, usize),
    LoadRemote(MessageId),
    LookUpKey(MessageId),
    TrustIssuer(MessageId),
    Unseal(MessageId),
}

pub type Note = (String, Option<(String, Target)>);

fn standard_name(standard: Standard) -> &'static str {
    match standard {
        Standard::OpenPgp => "OpenPGP",
        Standard::Smime => "S/MIME",
    }
}

pub fn security_notes(id: MessageId, security: &Security) -> Vec<Note> {
    let mut notes = Vec::new();
    if let Some(encrypted) = &security.encrypted {
        let standard = standard_name(encrypted.standard);
        let note = match &encrypted.state {
            Sealed::Pending => (tr!("Decrypting the message…"), None),
            Sealed::Decrypted => (
                tr!(
                    "This message was encrypted with {standard}.",
                    standard = standard
                ),
                None,
            ),
            Sealed::NoKey => (
                tr!(
                    "This message is encrypted with {standard}, and none of your keys can open it.",
                    standard = standard
                ),
                None,
            ),
            Sealed::Unsupported => (
                tr!("This message is encrypted in a way Mail can’t open."),
                None,
            ),
            Sealed::Failed(reason) => (
                tr!(
                    "This message could not be decrypted. {reason}",
                    reason = reason.clone()
                ),
                Some((tr!("Try Again"), Target::Unseal(id))),
            ),
        };
        notes.push(note);
    }
    if security.pending && security.encrypted.is_none() {
        notes.push((tr!("Checking the signature…"), None));
    }
    let Some(signed) = &security.signed else {
        return notes;
    };
    let signer = match signed.signer.trim() {
        "" => signed.key.clone(),
        signer => signer.to_owned(),
    };
    let key = signed.key.clone();
    let note = match (signed.verdict, signed.standard) {
        (Verdict::Verified, _) => (tr!("Signed by {signer}, verified.", signer = signer), None),
        (Verdict::Unverified, Standard::OpenPgp) => (
            tr!(
                "Signed by {signer}, but you haven’t verified that key {key} belongs to them.",
                signer = signer,
                key = key
            ),
            None,
        ),
        (Verdict::Unverified, Standard::Smime) => (
            tr!(
                "Signed by {signer}, but the certificate comes from an issuer you don’t trust.",
                signer = signer
            ),
            signed
                .issuer
                .as_ref()
                .map(|_| (tr!("Trust Issuer…"), Target::TrustIssuer(id))),
        ),
        (Verdict::WrongSender, _) => (
            tr!(
                "Signed by {signer}, who is not the sender of this message.",
                signer = signer
            ),
            None,
        ),
        (Verdict::UnknownSigner, Standard::OpenPgp) => (
            tr!("Signed with key {key}, which you don’t have.", key = key),
            Some((tr!("Look Up Key"), Target::LookUpKey(id))),
        ),
        (Verdict::UnknownSigner, Standard::Smime) => (
            tr!("Signed, but the signer’s certificate is not in the message."),
            None,
        ),
        (Verdict::InvalidKey, _) => (
            tr!(
                "Signed by {signer}, but the key or certificate has expired, was revoked or may not sign mail.",
                signer = signer
            ),
            None,
        ),
        (Verdict::Unsupported, _) => (
            tr!(
                "Signed with an algorithm Mail doesn’t accept as secure, so the signature was not checked."
            ),
            None,
        ),
        (Verdict::Bad, _) => (
            tr!(
                "The signature doesn’t match: the message was changed after it was signed, or the signature is forged."
            ),
            None,
        ),
    };
    notes.push(note);
    notes
}

#[derive(Default)]
pub struct Layout {
    pub items: Vec<Item>,
    pub height: f32,
    pub targets: Vec<(Rect, Target)>,
    pub tops: Vec<(MessageId, f32)>,
    pub texts: Vec<(MessageId, Point, Paragraph)>,
}

fn authority(text: &str) -> Option<String> {
    let text = text.trim();
    let rest = text
        .split_once("://")
        .map_or(text, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?;
    let host = match rest.strip_prefix('[') {
        Some(literal) => format!("[{}]", literal.split(']').next()?),
        None => rest.split(':').next()?.to_owned(),
    }
    .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    (!host.is_empty() && !host.contains(char::is_whitespace)).then_some(host)
}

fn named(host: &str) -> bool {
    let top_level = host.rsplit('.').next().unwrap_or_default();
    host.contains('.')
        && top_level.len() >= 2
        && top_level
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

fn web(url: &str) -> bool {
    url.split(':')
        .next()
        .is_some_and(|scheme| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https"))
}

pub(crate) fn link_host(url: &str) -> Option<String> {
    web(url).then(|| authority(url)).flatten()
}

fn claimed_host(shown: &str, url: &str) -> Option<String> {
    let claimed = authority(shown).filter(|host| named(host))?;
    if !web(url) {
        return None;
    }
    match link_host(url) {
        Some(real) if named(&real) => {
            (real != claimed && !real.ends_with(&format!(".{claimed}"))).then_some(claimed)
        }
        _ => Some(claimed),
    }
}

fn names(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|address| address.shown().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

struct Writer<'a> {
    entry: MessageId,
    text: &'a mut TextSystem,
    palette: &'a Palette,
    symbols: &'a mut Symbols,
    layout: Layout,
    y: f32,
    width: f32,
}

impl Writer<'_> {
    fn paragraph(&mut self, text: &str, style: TextStyle, width: f32) -> Paragraph {
        self.text
            .paragraph(text, style, Some(width.max(1.0)), TextAlign::Start)
    }

    fn put(&mut self, paragraph: Paragraph, x: f32) -> f32 {
        let height = paragraph.height();
        self.layout
            .texts
            .push((self.entry, Point::new(x, self.y), paragraph.clone()));
        self.layout.items.push(Item::Text {
            origin: Point::new(x, self.y),
            paragraph,
        });
        height
    }

    fn rule(&mut self) {
        self.layout.items.push(Item::Fill {
            shape: Shape::rect(Rect::new(MARGIN, self.y, self.width - 2.0 * MARGIN, 1.0)),
            paint: self.palette.hairline.into(),
        });
    }

    fn symbol(&mut self, symbol: Symbol, x: f32, y: f32) {
        if let Some(image) = self.symbols.get(symbol, SYMBOL_PIXELS, self.palette.ink2) {
            self.layout.items.push(Item::Image {
                rect: Rect::new(x, y, SYMBOL_SIZE, SYMBOL_SIZE),
                image,
            });
        }
    }

    fn target(&mut self, rect: Rect, target: Target) {
        self.layout.targets.push((rect, target));
    }
}

fn avatar(writer: &mut Writer<'_>, shown: &str, frame: Rect) {
    let palette = *writer.palette;
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(frame, frame.width / 2.0),
        paint: palette.well.into(),
    });
    let role = match frame.width >= AVATAR {
        true => TextRole::BodyStrong,
        false => TextRole::Callout,
    };
    let letters = writer.text.paragraph(
        &initials(shown),
        TextStyle::role(role, palette.ink2).weight(600.0),
        Some(frame.width),
        TextAlign::Center,
    );
    writer.layout.items.push(Item::Text {
        origin: Point::new(
            frame.x,
            frame.y + ((frame.height - letters.height()) / 2.0).round(),
        ),
        paragraph: letters,
    });
}

fn sender_name(sender: &Address) -> String {
    match sender.shown() {
        "" => tr!("No Sender"),
        shown => shown.to_owned(),
    }
}

fn respond_buttons(writer: &mut Writer<'_>, id: MessageId, y: f32) -> f32 {
    let mut x = writer.width - MARGIN - 3.0 * ACTION_SIZE;
    let left = x;
    for (symbol, respond) in [
        (Symbol::Reply, Respond::Reply),
        (Symbol::ReplyAll, Respond::ReplyAll),
        (Symbol::Forward, Respond::Forward),
    ] {
        let inset = ((ACTION_SIZE - SYMBOL_SIZE) / 2.0).round();
        writer.symbol(symbol, x + inset, y + inset);
        writer.target(
            Rect::new(x, y, ACTION_SIZE, ACTION_SIZE),
            Target::Respond(id, respond),
        );
        x += ACTION_SIZE;
    }
    left
}

fn header(
    writer: &mut Writer<'_>,
    entry: &Entry,
    reading: &Reading,
    many: bool,
    clock: &Clock<'_>,
) {
    let palette = *writer.palette;
    let id = entry.summary.id;
    let top = writer.y;
    let envelope = &reading.content.envelope;
    let shown = sender_name(&envelope.sender());
    avatar(writer, &shown, Rect::new(MARGIN, top, AVATAR, AVATAR));
    let text_x = MARGIN + AVATAR + AVATAR_GAP;
    let date = envelope
        .date
        .map(|date| full_date(date, clock.zone, clock.formats))
        .unwrap_or_default();
    let date = writer.text.paragraph(
        &date,
        TextStyle::role(TextRole::Callout, palette.ink3),
        None,
        TextAlign::Start,
    );
    let date_x = writer.width - MARGIN - date.width();
    let date_height = writer.put(date, date_x);
    let details_right = match many {
        true => respond_buttons(writer, id, top + date_height + LINE_GAP) - AVATAR_GAP,
        false => writer.width - MARGIN,
    };
    let name_width = (date_x - text_x - AVATAR_GAP).max(40.0);
    let name = writer.paragraph(
        &shown,
        TextStyle::role(TextRole::BodyStrong, palette.ink),
        name_width,
    );
    let name_rect = Rect::new(text_x, writer.y, name.width(), name.height());
    if !envelope.sender().email.is_empty() {
        writer.target(name_rect, Target::Address(envelope.sender()));
    }
    writer.y += writer.put(name, text_x) + LINE_GAP;
    for (label, addresses) in [
        (text!("To: {names}"), &envelope.to),
        (text!("Cc: {names}"), &envelope.cc),
    ] {
        if addresses.is_empty() {
            continue;
        }
        let joined = names(addresses);
        let line = label.tr_args(&[("names", &joined as &dyn std::fmt::Display)]);
        let paragraph = writer.paragraph(
            &line,
            TextStyle::role(TextRole::Callout, palette.ink2),
            details_right - text_x,
        );
        if let Some(mut at) = line.find(&joined) {
            for address in addresses {
                let end = at + address.shown().len();
                for rect in paragraph.span_rects(at..end) {
                    writer.target(
                        Rect::new(rect.x + text_x, rect.y + writer.y, rect.width, rect.height),
                        Target::Address(address.clone()),
                    );
                }
                at = end + ", ".len();
            }
        }
        writer.y += writer.put(paragraph, text_x) + LINE_GAP;
    }
    writer.y = writer.y.max(top + AVATAR) + SECTION_GAP / 2.0;
    let subject = match envelope.subject.as_str() {
        "" => tr!("No Subject"),
        subject => subject.to_owned(),
    };
    let subject = writer.paragraph(
        &subject,
        TextStyle::role(TextRole::Heading, palette.ink),
        writer.width - 2.0 * MARGIN,
    );
    writer.y += writer.put(subject, MARGIN);
    if many {
        writer.layout.targets.insert(
            0,
            (
                Rect::new(0.0, top, writer.width, writer.y - top),
                Target::Expand(id),
            ),
        );
    }
    writer.y += SECTION_GAP;
    writer.rule();
    writer.y += SECTION_GAP;
}

fn collapsed(writer: &mut Writer<'_>, summary: &Summary, clock: &Clock<'_>) {
    let palette = *writer.palette;
    let top = writer.y;
    let shown = sender_name(&summary.sender);
    avatar(
        writer,
        &shown,
        Rect::new(MARGIN, top, SMALL_AVATAR, SMALL_AVATAR),
    );
    let text_x = MARGIN + SMALL_AVATAR + AVATAR_GAP;
    let date = writer.text.paragraph(
        &list_date(summary.date, clock.now, clock.zone, clock.formats),
        TextStyle::role(TextRole::Callout, palette.ink3),
        None,
        TextAlign::Start,
    );
    let date_x = writer.width - MARGIN - date.width();
    writer.put(date, date_x);
    let name = writer.text.fitted(
        &shown,
        TextStyle::role(TextRole::BodyStrong, palette.ink),
        (date_x - text_x - AVATAR_GAP).max(40.0),
    );
    writer.y += writer.put(name, text_x) + 1.0;
    let preview = writer.text.fitted(
        &summary
            .preview
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        TextStyle::role(TextRole::Callout, palette.ink3),
        (writer.width - MARGIN - text_x).max(40.0),
    );
    writer.y += writer.put(preview, text_x);
    writer.y = writer.y.max(top + SMALL_AVATAR);
    writer.layout.targets.insert(
        0,
        (
            Rect::new(0.0, top, writer.width, writer.y - top),
            Target::Expand(summary.id),
        ),
    );
}

fn banner(writer: &mut Writer<'_>, symbol: Symbol, message: &str, action: Option<(&str, Target)>) {
    let palette = *writer.palette;
    let frame = Rect::new(MARGIN, writer.y, writer.width - 2.0 * MARGIN, BANNER_HEIGHT);
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(frame, bande_toolkit::Radius::S),
        paint: palette.well.into(),
    });
    writer.symbol(
        symbol,
        frame.x + CHIP_PADDING,
        frame.y + ((BANNER_HEIGHT - SYMBOL_SIZE) / 2.0).round(),
    );
    let mut right = frame.x + frame.width - CHIP_PADDING;
    if let Some((action, target)) = action {
        let button = writer.text.paragraph(
            action,
            TextStyle::role(TextRole::Callout, palette.accent_text).weight(600.0),
            None,
            TextAlign::Start,
        );
        let x = right - button.width();
        let rect = Rect::new(
            x - CHIP_PADDING,
            frame.y,
            button.width() + 2.0 * CHIP_PADDING,
            BANNER_HEIGHT,
        );
        writer.layout.items.push(Item::Text {
            origin: Point::new(
                x,
                frame.y + ((BANNER_HEIGHT - button.height()) / 2.0).round(),
            ),
            paragraph: button,
        });
        writer.target(rect, target);
        right = x - 2.0 * CHIP_PADDING;
    }
    let text_x = frame.x + 2.0 * CHIP_PADDING + SYMBOL_SIZE;
    let paragraph = writer.paragraph(
        message,
        TextStyle::role(TextRole::Callout, palette.ink2),
        right - text_x,
    );
    let y = writer.y;
    writer.y = frame.y + ((BANNER_HEIGHT - paragraph.height()) / 2.0).round();
    writer.put(paragraph, text_x);
    writer.y = y + BANNER_HEIGHT + SECTION_GAP;
}

fn picture(writer: &mut Writer<'_>, image: &Image, x: f32, width: f32) {
    let natural = (image.width() as f32, image.height() as f32);
    let shown_width = natural.0.min(width).max(1.0);
    let shown_height = (natural.1 * shown_width / natural.0.max(1.0)).round();
    writer.layout.items.push(Item::Image {
        rect: Rect::new(x, writer.y, shown_width.round(), shown_height),
        image: image.clone(),
    });
    writer.y += shown_height + BLOCK_GAP;
}

fn quoted_runs(blocks: &[Block]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.quote == 0 {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.end == index => run.end = index + 1,
            _ => {
                let attribution = index.checked_sub(1).filter(|before| {
                    let block = &blocks[*before];
                    let text = block.text.trim_end();
                    block.quote == 0
                        && block.kind == BlockKind::Paragraph
                        && text.ends_with(':')
                        && text.len() <= MAX_ATTRIBUTION
                });
                runs.push(attribution.unwrap_or(index)..index + 1);
            }
        }
    }
    runs
}

fn quote_toggle(writer: &mut Writer<'_>, id: MessageId, label: &str) {
    let palette = *writer.palette;
    let paragraph = writer.text.paragraph(
        label,
        TextStyle::role(TextRole::Callout, palette.accent_text).weight(600.0),
        None,
        TextAlign::Start,
    );
    let frame = Rect::new(
        MARGIN,
        writer.y,
        paragraph.width() + 2.0 * CHIP_PADDING,
        QUOTE_TOGGLE_HEIGHT,
    );
    writer.layout.items.push(Item::Fill {
        shape: Shape::rounded(frame, QUOTE_TOGGLE_HEIGHT / 2.0),
        paint: palette.well.into(),
    });
    writer.layout.items.push(Item::Text {
        origin: Point::new(
            frame.x + CHIP_PADDING,
            frame.y + ((QUOTE_TOGGLE_HEIGHT - paragraph.height()) / 2.0).round(),
        ),
        paragraph,
    });
    writer.target(frame, Target::Quotes(id));
    writer.y += QUOTE_TOGGLE_HEIGHT + BLOCK_GAP;
}

fn block(writer: &mut Writer<'_>, block: &Block, pictures: &HashMap<String, Image>) {
    let palette = *writer.palette;
    let quote = f32::from(block.quote) * QUOTE_INDENT;
    let indent = quote
        + match block.kind {
            BlockKind::Item => ITEM_INDENT,
            _ => 0.0,
        };
    let x = MARGIN + indent;
    let width = writer.width - 2.0 * MARGIN - indent;
    if block.kind == BlockKind::Rule {
        writer.rule();
        writer.y += BLOCK_GAP;
        return;
    }
    if block.kind == BlockKind::Image {
        if let Some(image) = block.image.as_ref().and_then(|source| pictures.get(source)) {
            picture(writer, image, x, width);
        }
        return;
    }
    let color = match block.quote {
        0 => palette.ink,
        _ => palette.ink2,
    };
    let style = match block.kind {
        BlockKind::Heading => TextStyle::role(TextRole::BodyStrong, color),
        BlockKind::Preformatted => TextStyle::role(TextRole::Mono, color),
        _ => TextStyle::role(TextRole::Body, color),
    };
    let emphasis = block
        .bold
        .iter()
        .map(|range| (range.clone(), BOLD))
        .collect::<Vec<_>>();
    let paragraph = writer.text.emphasized(
        &block.text,
        style,
        &emphasis,
        Some(width.max(1.0)),
        TextAlign::Start,
    );
    for (range, url) in &block.links {
        let shown = block.text.get(range.clone()).unwrap_or_default().trim();
        let claimed = claimed_host(shown, url);
        let named = match shown {
            "" => url.clone(),
            shown => shown.to_owned(),
        };
        for rect in paragraph.span_rects(range.clone()) {
            let rect = Rect::new(rect.x + x, rect.y + writer.y, rect.width, rect.height);
            writer.layout.items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    rect.x,
                    rect.y + rect.height - 2.0,
                    rect.width,
                    1.0,
                )),
                paint: palette.accent_text.into(),
            });
            writer.target(
                rect,
                Target::Link(url.clone(), claimed.clone(), named.clone()),
            );
        }
    }
    let top = writer.y;
    let height = writer.put(paragraph, x);
    for level in 0..block.quote {
        writer.layout.items.push(Item::Fill {
            shape: Shape::rounded(
                Rect::new(
                    MARGIN + f32::from(level) * QUOTE_INDENT,
                    top,
                    QUOTE_BAR,
                    height,
                ),
                QUOTE_BAR / 2.0,
            ),
            paint: palette.accent.with_opacity(0.45).into(),
        });
    }
    writer.y += height + BLOCK_GAP;
}

fn body(writer: &mut Writer<'_>, id: MessageId, reading: &Reading, quotes: bool) {
    let blocks = &reading.document.blocks;
    let runs = quoted_runs(blocks);
    let mut index = 0;
    while index < blocks.len() {
        if let Some(run) = runs.iter().find(|run| run.start == index) {
            match quotes {
                true => quote_toggle(writer, id, &tr!("Hide Quoted Text")),
                false => {
                    quote_toggle(writer, id, &tr!("Show Quoted Text"));
                    index = run.end;
                    continue;
                }
            }
        }
        block(writer, &blocks[index], &reading.pictures);
        index += 1;
    }
}

fn attachments(writer: &mut Writer<'_>, id: MessageId, reading: &Reading) {
    let content = &reading.content;
    if (0..content.attachments.len()).all(|index| reading.shown_inline(index)) {
        return;
    }
    let palette = *writer.palette;
    writer.y += SECTION_GAP / 2.0;
    writer.rule();
    writer.y += SECTION_GAP;
    let mut x = MARGIN;
    for (index, attachment) in content.attachments.iter().enumerate() {
        if reading.shown_inline(index) {
            continue;
        }
        let label = format!(
            "{}  {}",
            attachment.name,
            bande_fs::format_size(attachment.data.len() as u64)
        );
        let paragraph = writer.text.paragraph(
            &label,
            TextStyle::role(TextRole::Callout, palette.ink),
            Some(CHIP_MAX - 3.0 * CHIP_PADDING - SYMBOL_SIZE),
            TextAlign::Start,
        );
        let width = (paragraph.width() + 3.0 * CHIP_PADDING + SYMBOL_SIZE).min(CHIP_MAX);
        if x + width > writer.width - MARGIN && x > MARGIN {
            x = MARGIN;
            writer.y += CHIP_HEIGHT + CHIP_GAP;
        }
        let chip = Rect::new(x, writer.y, width, CHIP_HEIGHT);
        writer.layout.items.push(Item::Fill {
            shape: Shape::rounded(chip, bande_toolkit::Radius::S),
            paint: palette.well.into(),
        });
        writer.symbol(
            Symbol::Attachment,
            chip.x + CHIP_PADDING,
            chip.y + ((CHIP_HEIGHT - SYMBOL_SIZE) / 2.0).round(),
        );
        let text_y = chip.y + ((CHIP_HEIGHT - paragraph.height()) / 2.0).round();
        writer.layout.items.push(Item::Clip {
            shape: Shape::rect(chip),
            items: vec![Item::Text {
                origin: Point::new(chip.x + 2.0 * CHIP_PADDING + SYMBOL_SIZE, text_y),
                paragraph,
            }],
        });
        writer.target(chip, Target::Attachment(id, index));
        x += width + CHIP_GAP;
    }
    writer.y += CHIP_HEIGHT + BLOCK_GAP;
}

fn expanded(
    writer: &mut Writer<'_>,
    entry: &Entry,
    reading: &Reading,
    many: bool,
    clock: &Clock<'_>,
) {
    let id = entry.summary.id;
    header(writer, entry, reading, many, clock);
    for (note, action) in security_notes(id, &reading.content.security) {
        banner(
            writer,
            Symbol::Security,
            &note,
            action
                .as_ref()
                .map(|(label, target)| (label.as_str(), target.clone())),
        );
    }
    if !reading.downloaded {
        banner(
            writer,
            Symbol::Remote,
            &tr!("Downloading the message…"),
            None,
        );
    } else if reading.document.remote_images > 0 {
        match reading.remote {
            Remote::Blocked => banner(
                writer,
                Symbol::Remote,
                &tr!("Remote content in this message was not loaded, to protect your privacy."),
                Some((&tr!("Load Remote Content"), Target::LoadRemote(id))),
            ),
            Remote::Loading => banner(
                writer,
                Symbol::Remote,
                &tr!("Loading remote content…"),
                None,
            ),
            Remote::Loaded => {}
        }
    }
    body(writer, id, reading, entry.quotes);
    attachments(writer, id, reading);
}

pub struct Clock<'a> {
    pub zone: &'a TimeZone,
    pub formats: &'a DateFormats,
    pub now: Timestamp,
}

pub fn lay_out(
    thread: &Thread,
    width: f32,
    text: &mut TextSystem,
    palette: &Palette,
    symbols: &mut Symbols,
    clock: Clock<'_>,
) -> Layout {
    let mut writer = Writer {
        entry: thread.focus,
        text,
        palette,
        symbols,
        layout: Layout::default(),
        y: 0.0,
        width,
    };
    let many = thread.entries.len() > 1;
    for (index, entry) in thread.entries.iter().enumerate() {
        let id = entry.summary.id;
        writer.entry = id;
        if index > 0 {
            writer.rule();
        }
        let top = writer.y;
        writer.layout.tops.push((id, top));
        writer.y += match index {
            0 => MARGIN,
            _ => SECTION_GAP,
        };
        let targets = std::mem::take(&mut writer.layout.targets);
        match &entry.reading {
            Some(reading) => expanded(&mut writer, entry, reading, many, &clock),
            None => {
                collapsed(&mut writer, &entry.summary, &clock);
                writer.y += SECTION_GAP;
            }
        }
        let card = Rect::new(0.0, top, width, writer.y - top);
        let inner = std::mem::replace(&mut writer.layout.targets, targets);
        writer.layout.targets.push((card, Target::Card(id)));
        writer.layout.targets.extend(inner);
    }
    writer.layout.height = writer.y + MARGIN;
    writer.layout
}

pub fn focus_bar(thread: &Thread, layout: &Layout, color: Color) -> Option<Item> {
    if thread.entries.len() < 2 {
        return None;
    }
    let card = layout
        .targets
        .iter()
        .find_map(|(rect, target)| (*target == Target::Card(thread.focus)).then_some(*rect))?;
    Some(Item::Fill {
        shape: Shape::rounded(
            Rect::new(
                ((MARGIN - FOCUS_BAR) / 2.0).round(),
                card.y + SECTION_GAP / 2.0,
                FOCUS_BAR,
                card.height - SECTION_GAP,
            ),
            FOCUS_BAR / 2.0,
        ),
        paint: color.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::claimed_host;

    #[test]
    fn a_link_that_shows_one_host_and_goes_to_another_asks_first() {
        let bank = Some("bank.example".to_owned());
        assert_eq!(claimed_host("bank.example", "http://203.0.113.5/"), bank);
        assert_eq!(
            claimed_host("www.bank.example", "https://[2001:db8::1]:8443/"),
            bank
        );
        assert_eq!(claimed_host("bank.example", "https://evil.example/"), bank);
        assert_eq!(
            claimed_host("bank.example", "https://login.bank.example/"),
            None
        );
        assert_eq!(
            claimed_host("support@bank.example", "mailto:support@bank.example"),
            None
        );
    }
}
