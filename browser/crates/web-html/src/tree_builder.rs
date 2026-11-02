use encoding_rs::Encoding;
use web_atoms::{Atom, Namespace, QualName};

use crate::encoding::{charset_from_meta_content, label_encoding};
use crate::foreign::{
    adjust_foreign_attributes, adjust_mathml_attributes, adjust_svg_attributes, svg_tag_name,
};
use crate::sink::{QuirksMode, TreeSink};
use crate::tokenizer::{Doctype, State, Tag, Token};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

enum Tok<'a> {
    Chars(&'a str),
    Start(&'a mut Tag),
    End(&'a mut Tag),
    Comment(&'a str),
    Pi(&'a str, &'a str),
    Doctype(&'a Doctype),
    Eof,
}

#[derive(Clone, Copy)]
struct Open<N> {
    node: N,
    name: QualName,
    html_integration: bool,
}

enum Formatting<N> {
    Marker,
    Element { node: N, tag: Tag },
}

#[derive(Clone, Copy)]
enum Scope {
    Default,
    ListItem,
    Button,
    Table,
}

pub(crate) struct TreeBuilder<S: TreeSink> {
    pub(crate) sink: S,
    pub(crate) tokenizer_state: Option<State>,
    pub(crate) declared_encoding: Option<&'static Encoding>,
    mode: Mode,
    original_mode: Mode,
    template_modes: Vec<Mode>,
    open: Vec<Open<S::Node>>,
    formatting: Vec<Formatting<S::Node>>,
    head: Option<S::Node>,
    form: Option<S::Node>,
    context: Option<(QualName, bool)>,
    scripting: bool,
    frameset_ok: bool,
    foster: bool,
    quirks: QuirksMode,
    skip_newline: bool,
    table_text: String,
    table_text_has_content: bool,
}

const fn html(local: Atom) -> QualName {
    QualName::html(local)
}

fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

fn all_space(text: &str) -> bool {
    text.chars().all(is_space)
}

fn special(name: QualName) -> bool {
    match name.ns {
        Namespace::Html => matches!(
            name.local,
            Atom::ADDRESS
                | Atom::APPLET
                | Atom::AREA
                | Atom::ARTICLE
                | Atom::ASIDE
                | Atom::BASE
                | Atom::BASEFONT
                | Atom::BGSOUND
                | Atom::BLOCKQUOTE
                | Atom::BODY
                | Atom::BR
                | Atom::BUTTON
                | Atom::CAPTION
                | Atom::CENTER
                | Atom::COL
                | Atom::COLGROUP
                | Atom::DD
                | Atom::DETAILS
                | Atom::DIR
                | Atom::DIV
                | Atom::DL
                | Atom::DT
                | Atom::EMBED
                | Atom::FIELDSET
                | Atom::FIGCAPTION
                | Atom::FIGURE
                | Atom::FOOTER
                | Atom::FORM
                | Atom::FRAME
                | Atom::FRAMESET
                | Atom::H1
                | Atom::H2
                | Atom::H3
                | Atom::H4
                | Atom::H5
                | Atom::H6
                | Atom::HEAD
                | Atom::HEADER
                | Atom::HGROUP
                | Atom::HR
                | Atom::HTML
                | Atom::IFRAME
                | Atom::IMG
                | Atom::INPUT
                | Atom::KEYGEN
                | Atom::LI
                | Atom::LINK
                | Atom::LISTING
                | Atom::MAIN
                | Atom::MARQUEE
                | Atom::MENU
                | Atom::META
                | Atom::NAV
                | Atom::NOEMBED
                | Atom::NOFRAMES
                | Atom::NOSCRIPT
                | Atom::OBJECT
                | Atom::OL
                | Atom::P
                | Atom::PARAM
                | Atom::PLAINTEXT
                | Atom::PRE
                | Atom::SCRIPT
                | Atom::SEARCH
                | Atom::SECTION
                | Atom::SELECT
                | Atom::SOURCE
                | Atom::STYLE
                | Atom::SUMMARY
                | Atom::TABLE
                | Atom::TBODY
                | Atom::TD
                | Atom::TEMPLATE
                | Atom::TEXTAREA
                | Atom::TFOOT
                | Atom::TH
                | Atom::THEAD
                | Atom::TITLE
                | Atom::TR
                | Atom::TRACK
                | Atom::UL
                | Atom::WBR
                | Atom::XMP
        ),
        Namespace::MathMl => mathml_text_integration(name) || name.local == Atom::ANNOTATION_XML,
        Namespace::Svg => svg_html_integration(name),
        _ => false,
    }
}

fn mathml_text_integration(name: QualName) -> bool {
    name.ns == Namespace::MathMl
        && matches!(
            name.local,
            Atom::MI | Atom::MO | Atom::MN | Atom::MS | Atom::MTEXT
        )
}

fn svg_html_integration(name: QualName) -> bool {
    name.ns == Namespace::Svg
        && matches!(name.local, Atom::FOREIGN_OBJECT | Atom::DESC | Atom::TITLE)
}

fn scope_boundary(name: QualName, scope: Scope) -> bool {
    let html_local = (name.ns == Namespace::Html).then_some(name.local);
    let base = || {
        matches!(
            html_local,
            Some(
                Atom::APPLET
                    | Atom::CAPTION
                    | Atom::HTML
                    | Atom::TABLE
                    | Atom::TD
                    | Atom::TH
                    | Atom::MARQUEE
                    | Atom::OBJECT
                    | Atom::SELECT
                    | Atom::TEMPLATE
            )
        ) || mathml_text_integration(name)
            || (name.ns == Namespace::MathMl && name.local == Atom::ANNOTATION_XML)
            || svg_html_integration(name)
    };
    match scope {
        Scope::Default => base(),
        Scope::ListItem => base() || matches!(html_local, Some(Atom::OL | Atom::UL)),
        Scope::Button => base() || html_local == Some(Atom::BUTTON),
        Scope::Table => matches!(html_local, Some(Atom::HTML | Atom::TABLE | Atom::TEMPLATE)),
    }
}

fn implied_end_tag(local: Atom) -> bool {
    matches!(
        local,
        Atom::DD
            | Atom::DT
            | Atom::LI
            | Atom::OPTGROUP
            | Atom::OPTION
            | Atom::P
            | Atom::RB
            | Atom::RP
            | Atom::RT
            | Atom::RTC
    )
}

fn thorough_end_tag(local: Atom) -> bool {
    implied_end_tag(local)
        || matches!(
            local,
            Atom::CAPTION
                | Atom::COLGROUP
                | Atom::TBODY
                | Atom::TD
                | Atom::TFOOT
                | Atom::TH
                | Atom::THEAD
                | Atom::TR
        )
}

fn quirky_doctype(doctype: &Doctype) -> QuirksMode {
    const QUIRKY_PREFIXES: &[&str] = &[
        "+//silmaril//dtd html pro v0r11 19970101//",
        "-//as//dtd html 3.0 aswedit + extensions//",
        "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
        "-//ietf//dtd html 2.0 level 1//",
        "-//ietf//dtd html 2.0 level 2//",
        "-//ietf//dtd html 2.0 strict level 1//",
        "-//ietf//dtd html 2.0 strict level 2//",
        "-//ietf//dtd html 2.0 strict//",
        "-//ietf//dtd html 2.0//",
        "-//ietf//dtd html 2.1e//",
        "-//ietf//dtd html 3.0//",
        "-//ietf//dtd html 3.2 final//",
        "-//ietf//dtd html 3.2//",
        "-//ietf//dtd html 3//",
        "-//ietf//dtd html level 0//",
        "-//ietf//dtd html level 1//",
        "-//ietf//dtd html level 2//",
        "-//ietf//dtd html level 3//",
        "-//ietf//dtd html strict level 0//",
        "-//ietf//dtd html strict level 1//",
        "-//ietf//dtd html strict level 2//",
        "-//ietf//dtd html strict level 3//",
        "-//ietf//dtd html strict//",
        "-//ietf//dtd html//",
        "-//metrius//dtd metrius presentational//",
        "-//microsoft//dtd internet explorer 2.0 html strict//",
        "-//microsoft//dtd internet explorer 2.0 html//",
        "-//microsoft//dtd internet explorer 2.0 tables//",
        "-//microsoft//dtd internet explorer 3.0 html strict//",
        "-//microsoft//dtd internet explorer 3.0 html//",
        "-//microsoft//dtd internet explorer 3.0 tables//",
        "-//netscape comm. corp.//dtd html//",
        "-//netscape comm. corp.//dtd strict html//",
        "-//o'reilly and associates//dtd html 2.0//",
        "-//o'reilly and associates//dtd html extended 1.0//",
        "-//o'reilly and associates//dtd html extended relaxed 1.0//",
        "-//sq//dtd html 2.0 hotmetal + extensions//",
        "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
        "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
        "-//spyglass//dtd html 2.0 extended//",
        "-//sun microsystems corp.//dtd hotjava html//",
        "-//sun microsystems corp.//dtd hotjava strict html//",
        "-//w3c//dtd html 3 1995-03-24//",
        "-//w3c//dtd html 3.2 draft//",
        "-//w3c//dtd html 3.2 final//",
        "-//w3c//dtd html 3.2//",
        "-//w3c//dtd html 3.2s draft//",
        "-//w3c//dtd html 4.0 frameset//",
        "-//w3c//dtd html 4.0 transitional//",
        "-//w3c//dtd html experimental 19960712//",
        "-//w3c//dtd html experimental 970421//",
        "-//w3c//dtd w3 html//",
        "-//w3o//dtd w3 html 3.0//",
        "-//webtechs//dtd mozilla html 2.0//",
        "-//webtechs//dtd mozilla html//",
    ];
    let public = doctype.public_id.as_deref().map(str::to_ascii_lowercase);
    let system = doctype.system_id.as_deref().map(str::to_ascii_lowercase);
    let public_is = |s: &str| public.as_deref() == Some(s);
    let public_starts = |s: &str| public.as_deref().is_some_and(|p| p.starts_with(s));
    let html401 = public_starts("-//w3c//dtd html 4.01 frameset//")
        || public_starts("-//w3c//dtd html 4.01 transitional//");
    if doctype.force_quirks
        || doctype.name.as_deref() != Some("html")
        || public_is("-//w3o//dtd w3 html strict 3.0//en//")
        || public_is("-/w3c/dtd html 4.0 transitional/en")
        || public_is("html")
        || system.as_deref() == Some("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
        || QUIRKY_PREFIXES.iter().any(|prefix| public_starts(prefix))
        || (system.is_none() && html401)
    {
        QuirksMode::Quirks
    } else if public_starts("-//w3c//dtd xhtml 1.0 frameset//")
        || public_starts("-//w3c//dtd xhtml 1.0 transitional//")
        || (system.is_some() && html401)
    {
        QuirksMode::LimitedQuirks
    } else {
        QuirksMode::NoQuirks
    }
}

impl<S: TreeSink> TreeBuilder<S> {
    pub(crate) fn new(sink: S, scripting: bool) -> Self {
        TreeBuilder {
            sink,
            tokenizer_state: None,
            declared_encoding: None,
            mode: Mode::Initial,
            original_mode: Mode::Initial,
            template_modes: Vec::new(),
            open: Vec::new(),
            formatting: Vec::new(),
            head: None,
            form: None,
            context: None,
            scripting,
            frameset_ok: true,
            foster: false,
            quirks: QuirksMode::NoQuirks,
            skip_newline: false,
            table_text: String::new(),
            table_text_has_content: false,
        }
    }

    pub(crate) fn start_fragment(&mut self, context: QualName, context_tag: &Tag) -> State {
        let root = self
            .sink
            .create_element(html(Atom::HTML), Tag::default().attributes());
        let document = self.sink.document();
        self.sink.insert(document, root, None);
        self.open.push(Open {
            node: root,
            name: html(Atom::HTML),
            html_integration: false,
        });
        let integration = Self::html_integration(context, context_tag);
        self.context = Some((context, integration));
        if context == html(Atom::TEMPLATE) {
            self.template_modes.push(Mode::InTemplate);
        }
        self.reset_mode();
        if context.ns != Namespace::Html {
            return State::Data;
        }
        match context.local {
            Atom::TITLE | Atom::TEXTAREA => State::Rcdata,
            Atom::STYLE | Atom::XMP | Atom::IFRAME | Atom::NOEMBED | Atom::NOFRAMES => {
                State::Rawtext
            }
            Atom::SCRIPT => State::ScriptData,
            Atom::NOSCRIPT if self.scripting => State::Rawtext,
            Atom::PLAINTEXT => State::Plaintext,
            _ => State::Data,
        }
    }

    pub(crate) fn foreign_cdata_allowed(&self) -> bool {
        self.adjusted_current()
            .is_some_and(|(name, _)| name.ns != Namespace::Html)
    }

    pub(crate) fn process(&mut self, token: Token<'_>) {
        match token {
            Token::Text(text) => self.process_text(text),
            Token::StartTag(tag) => self.dispatch(&mut Tok::Start(tag)),
            Token::EndTag(tag) => self.dispatch(&mut Tok::End(tag)),
            Token::Comment(text) => self.dispatch(&mut Tok::Comment(text)),
            Token::ProcessingInstruction(target, data) => self.dispatch(&mut Tok::Pi(target, data)),
            Token::Doctype(doctype) => self.dispatch(&mut Tok::Doctype(doctype)),
            Token::Eof => self.dispatch(&mut Tok::Eof),
        }
    }

    fn process_text(&mut self, mut text: &str) {
        if std::mem::take(&mut self.skip_newline) {
            text = text.strip_prefix('\n').unwrap_or(text);
        }
        while let Some(first) = text.chars().next() {
            let end = if is_space(first) {
                text.find(|c| !is_space(c))
            } else if first == '\0' {
                text.find(|c| c != '\0')
            } else {
                text.find('\0')
            }
            .unwrap_or(text.len());
            let (segment, rest) = text.split_at(end);
            text = rest;
            self.dispatch(&mut Tok::Chars(segment));
        }
    }

    fn adjusted_current(&self) -> Option<(QualName, bool)> {
        match (self.context, self.open.len()) {
            (Some(context), 1) => Some(context),
            _ => self
                .open
                .last()
                .map(|open| (open.name, open.html_integration)),
        }
    }

    fn dispatch(&mut self, tok: &mut Tok<'_>) {
        if !matches!(tok, Tok::Start(_)) {
            self.skip_newline = false;
        }
        let html_rules = match self.adjusted_current() {
            None => true,
            Some((name, integration)) => {
                name.ns == Namespace::Html
                    || matches!(tok, Tok::Eof)
                    || (mathml_text_integration(name)
                        && match tok {
                            Tok::Start(tag) => {
                                tag.name != Atom::MGLYPH && tag.name != Atom::MALIGNMARK
                            }
                            Tok::Chars(_) => true,
                            _ => false,
                        })
                    || (name.ns == Namespace::MathMl
                        && name.local == Atom::ANNOTATION_XML
                        && matches!(tok, Tok::Start(tag) if tag.name == Atom::SVG))
                    || (integration && matches!(tok, Tok::Start(_) | Tok::Chars(_)))
            }
        };
        if html_rules {
            self.html(self.mode, tok);
        } else {
            self.foreign(tok);
        }
    }

    fn current(&self) -> Option<&Open<S::Node>> {
        self.open.last()
    }

    fn current_is(&self, local: Atom) -> bool {
        self.current().is_some_and(|open| open.name == html(local))
    }

    fn current_index(&self) -> usize {
        self.open.len().saturating_sub(1)
    }

    fn in_scope_where(&self, scope: Scope, target: impl Fn(&Open<S::Node>) -> bool) -> bool {
        for open in self.open.iter().rev() {
            if target(open) {
                return true;
            }
            if scope_boundary(open.name, scope) {
                return false;
            }
        }
        false
    }

    fn in_scope(&self, local: Atom, scope: Scope) -> bool {
        self.in_scope_where(scope, |open| open.name == html(local))
    }

    fn in_template_contents(&self) -> bool {
        self.has_open(Atom::TEMPLATE)
            || self
                .context
                .is_some_and(|(name, _)| name == html(Atom::TEMPLATE))
    }

    fn has_open(&self, local: Atom) -> bool {
        self.open.iter().any(|open| open.name == html(local))
    }

    fn pop(&mut self) {
        self.open.pop();
    }

    fn pop_until(&mut self, local: Atom) {
        while let Some(open) = self.open.pop() {
            if open.name == html(local) {
                break;
            }
        }
    }

    fn pop_until_one_of(&mut self, locals: &[Atom]) {
        while let Some(open) = self.open.pop() {
            if open.name.ns == Namespace::Html && locals.contains(&open.name.local) {
                break;
            }
        }
    }

    fn generate_implied_end_tags(&mut self, except: Option<Atom>) {
        while let Some(open) = self.current() {
            if open.name.ns != Namespace::Html
                || !implied_end_tag(open.name.local)
                || Some(open.name.local) == except
            {
                break;
            }
            self.pop();
        }
    }

    fn generate_all_implied_end_tags(&mut self) {
        while self.current().is_some_and(|open| {
            open.name.ns == Namespace::Html && thorough_end_tag(open.name.local)
        }) {
            self.pop();
        }
    }

    fn close_p(&mut self) {
        self.generate_implied_end_tags(Some(Atom::P));
        self.pop_until(Atom::P);
    }

    fn close_p_in_button_scope(&mut self) {
        if self.in_scope(Atom::P, Scope::Button) {
            self.close_p();
        }
    }

    fn appropriate_place(&mut self, target: usize) -> Option<(S::Node, Option<S::Node>)> {
        let mut target = target;
        let open = self.open.get(target)?;
        let fosterable = open.name.ns == Namespace::Html
            && matches!(
                open.name.local,
                Atom::TABLE | Atom::TBODY | Atom::TFOOT | Atom::THEAD | Atom::TR
            );
        if self.foster && fosterable {
            let template = self
                .open
                .iter()
                .rposition(|o| o.name == html(Atom::TEMPLATE));
            let table = self.open.iter().rposition(|o| o.name == html(Atom::TABLE));
            match (template, table) {
                (Some(template), table) if table.is_none_or(|table| template > table) => {
                    target = template;
                }
                (_, None) => target = 0,
                (_, Some(table)) => {
                    let table_node = self.open.get(table)?.node;
                    if let Some(parent) = self.sink.parent(table_node) {
                        return Some((parent, Some(table_node)));
                    }
                    target = table.saturating_sub(1);
                }
            }
        }
        let open = *self.open.get(target)?;
        if open.name == html(Atom::TEMPLATE) {
            return Some((self.sink.template_contents(open.node), None));
        }
        Some((open.node, None))
    }

    fn insert_at_current(&mut self, node: S::Node) {
        if let Some((parent, before)) = self.appropriate_place(self.current_index()) {
            self.sink.insert(parent, node, before);
        }
    }

    fn html_integration(name: QualName, tag: &Tag) -> bool {
        svg_html_integration(name)
            || (name.ns == Namespace::MathMl
                && name.local == Atom::ANNOTATION_XML
                && tag.attribute(Atom::ENCODING).is_some_and(|encoding| {
                    encoding.eq_ignore_ascii_case("text/html")
                        || encoding.eq_ignore_ascii_case("application/xhtml+xml")
                }))
    }

    fn insert_element(&mut self, tag: &Tag, ns: Namespace) -> S::Node {
        let name = QualName {
            ns,
            local: tag.name,
        };
        let node = self.sink.create_element(name, tag.attributes());
        self.insert_at_current(node);
        self.open.push(Open {
            node,
            name,
            html_integration: Self::html_integration(name, tag),
        });
        node
    }

    fn insert_html(&mut self, tag: &Tag) -> S::Node {
        self.insert_element(tag, Namespace::Html)
    }

    fn insert_implied(&mut self, local: Atom) -> S::Node {
        self.insert_html(&Tag {
            name: local,
            ..Tag::default()
        })
    }

    fn insert_void(&mut self, tag: &Tag) {
        self.insert_html(tag);
        self.pop();
    }

    fn insert_text(&mut self, text: &str) {
        if let Some((parent, before)) = self.appropriate_place(self.current_index()) {
            self.sink.insert_text(parent, before, text);
        }
    }

    fn insert_comment_like(&mut self, tok: &Tok<'_>, place: Option<S::Node>) {
        let node = match tok {
            Tok::Comment(text) => self.sink.create_comment(text),
            Tok::Pi(target, data) => self.sink.create_processing_instruction(target, data),
            _ => return,
        };
        match place {
            Some(parent) => self.sink.insert(parent, node, None),
            None => self.insert_at_current(node),
        }
    }

    fn switch_tokenizer(&mut self, state: State) {
        self.tokenizer_state = Some(state);
    }

    fn generic_text_element(&mut self, tag: &Tag, state: State) {
        self.insert_html(tag);
        self.switch_tokenizer(state);
        self.original_mode = self.mode;
        self.mode = Mode::Text;
    }

    fn formatting_index(&self, node: S::Node) -> Option<usize> {
        self.formatting
            .iter()
            .rposition(|entry| matches!(entry, Formatting::Element { node: n, .. } if *n == node))
    }

    fn open_index(&self, node: S::Node) -> Option<usize> {
        self.open.iter().rposition(|open| open.node == node)
    }

    fn push_formatting(&mut self, node: S::Node, tag: &Tag) {
        let mut same = 0;
        let mut earliest = None;
        for (i, entry) in self.formatting.iter().enumerate().rev() {
            match entry {
                Formatting::Marker => break,
                Formatting::Element { tag: other, .. } => {
                    if other.name == tag.name && other.same_attributes(tag) {
                        same += 1;
                        earliest = Some(i);
                    }
                }
            }
        }
        if same >= 3
            && let Some(i) = earliest
        {
            self.formatting.remove(i);
        }
        self.formatting.push(Formatting::Element {
            node,
            tag: tag.clone(),
        });
    }

    fn reconstruct_formatting(&mut self) {
        let needs = |builder: &Self, entry: &Formatting<S::Node>| match entry {
            Formatting::Marker => false,
            Formatting::Element { node, .. } => builder.open_index(*node).is_none(),
        };
        let Some(last) = self.formatting.last() else {
            return;
        };
        if !needs(self, last) {
            return;
        }
        let mut start = self.formatting.len() - 1;
        while start > 0 {
            let Some(entry) = self.formatting.get(start - 1) else {
                break;
            };
            if !needs(self, entry) {
                break;
            }
            start -= 1;
        }
        for i in start..self.formatting.len() {
            let Some(Formatting::Element { tag, .. }) = self.formatting.get(i) else {
                continue;
            };
            let tag = tag.clone();
            let node = self.insert_html(&tag);
            if let Some(entry) = self.formatting.get_mut(i) {
                *entry = Formatting::Element { node, tag };
            }
        }
    }

    fn clear_formatting_to_marker(&mut self) {
        while let Some(entry) = self.formatting.pop() {
            if matches!(entry, Formatting::Marker) {
                break;
            }
        }
    }

    fn reset_mode(&mut self) {
        for i in (0..self.open.len()).rev() {
            let last = i == 0;
            let name = match (last, self.context) {
                (true, Some((context, _))) => context,
                _ => match self.open.get(i) {
                    Some(open) => open.name,
                    None => continue,
                },
            };
            if name.ns == Namespace::Html {
                let mode = match name.local {
                    Atom::TD | Atom::TH if !last => Some(Mode::InCell),
                    Atom::TR => Some(Mode::InRow),
                    Atom::TBODY | Atom::THEAD | Atom::TFOOT => Some(Mode::InTableBody),
                    Atom::CAPTION => Some(Mode::InCaption),
                    Atom::COLGROUP => Some(Mode::InColumnGroup),
                    Atom::TABLE => Some(Mode::InTable),
                    Atom::TEMPLATE => self.template_modes.last().copied(),
                    Atom::HEAD if !last => Some(Mode::InHead),
                    Atom::BODY => Some(Mode::InBody),
                    Atom::FRAMESET => Some(Mode::InFrameset),
                    Atom::HTML => Some(if self.head.is_none() {
                        Mode::BeforeHead
                    } else {
                        Mode::AfterHead
                    }),
                    _ => None,
                };
                if let Some(mode) = mode {
                    self.mode = mode;
                    return;
                }
            }
            if last {
                self.mode = Mode::InBody;
                return;
            }
        }
        self.mode = Mode::InBody;
    }

    fn html(&mut self, mode: Mode, tok: &mut Tok<'_>) {
        match mode {
            Mode::Initial => self.initial(tok),
            Mode::BeforeHtml => self.before_html(tok),
            Mode::BeforeHead => self.before_head(tok),
            Mode::InHead => self.in_head(tok),
            Mode::InHeadNoscript => self.in_head_noscript(tok),
            Mode::AfterHead => self.after_head(tok),
            Mode::InBody => self.in_body(tok),
            Mode::Text => self.text(tok),
            Mode::InTable => self.in_table(tok),
            Mode::InTableText => self.in_table_text(tok),
            Mode::InCaption => self.in_caption(tok),
            Mode::InColumnGroup => self.in_column_group(tok),
            Mode::InTableBody => self.in_table_body(tok),
            Mode::InRow => self.in_row(tok),
            Mode::InCell => self.in_cell(tok),
            Mode::InTemplate => self.in_template(tok),
            Mode::AfterBody => self.after_body(tok),
            Mode::InFrameset | Mode::AfterFrameset => self.frameset(mode, tok),
            Mode::AfterAfterBody => self.after_after_body(tok),
            Mode::AfterAfterFrameset => self.after_after_frameset(tok),
        }
    }

    fn reprocess(&mut self, mode: Mode, tok: &mut Tok<'_>) {
        self.mode = mode;
        self.dispatch(tok);
    }

    fn split_leading_space<'t>(&mut self, tok: &mut Tok<'t>) -> Option<&'t str> {
        let Tok::Chars(text) = *tok else {
            return None;
        };
        let end = text.find(|c| !is_space(c)).unwrap_or(text.len());
        let (space, rest) = text.split_at(end);
        *tok = Tok::Chars(rest);
        Some(space)
    }

    fn initial(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) if all_space(text) => {}
            Tok::Comment(_) | Tok::Pi(..) => {
                let document = self.sink.document();
                self.insert_comment_like(tok, Some(document));
            }
            Tok::Doctype(doctype) => {
                self.sink.append_doctype(
                    doctype.name.as_deref().unwrap_or_default(),
                    doctype.public_id.as_deref().unwrap_or_default(),
                    doctype.system_id.as_deref().unwrap_or_default(),
                );
                self.quirks = quirky_doctype(doctype);
                self.sink.set_quirks_mode(self.quirks);
                self.mode = Mode::BeforeHtml;
            }
            _ => {
                self.quirks = QuirksMode::Quirks;
                self.sink.set_quirks_mode(self.quirks);
                self.reprocess(Mode::BeforeHtml, tok);
            }
        }
    }

    fn create_root(&mut self, tag: &Tag) {
        let node = self.sink.create_element(html(Atom::HTML), tag.attributes());
        let document = self.sink.document();
        self.sink.insert(document, node, None);
        self.open.push(Open {
            node,
            name: html(Atom::HTML),
            html_integration: false,
        });
    }

    fn before_html(&mut self, tok: &mut Tok<'_>) {
        if let Some(space) = self.split_leading_space(tok)
            && !space.is_empty()
            && matches!(tok, Tok::Chars(""))
        {
            return;
        }
        match tok {
            Tok::Chars("") => {}
            Tok::Doctype(_) => {}
            Tok::Comment(_) | Tok::Pi(..) => {
                let document = self.sink.document();
                self.insert_comment_like(tok, Some(document));
            }
            Tok::Start(tag) if tag.name == Atom::HTML => {
                self.create_root(tag);
                self.mode = Mode::BeforeHead;
            }
            Tok::End(tag)
                if !matches!(tag.name, Atom::HEAD | Atom::BODY | Atom::HTML | Atom::BR) => {}
            _ => {
                self.create_root(&Tag::default());
                self.reprocess(Mode::BeforeHead, tok);
            }
        }
    }

    fn before_head(&mut self, tok: &mut Tok<'_>) {
        if let Some(_space) = self.split_leading_space(tok)
            && matches!(tok, Tok::Chars(""))
        {
            return;
        }
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::Start(tag) if tag.name == Atom::HEAD => {
                self.head = Some(self.insert_html(tag));
                self.mode = Mode::InHead;
            }
            Tok::End(tag)
                if !matches!(tag.name, Atom::HEAD | Atom::BODY | Atom::HTML | Atom::BR) => {}
            _ => {
                self.head = Some(self.insert_implied(Atom::HEAD));
                self.reprocess(Mode::InHead, tok);
            }
        }
    }

    fn in_head(&mut self, tok: &mut Tok<'_>) {
        if let Some(space) = self.split_leading_space(tok) {
            if !space.is_empty() {
                self.insert_text(space);
            }
            if matches!(tok, Tok::Chars("")) {
                return;
            }
        }
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) => match tag.name {
                Atom::HTML => self.in_body(tok),
                Atom::BASE | Atom::BASEFONT | Atom::BGSOUND | Atom::LINK => self.insert_void(tag),
                Atom::META => {
                    self.insert_void(tag);
                    self.note_meta_charset(tag);
                }
                Atom::TITLE => self.generic_text_element(tag, State::Rcdata),
                Atom::NOSCRIPT if self.scripting => self.generic_text_element(tag, State::Rawtext),
                Atom::NOFRAMES | Atom::STYLE => self.generic_text_element(tag, State::Rawtext),
                Atom::NOSCRIPT => {
                    self.insert_html(tag);
                    self.mode = Mode::InHeadNoscript;
                }
                Atom::SCRIPT => self.generic_text_element(tag, State::ScriptData),
                Atom::TEMPLATE => {
                    self.insert_html(tag);
                    self.formatting.push(Formatting::Marker);
                    self.frameset_ok = false;
                    self.mode = Mode::InTemplate;
                    self.template_modes.push(Mode::InTemplate);
                }
                Atom::HEAD => {}
                _ => self.leave_head(tok),
            },
            Tok::End(tag) => match tag.name {
                Atom::HEAD => {
                    self.pop();
                    self.mode = Mode::AfterHead;
                }
                Atom::BODY | Atom::HTML | Atom::BR => self.leave_head(tok),
                Atom::TEMPLATE => {
                    if !self.has_open(Atom::TEMPLATE) {
                        return;
                    }
                    self.generate_all_implied_end_tags();
                    self.pop_until(Atom::TEMPLATE);
                    self.clear_formatting_to_marker();
                    self.template_modes.pop();
                    self.reset_mode();
                }
                _ => {}
            },
            _ => self.leave_head(tok),
        }
    }

    fn leave_head(&mut self, tok: &mut Tok<'_>) {
        self.pop();
        self.reprocess(Mode::AfterHead, tok);
    }

    fn note_meta_charset(&mut self, tag: &Tag) {
        let declared = match tag.attribute(Atom::CHARSET) {
            Some(charset) => label_encoding(charset),
            None => tag
                .attribute(Atom::HTTP_EQUIV)
                .filter(|value| value.eq_ignore_ascii_case("content-type"))
                .and_then(|_| tag.attribute(Atom::CONTENT))
                .and_then(|content| charset_from_meta_content(content.as_bytes())),
        };
        if declared.is_some() && self.declared_encoding.is_none() {
            self.declared_encoding = declared;
        }
    }

    fn in_head_noscript(&mut self, tok: &mut Tok<'_>) {
        if let Some(space) = self.split_leading_space(tok) {
            if !space.is_empty() {
                self.insert_text(space);
            }
            if matches!(tok, Tok::Chars("")) {
                return;
            }
        }
        match tok {
            Tok::Doctype(_) => {}
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::End(tag) if tag.name == Atom::NOSCRIPT => {
                self.pop();
                self.mode = Mode::InHead;
            }
            Tok::Comment(_) | Tok::Pi(..) => self.in_head(tok),
            Tok::Start(tag)
                if matches!(
                    tag.name,
                    Atom::BASEFONT
                        | Atom::BGSOUND
                        | Atom::LINK
                        | Atom::META
                        | Atom::NOFRAMES
                        | Atom::STYLE
                ) =>
            {
                self.in_head(tok)
            }
            Tok::Start(tag) if matches!(tag.name, Atom::HEAD | Atom::NOSCRIPT) => {}
            Tok::End(tag) if tag.name != Atom::BR => {}
            _ => {
                self.pop();
                self.reprocess(Mode::InHead, tok);
            }
        }
    }

    fn after_head(&mut self, tok: &mut Tok<'_>) {
        if let Some(space) = self.split_leading_space(tok) {
            if !space.is_empty() {
                self.insert_text(space);
            }
            if matches!(tok, Tok::Chars("")) {
                return;
            }
        }
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) => match tag.name {
                Atom::HTML => self.in_body(tok),
                Atom::BODY => {
                    self.insert_html(tag);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                Atom::FRAMESET => {
                    self.insert_html(tag);
                    self.mode = Mode::InFrameset;
                }
                Atom::BASE
                | Atom::BASEFONT
                | Atom::BGSOUND
                | Atom::LINK
                | Atom::META
                | Atom::NOFRAMES
                | Atom::SCRIPT
                | Atom::STYLE
                | Atom::TEMPLATE
                | Atom::TITLE => {
                    let Some(head) = self.head else {
                        return;
                    };
                    self.open.push(Open {
                        node: head,
                        name: html(Atom::HEAD),
                        html_integration: false,
                    });
                    self.in_head(tok);
                    if let Some(i) = self.open_index(head) {
                        self.open.remove(i);
                    }
                }
                Atom::HEAD => {}
                _ => self.implied_body(tok),
            },
            Tok::End(tag) => match tag.name {
                Atom::TEMPLATE => self.in_head(tok),
                Atom::BODY | Atom::HTML | Atom::BR => self.implied_body(tok),
                _ => {}
            },
            _ => self.implied_body(tok),
        }
    }

    fn implied_body(&mut self, tok: &mut Tok<'_>) {
        self.insert_implied(Atom::BODY);
        self.frameset_ok = true;
        self.reprocess(Mode::InBody, tok);
    }

    fn in_body(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) => {
                if text.starts_with('\0') {
                    return;
                }
                self.reconstruct_formatting();
                self.insert_text(text);
                if !all_space(text) {
                    self.frameset_ok = false;
                }
            }
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) => self.in_body_start(tag),
            Tok::End(tag) => self.in_body_end(tag),
            Tok::Eof => {
                if !self.template_modes.is_empty() {
                    self.in_template(tok);
                }
            }
        }
    }

    fn in_body_start(&mut self, tag: &mut Tag) {
        match tag.name {
            Atom::HTML => {
                if self.has_open(Atom::TEMPLATE) {
                    return;
                }
                if let Some(root) = self.open.first().map(|open| open.node) {
                    self.sink.add_attributes_if_missing(root, tag.attributes());
                }
            }
            Atom::BASE
            | Atom::BASEFONT
            | Atom::BGSOUND
            | Atom::LINK
            | Atom::META
            | Atom::NOFRAMES
            | Atom::SCRIPT
            | Atom::STYLE
            | Atom::TEMPLATE
            | Atom::TITLE => self.in_head(&mut Tok::Start(tag)),
            Atom::BODY => {
                let body = self
                    .open
                    .get(1)
                    .filter(|open| open.name == html(Atom::BODY));
                let Some(body) = body.map(|open| open.node) else {
                    return;
                };
                if self.open.len() == 1 || self.has_open(Atom::TEMPLATE) {
                    return;
                }
                self.frameset_ok = false;
                self.sink.add_attributes_if_missing(body, tag.attributes());
            }
            Atom::FRAMESET => {
                let body = self
                    .open
                    .get(1)
                    .filter(|open| open.name == html(Atom::BODY));
                let Some(body) = body.map(|open| open.node) else {
                    return;
                };
                if !self.frameset_ok {
                    return;
                }
                self.sink.detach(body);
                self.open.truncate(1);
                self.insert_html(tag);
                self.mode = Mode::InFrameset;
            }
            Atom::ADDRESS
            | Atom::ARTICLE
            | Atom::ASIDE
            | Atom::BLOCKQUOTE
            | Atom::CENTER
            | Atom::DETAILS
            | Atom::DIALOG
            | Atom::DIR
            | Atom::DIV
            | Atom::DL
            | Atom::FIELDSET
            | Atom::FIGCAPTION
            | Atom::FIGURE
            | Atom::FOOTER
            | Atom::HEADER
            | Atom::HGROUP
            | Atom::MAIN
            | Atom::MENU
            | Atom::NAV
            | Atom::OL
            | Atom::P
            | Atom::SEARCH
            | Atom::SECTION
            | Atom::SUMMARY
            | Atom::UL => {
                self.close_p_in_button_scope();
                self.insert_html(tag);
            }
            Atom::H1 | Atom::H2 | Atom::H3 | Atom::H4 | Atom::H5 | Atom::H6 => {
                self.close_p_in_button_scope();
                if self.current().is_some_and(|open| {
                    open.name.ns == Namespace::Html
                        && matches!(
                            open.name.local,
                            Atom::H1 | Atom::H2 | Atom::H3 | Atom::H4 | Atom::H5 | Atom::H6
                        )
                }) {
                    self.pop();
                }
                self.insert_html(tag);
            }
            Atom::PRE | Atom::LISTING => {
                self.close_p_in_button_scope();
                self.insert_html(tag);
                self.skip_newline = true;
                self.frameset_ok = false;
            }
            Atom::FORM => {
                let in_template = self.in_template_contents();
                if self.form.is_some() && !in_template {
                    return;
                }
                self.close_p_in_button_scope();
                let node = self.insert_html(tag);
                if !in_template {
                    self.form = Some(node);
                }
            }
            Atom::LI | Atom::DD | Atom::DT => {
                self.frameset_ok = false;
                let closes: &[Atom] = if tag.name == Atom::LI {
                    &[Atom::LI]
                } else {
                    &[Atom::DD, Atom::DT]
                };
                for i in (0..self.open.len()).rev() {
                    let Some(name) = self.open.get(i).map(|open| open.name) else {
                        break;
                    };
                    if name.ns == Namespace::Html && closes.contains(&name.local) {
                        self.generate_implied_end_tags(Some(name.local));
                        self.pop_until(name.local);
                        break;
                    }
                    if special(name)
                        && !matches!(name, n if n == html(Atom::ADDRESS) || n == html(Atom::DIV) || n == html(Atom::P))
                    {
                        break;
                    }
                }
                self.close_p_in_button_scope();
                self.insert_html(tag);
            }
            Atom::PLAINTEXT => {
                self.close_p_in_button_scope();
                self.insert_html(tag);
                self.switch_tokenizer(State::Plaintext);
            }
            Atom::BUTTON => {
                if self.in_scope(Atom::BUTTON, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(Atom::BUTTON);
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
                self.frameset_ok = false;
            }
            Atom::A => {
                let existing = self
                    .formatting
                    .iter()
                    .rev()
                    .take_while(|entry| !matches!(entry, Formatting::Marker))
                    .find_map(|entry| match entry {
                        Formatting::Element { node, tag } if tag.name == Atom::A => Some(*node),
                        _ => None,
                    });
                if let Some(node) = existing {
                    self.adoption_agency(Atom::A);
                    if let Some(i) = self.formatting_index(node) {
                        self.formatting.remove(i);
                    }
                    if let Some(i) = self.open_index(node) {
                        self.open.remove(i);
                    }
                }
                self.reconstruct_formatting();
                let node = self.insert_html(tag);
                self.push_formatting(node, tag);
            }
            Atom::B
            | Atom::BIG
            | Atom::CODE
            | Atom::EM
            | Atom::FONT
            | Atom::I
            | Atom::S
            | Atom::SMALL
            | Atom::STRIKE
            | Atom::STRONG
            | Atom::TT
            | Atom::U => {
                self.reconstruct_formatting();
                let node = self.insert_html(tag);
                self.push_formatting(node, tag);
            }
            Atom::NOBR => {
                self.reconstruct_formatting();
                if self.in_scope(Atom::NOBR, Scope::Default) {
                    self.adoption_agency(Atom::NOBR);
                    self.reconstruct_formatting();
                }
                let node = self.insert_html(tag);
                self.push_formatting(node, tag);
            }
            Atom::APPLET | Atom::MARQUEE | Atom::OBJECT => {
                self.reconstruct_formatting();
                self.insert_html(tag);
                self.formatting.push(Formatting::Marker);
                self.frameset_ok = false;
            }
            Atom::TABLE => {
                if self.quirks != QuirksMode::Quirks {
                    self.close_p_in_button_scope();
                }
                self.insert_html(tag);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            Atom::AREA | Atom::BR | Atom::EMBED | Atom::IMG | Atom::KEYGEN | Atom::WBR => {
                self.reconstruct_formatting();
                self.insert_void(tag);
                self.frameset_ok = false;
            }
            Atom::INPUT => {
                if self
                    .context
                    .is_some_and(|(name, _)| name == html(Atom::SELECT))
                {
                    return;
                }
                if self.in_scope(Atom::SELECT, Scope::Default) {
                    self.pop_until(Atom::SELECT);
                }
                self.reconstruct_formatting();
                self.insert_void(tag);
                if !tag
                    .attribute(Atom::TYPE)
                    .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
                {
                    self.frameset_ok = false;
                }
            }
            Atom::PARAM | Atom::SOURCE | Atom::TRACK => self.insert_void(tag),
            Atom::HR => {
                self.close_p_in_button_scope();
                if self.in_scope(Atom::SELECT, Scope::Default) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_void(tag);
                self.frameset_ok = false;
            }
            Atom::IMAGE => {
                tag.name = Atom::IMG;
                self.dispatch(&mut Tok::Start(tag));
            }
            Atom::TEXTAREA => {
                self.insert_html(tag);
                self.skip_newline = true;
                self.switch_tokenizer(State::Rcdata);
                self.original_mode = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
            }
            Atom::XMP => {
                self.close_p_in_button_scope();
                self.reconstruct_formatting();
                self.frameset_ok = false;
                self.generic_text_element(tag, State::Rawtext);
            }
            Atom::IFRAME => {
                self.frameset_ok = false;
                self.generic_text_element(tag, State::Rawtext);
            }
            Atom::NOEMBED => self.generic_text_element(tag, State::Rawtext),
            Atom::NOSCRIPT if self.scripting => self.generic_text_element(tag, State::Rawtext),
            Atom::SELECT => {
                if self
                    .context
                    .is_some_and(|(name, _)| name == html(Atom::SELECT))
                {
                    return;
                }
                if self.in_scope(Atom::SELECT, Scope::Default) {
                    self.pop_until(Atom::SELECT);
                    return;
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
                self.frameset_ok = false;
            }
            Atom::OPTION => {
                if self.in_scope(Atom::SELECT, Scope::Default) {
                    self.generate_implied_end_tags(Some(Atom::OPTGROUP));
                } else if self.current_is(Atom::OPTION) {
                    self.pop();
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
            }
            Atom::OPTGROUP => {
                if self.in_scope(Atom::SELECT, Scope::Default) {
                    self.generate_implied_end_tags(None);
                } else if self.current_is(Atom::OPTION) {
                    self.pop();
                }
                self.reconstruct_formatting();
                self.insert_html(tag);
            }
            Atom::RB | Atom::RTC => {
                if self.in_scope(Atom::RUBY, Scope::Default) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html(tag);
            }
            Atom::RP | Atom::RT => {
                if self.in_scope(Atom::RUBY, Scope::Default) {
                    self.generate_implied_end_tags(Some(Atom::RTC));
                }
                self.insert_html(tag);
            }
            Atom::MATH | Atom::SVG => {
                self.reconstruct_formatting();
                let ns = if tag.name == Atom::MATH {
                    adjust_mathml_attributes(tag);
                    Namespace::MathMl
                } else {
                    adjust_svg_attributes(tag);
                    Namespace::Svg
                };
                adjust_foreign_attributes(tag);
                self.insert_element(tag, ns);
                if tag.self_closing {
                    self.pop();
                }
            }
            Atom::CAPTION
            | Atom::COL
            | Atom::COLGROUP
            | Atom::FRAME
            | Atom::HEAD
            | Atom::TBODY
            | Atom::TD
            | Atom::TFOOT
            | Atom::TH
            | Atom::THEAD
            | Atom::TR => {}
            _ => {
                self.reconstruct_formatting();
                self.insert_html(tag);
            }
        }
    }

    fn close_html_element_in_scope(&mut self, local: Atom) {
        if !self.in_scope(local, Scope::Default) {
            return;
        }
        self.generate_implied_end_tags(None);
        self.pop_until(local);
    }

    fn in_body_end(&mut self, tag: &mut Tag) {
        match tag.name {
            Atom::TEMPLATE => self.in_head(&mut Tok::End(tag)),
            Atom::BODY | Atom::HTML => {
                if !self.in_scope(Atom::BODY, Scope::Default) {
                    return;
                }
                self.mode = Mode::AfterBody;
                if tag.name == Atom::HTML {
                    self.dispatch(&mut Tok::End(tag));
                }
            }
            Atom::ADDRESS
            | Atom::ARTICLE
            | Atom::ASIDE
            | Atom::BLOCKQUOTE
            | Atom::BUTTON
            | Atom::CENTER
            | Atom::DETAILS
            | Atom::DIALOG
            | Atom::DIR
            | Atom::DIV
            | Atom::DL
            | Atom::FIELDSET
            | Atom::FIGCAPTION
            | Atom::FIGURE
            | Atom::FOOTER
            | Atom::HEADER
            | Atom::HGROUP
            | Atom::LISTING
            | Atom::MAIN
            | Atom::MENU
            | Atom::NAV
            | Atom::OL
            | Atom::PRE
            | Atom::SEARCH
            | Atom::SECTION
            | Atom::SELECT
            | Atom::SUMMARY
            | Atom::UL => self.close_html_element_in_scope(tag.name),
            Atom::FORM => {
                if self.in_template_contents() {
                    self.close_html_element_in_scope(Atom::FORM);
                    return;
                }
                let node = self.form.take();
                let Some(node) = node else {
                    return;
                };
                if !self.in_scope_where(Scope::Default, |open| open.node == node) {
                    return;
                }
                self.generate_implied_end_tags(None);
                if let Some(i) = self.open_index(node) {
                    self.open.remove(i);
                }
            }
            Atom::P => {
                if !self.in_scope(Atom::P, Scope::Button) {
                    self.insert_implied(Atom::P);
                }
                self.close_p();
            }
            Atom::LI => {
                if self.in_scope(Atom::LI, Scope::ListItem) {
                    self.generate_implied_end_tags(Some(Atom::LI));
                    self.pop_until(Atom::LI);
                }
            }
            Atom::DD | Atom::DT => {
                if self.in_scope(tag.name, Scope::Default) {
                    self.generate_implied_end_tags(Some(tag.name));
                    self.pop_until(tag.name);
                }
            }
            Atom::H1 | Atom::H2 | Atom::H3 | Atom::H4 | Atom::H5 | Atom::H6 => {
                const HEADINGS: [Atom; 6] =
                    [Atom::H1, Atom::H2, Atom::H3, Atom::H4, Atom::H5, Atom::H6];
                if self.in_scope_where(Scope::Default, |open| {
                    open.name.ns == Namespace::Html && HEADINGS.contains(&open.name.local)
                }) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_one_of(&HEADINGS);
                }
            }
            Atom::A
            | Atom::B
            | Atom::BIG
            | Atom::CODE
            | Atom::EM
            | Atom::FONT
            | Atom::I
            | Atom::NOBR
            | Atom::S
            | Atom::SMALL
            | Atom::STRIKE
            | Atom::STRONG
            | Atom::TT
            | Atom::U => {
                self.adoption_agency(tag.name);
            }
            Atom::APPLET | Atom::MARQUEE | Atom::OBJECT => {
                if self.in_scope(tag.name, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(tag.name);
                    self.clear_formatting_to_marker();
                }
            }
            Atom::BR => {
                let mut br = Tag {
                    name: Atom::BR,
                    ..Tag::default()
                };
                self.in_body_start(&mut br);
            }
            _ => self.any_other_end_tag(tag.name),
        }
    }

    fn any_other_end_tag(&mut self, local: Atom) {
        for i in (0..self.open.len()).rev() {
            let Some(name) = self.open.get(i).map(|open| open.name) else {
                return;
            };
            if name == html(local) {
                self.generate_implied_end_tags(Some(local));
                self.open.truncate(i);
                return;
            }
            if special(name) {
                return;
            }
        }
    }

    fn adoption_agency(&mut self, subject: Atom) {
        if !self.adoption_agency_steps(subject) {
            self.any_other_end_tag(subject);
        }
    }

    fn adoption_agency_steps(&mut self, subject: Atom) -> bool {
        if let Some(current) = self.current()
            && current.name == html(subject)
            && self.formatting_index(current.node).is_none()
        {
            self.pop();
            return true;
        }
        for _ in 0..8 {
            let mut found = None;
            for (i, entry) in self.formatting.iter().enumerate().rev() {
                match entry {
                    Formatting::Marker => break,
                    Formatting::Element { node, tag } if tag.name == subject => {
                        found = Some((i, *node, tag.clone()));
                        break;
                    }
                    Formatting::Element { .. } => {}
                }
            }
            let Some((formatting_i, formatting_node, formatting_tag)) = found else {
                return false;
            };
            let Some(stack_i) = self.open_index(formatting_node) else {
                self.formatting.remove(formatting_i);
                return true;
            };
            if !self.in_scope_where(Scope::Default, |open| open.node == formatting_node) {
                return true;
            }
            let furthest = self
                .open
                .iter()
                .enumerate()
                .skip(stack_i + 1)
                .find(|(_, open)| special(open.name))
                .map(|(i, open)| (i, open.node));
            let Some((furthest_i, furthest_node)) = furthest else {
                self.open.truncate(stack_i);
                self.formatting.remove(formatting_i);
                return true;
            };
            let Some(common) = stack_i.checked_sub(1) else {
                return true;
            };
            let common_node = self.open.get(common).map(|open| open.node);
            let mut bookmark = formatting_i;
            let mut node_i = furthest_i;
            let mut last_node = furthest_node;
            let mut inner = 0;
            loop {
                inner += 1;
                node_i -= 1;
                let Some(node) = self.open.get(node_i).map(|open| open.node) else {
                    break;
                };
                if node == formatting_node {
                    break;
                }
                let mut entry = self.formatting_index(node);
                if inner > 3
                    && let Some(i) = entry
                {
                    self.formatting.remove(i);
                    if i < bookmark {
                        bookmark -= 1;
                    }
                    entry = None;
                }
                let Some(entry) = entry else {
                    self.open.remove(node_i);
                    continue;
                };
                let Some(Formatting::Element { tag, .. }) = self.formatting.get(entry) else {
                    break;
                };
                let tag = tag.clone();
                let new = self.sink.create_element(html(tag.name), tag.attributes());
                if let Some(open) = self.open.get_mut(node_i) {
                    open.node = new;
                }
                if last_node == furthest_node {
                    bookmark = entry + 1;
                }
                if let Some(slot) = self.formatting.get_mut(entry) {
                    *slot = Formatting::Element { node: new, tag };
                }
                self.sink.insert(new, last_node, None);
                last_node = new;
            }
            if let Some(common_i) = common_node.and_then(|node| self.open_index(node)) {
                match self.appropriate_place(common_i) {
                    Some((parent, before)) => self.sink.insert(parent, last_node, before),
                    None => self.sink.detach(last_node),
                }
            }
            let new = self
                .sink
                .create_element(html(formatting_tag.name), formatting_tag.attributes());
            self.sink.reparent_children(furthest_node, new);
            self.sink.insert(furthest_node, new, None);
            if let Some(i) = self.formatting_index(formatting_node) {
                self.formatting.remove(i);
                if i < bookmark {
                    bookmark -= 1;
                }
            }
            let bookmark = bookmark.min(self.formatting.len());
            self.formatting.insert(
                bookmark,
                Formatting::Element {
                    node: new,
                    tag: formatting_tag,
                },
            );
            if let Some(i) = self.open_index(formatting_node) {
                self.open.remove(i);
            }
            if let Some(i) = self.open_index(furthest_node) {
                self.open.insert(
                    i + 1,
                    Open {
                        node: new,
                        name: html(subject),
                        html_integration: false,
                    },
                );
            }
        }
        true
    }

    fn text(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) => self.insert_text(text),
            Tok::Eof => {
                self.pop();
                let mode = self.original_mode;
                self.reprocess(mode, tok);
            }
            Tok::End(_) => {
                self.pop();
                self.mode = self.original_mode;
            }
            _ => {}
        }
    }

    fn current_is_table_like(&self) -> bool {
        self.current().is_some_and(|open| {
            open.name.ns == Namespace::Html
                && matches!(
                    open.name.local,
                    Atom::TABLE
                        | Atom::TBODY
                        | Atom::TEMPLATE
                        | Atom::TFOOT
                        | Atom::THEAD
                        | Atom::TR
                )
        })
    }

    fn clear_to_context(&mut self, keep: &[Atom]) {
        while let Some(open) = self.current() {
            if open.name.ns == Namespace::Html
                && (keep.contains(&open.name.local)
                    || matches!(open.name.local, Atom::TEMPLATE | Atom::HTML))
            {
                break;
            }
            self.pop();
        }
    }

    fn in_table(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(_) if self.current_is_table_like() => {
                self.table_text.clear();
                self.table_text_has_content = false;
                self.original_mode = self.mode;
                self.reprocess(Mode::InTableText, tok);
            }
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) => match tag.name {
                Atom::CAPTION => {
                    self.clear_to_context(&[Atom::TABLE]);
                    self.formatting.push(Formatting::Marker);
                    self.insert_html(tag);
                    self.mode = Mode::InCaption;
                }
                Atom::COLGROUP => {
                    self.clear_to_context(&[Atom::TABLE]);
                    self.insert_html(tag);
                    self.mode = Mode::InColumnGroup;
                }
                Atom::COL => {
                    self.clear_to_context(&[Atom::TABLE]);
                    self.insert_implied(Atom::COLGROUP);
                    self.reprocess(Mode::InColumnGroup, tok);
                }
                Atom::TBODY | Atom::TFOOT | Atom::THEAD => {
                    self.clear_to_context(&[Atom::TABLE]);
                    self.insert_html(tag);
                    self.mode = Mode::InTableBody;
                }
                Atom::TD | Atom::TH | Atom::TR => {
                    self.clear_to_context(&[Atom::TABLE]);
                    self.insert_implied(Atom::TBODY);
                    self.reprocess(Mode::InTableBody, tok);
                }
                Atom::TABLE => {
                    if self.in_scope(Atom::TABLE, Scope::Table) {
                        self.pop_until(Atom::TABLE);
                        self.reset_mode();
                        self.dispatch(tok);
                    }
                }
                Atom::STYLE | Atom::SCRIPT | Atom::TEMPLATE => self.in_head(tok),
                Atom::INPUT
                    if tag
                        .attribute(Atom::TYPE)
                        .is_some_and(|value| value.eq_ignore_ascii_case("hidden")) =>
                {
                    self.insert_void(tag);
                }
                Atom::FORM => {
                    if self.form.is_some() && !self.in_template_contents() {
                        return;
                    }
                    let node = self.insert_html(tag);
                    if !self.in_template_contents() {
                        self.form = Some(node);
                    }
                    self.pop();
                }
                _ => self.foster_in_body(tok),
            },
            Tok::End(tag) => match tag.name {
                Atom::TABLE => {
                    if self.in_scope(Atom::TABLE, Scope::Table) {
                        self.pop_until(Atom::TABLE);
                        self.reset_mode();
                    }
                }
                Atom::BODY
                | Atom::CAPTION
                | Atom::COL
                | Atom::COLGROUP
                | Atom::HTML
                | Atom::TBODY
                | Atom::TD
                | Atom::TFOOT
                | Atom::TH
                | Atom::THEAD
                | Atom::TR => {}
                Atom::TEMPLATE => self.in_head(tok),
                _ => self.foster_in_body(tok),
            },
            Tok::Eof => self.in_body(tok),
            Tok::Chars(_) => self.foster_in_body(tok),
        }
    }

    fn foster_in_body(&mut self, tok: &mut Tok<'_>) {
        let previous = self.foster;
        self.foster = true;
        self.in_body(tok);
        self.foster = previous;
    }

    fn in_table_text(&mut self, tok: &mut Tok<'_>) {
        if let Tok::Chars(text) = tok {
            if text.starts_with('\0') {
                return;
            }
            self.table_text.push_str(text);
            if !all_space(text) {
                self.table_text_has_content = true;
            }
            return;
        }
        let pending = std::mem::take(&mut self.table_text);
        if self.table_text_has_content {
            let previous = self.foster;
            self.foster = true;
            self.in_body(&mut Tok::Chars(&pending));
            self.foster = previous;
        } else if !pending.is_empty() {
            self.insert_text(&pending);
        }
        self.table_text = pending;
        self.table_text.clear();
        let mode = self.original_mode;
        self.reprocess(mode, tok);
    }

    fn close_caption(&mut self) -> bool {
        if !self.in_scope(Atom::CAPTION, Scope::Table) {
            return false;
        }
        self.generate_implied_end_tags(None);
        self.pop_until(Atom::CAPTION);
        self.clear_formatting_to_marker();
        self.mode = Mode::InTable;
        true
    }

    fn in_caption(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::End(tag) if tag.name == Atom::CAPTION => {
                self.close_caption();
            }
            Tok::Start(tag)
                if matches!(
                    tag.name,
                    Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::TBODY
                        | Atom::TD
                        | Atom::TFOOT
                        | Atom::TH
                        | Atom::THEAD
                        | Atom::TR
                ) =>
            {
                if self.close_caption() {
                    self.dispatch(tok);
                }
            }
            Tok::End(tag) if tag.name == Atom::TABLE => {
                if self.close_caption() {
                    self.dispatch(tok);
                }
            }
            Tok::End(tag)
                if matches!(
                    tag.name,
                    Atom::BODY
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::HTML
                        | Atom::TBODY
                        | Atom::TD
                        | Atom::TFOOT
                        | Atom::TH
                        | Atom::THEAD
                        | Atom::TR
                ) => {}
            _ => self.in_body(tok),
        }
    }

    fn in_column_group(&mut self, tok: &mut Tok<'_>) {
        if let Some(space) = self.split_leading_space(tok) {
            if !space.is_empty() {
                self.insert_text(space);
            }
            if matches!(tok, Tok::Chars("")) {
                return;
            }
        }
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) => {}
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::Start(tag) if tag.name == Atom::COL => self.insert_void(tag),
            Tok::End(tag) if tag.name == Atom::COLGROUP => {
                if self.current_is(Atom::COLGROUP) {
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Tok::End(tag) if tag.name == Atom::COL => {}
            Tok::Start(tag) if tag.name == Atom::TEMPLATE => self.in_head(tok),
            Tok::End(tag) if tag.name == Atom::TEMPLATE => self.in_head(tok),
            Tok::Eof => self.in_body(tok),
            _ => {
                if self.current_is(Atom::COLGROUP) {
                    self.pop();
                    self.reprocess(Mode::InTable, tok);
                }
            }
        }
    }

    fn in_table_body(&mut self, tok: &mut Tok<'_>) {
        const SECTIONS: [Atom; 3] = [Atom::TBODY, Atom::THEAD, Atom::TFOOT];
        match tok {
            Tok::Start(tag) if tag.name == Atom::TR => {
                self.clear_to_context(&SECTIONS);
                self.insert_html(tag);
                self.mode = Mode::InRow;
            }
            Tok::Start(tag) if matches!(tag.name, Atom::TH | Atom::TD) => {
                self.clear_to_context(&SECTIONS);
                self.insert_implied(Atom::TR);
                self.reprocess(Mode::InRow, tok);
            }
            Tok::End(tag) if SECTIONS.contains(&tag.name) => {
                if self.in_scope(tag.name, Scope::Table) {
                    self.clear_to_context(&SECTIONS);
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Tok::Start(tag)
                if matches!(
                    tag.name,
                    Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::TBODY
                        | Atom::TFOOT
                        | Atom::THEAD
                ) =>
            {
                self.leave_table_body(tok)
            }
            Tok::End(tag) if tag.name == Atom::TABLE => self.leave_table_body(tok),
            Tok::End(tag)
                if matches!(
                    tag.name,
                    Atom::BODY
                        | Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::HTML
                        | Atom::TD
                        | Atom::TH
                        | Atom::TR
                ) => {}
            _ => self.in_table(tok),
        }
    }

    fn leave_table_body(&mut self, tok: &mut Tok<'_>) {
        if !self.in_scope_where(Scope::Table, |open| {
            open.name.ns == Namespace::Html
                && matches!(open.name.local, Atom::TBODY | Atom::THEAD | Atom::TFOOT)
        }) {
            return;
        }
        self.clear_to_context(&[Atom::TBODY, Atom::THEAD, Atom::TFOOT]);
        self.pop();
        self.reprocess(Mode::InTable, tok);
    }

    fn close_row(&mut self) -> bool {
        if !self.in_scope(Atom::TR, Scope::Table) {
            return false;
        }
        self.clear_to_context(&[Atom::TR]);
        self.pop();
        self.mode = Mode::InTableBody;
        true
    }

    fn in_row(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Start(tag) if matches!(tag.name, Atom::TH | Atom::TD) => {
                self.clear_to_context(&[Atom::TR]);
                self.insert_html(tag);
                self.mode = Mode::InCell;
                self.formatting.push(Formatting::Marker);
            }
            Tok::End(tag) if tag.name == Atom::TR => {
                self.close_row();
            }
            Tok::Start(tag)
                if matches!(
                    tag.name,
                    Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::TBODY
                        | Atom::TFOOT
                        | Atom::THEAD
                        | Atom::TR
                ) =>
            {
                if self.close_row() {
                    self.dispatch(tok);
                }
            }
            Tok::End(tag) if tag.name == Atom::TABLE => {
                if self.close_row() {
                    self.dispatch(tok);
                }
            }
            Tok::End(tag) if matches!(tag.name, Atom::TBODY | Atom::TFOOT | Atom::THEAD) => {
                if self.in_scope(tag.name, Scope::Table) && self.close_row() {
                    self.dispatch(tok);
                }
            }
            Tok::End(tag)
                if matches!(
                    tag.name,
                    Atom::BODY
                        | Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::HTML
                        | Atom::TD
                        | Atom::TH
                ) => {}
            _ => self.in_table(tok),
        }
    }

    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        self.pop_until_one_of(&[Atom::TD, Atom::TH]);
        self.clear_formatting_to_marker();
        self.mode = Mode::InRow;
    }

    fn in_cell(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::End(tag) if matches!(tag.name, Atom::TD | Atom::TH) => {
                if self.in_scope(tag.name, Scope::Table) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(tag.name);
                    self.clear_formatting_to_marker();
                    self.mode = Mode::InRow;
                }
            }
            Tok::Start(tag)
                if matches!(
                    tag.name,
                    Atom::CAPTION
                        | Atom::COL
                        | Atom::COLGROUP
                        | Atom::TBODY
                        | Atom::TD
                        | Atom::TFOOT
                        | Atom::TH
                        | Atom::THEAD
                        | Atom::TR
                ) =>
            {
                if self.in_scope(Atom::TD, Scope::Table) || self.in_scope(Atom::TH, Scope::Table) {
                    self.close_cell();
                    self.dispatch(tok);
                }
            }
            Tok::End(tag)
                if matches!(
                    tag.name,
                    Atom::BODY | Atom::CAPTION | Atom::COL | Atom::COLGROUP | Atom::HTML
                ) => {}
            Tok::End(tag)
                if matches!(
                    tag.name,
                    Atom::TABLE | Atom::TBODY | Atom::TFOOT | Atom::THEAD | Atom::TR
                ) =>
            {
                if self.in_scope(tag.name, Scope::Table) {
                    self.close_cell();
                    self.dispatch(tok);
                }
            }
            _ => self.in_body(tok),
        }
    }

    fn in_template(&mut self, tok: &mut Tok<'_>) {
        let switch_to = match tok {
            Tok::Chars(_) | Tok::Comment(_) | Tok::Pi(..) | Tok::Doctype(_) => {
                self.in_body(tok);
                return;
            }
            Tok::Start(tag) => match tag.name {
                Atom::BASE
                | Atom::BASEFONT
                | Atom::BGSOUND
                | Atom::LINK
                | Atom::META
                | Atom::NOFRAMES
                | Atom::SCRIPT
                | Atom::STYLE
                | Atom::TEMPLATE
                | Atom::TITLE => {
                    self.in_head(tok);
                    return;
                }
                Atom::CAPTION | Atom::COLGROUP | Atom::TBODY | Atom::TFOOT | Atom::THEAD => {
                    Mode::InTable
                }
                Atom::COL => Mode::InColumnGroup,
                Atom::TR => Mode::InTableBody,
                Atom::TD | Atom::TH => Mode::InRow,
                _ => Mode::InBody,
            },
            Tok::End(tag) => {
                if tag.name == Atom::TEMPLATE {
                    self.in_head(tok);
                }
                return;
            }
            Tok::Eof => {
                if !self.has_open(Atom::TEMPLATE) {
                    return;
                }
                self.pop_until(Atom::TEMPLATE);
                self.clear_formatting_to_marker();
                self.template_modes.pop();
                self.reset_mode();
                self.dispatch(tok);
                return;
            }
        };
        self.template_modes.pop();
        self.template_modes.push(switch_to);
        self.reprocess(switch_to, tok);
    }

    fn after_body(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) if all_space(text) => self.in_body(tok),
            Tok::Comment(_) | Tok::Pi(..) => {
                let root = self.open.first().map(|open| open.node);
                self.insert_comment_like(tok, root);
            }
            Tok::Doctype(_) => {}
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::End(tag) if tag.name == Atom::HTML => {
                if self.context.is_none() {
                    self.mode = Mode::AfterAfterBody;
                }
            }
            Tok::Eof => {}
            _ => self.reprocess(Mode::InBody, tok),
        }
    }

    fn frameset(&mut self, mode: Mode, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) => {
                let spaces: String = text.chars().filter(|&c| is_space(c)).collect();
                if !spaces.is_empty() {
                    self.insert_text(&spaces);
                }
            }
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Start(tag) => match tag.name {
                Atom::HTML => self.in_body(tok),
                Atom::FRAMESET if mode == Mode::InFrameset => {
                    self.insert_html(tag);
                }
                Atom::FRAME if mode == Mode::InFrameset => self.insert_void(tag),
                Atom::NOFRAMES => self.in_head(tok),
                _ => {}
            },
            Tok::End(tag) if mode == Mode::InFrameset && tag.name == Atom::FRAMESET => {
                if self.open.len() <= 1 {
                    return;
                }
                self.pop();
                if self.context.is_none() && !self.current_is(Atom::FRAMESET) {
                    self.mode = Mode::AfterFrameset;
                }
            }
            Tok::End(tag) if mode == Mode::AfterFrameset && tag.name == Atom::HTML => {
                self.mode = Mode::AfterAfterFrameset;
            }
            _ => {}
        }
    }

    fn after_after_body(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => {
                let document = self.sink.document();
                self.insert_comment_like(tok, Some(document));
            }
            Tok::Doctype(_) => self.in_body(tok),
            Tok::Chars(text) if all_space(text) => self.in_body(tok),
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::Eof => {}
            _ => self.reprocess(Mode::InBody, tok),
        }
    }

    fn after_after_frameset(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Comment(_) | Tok::Pi(..) => {
                let document = self.sink.document();
                self.insert_comment_like(tok, Some(document));
            }
            Tok::Doctype(_) => self.in_body(tok),
            Tok::Chars(_) => self.frameset(Mode::AfterFrameset, tok),
            Tok::Start(tag) if tag.name == Atom::HTML => self.in_body(tok),
            Tok::Start(tag) if tag.name == Atom::NOFRAMES => self.in_head(tok),
            _ => {}
        }
    }

    fn foreign(&mut self, tok: &mut Tok<'_>) {
        match tok {
            Tok::Chars(text) => {
                if text.starts_with('\0') {
                    let replacement = "\u{FFFD}".repeat(text.len());
                    self.insert_text(&replacement);
                    return;
                }
                self.insert_text(text);
                if !all_space(text) {
                    self.frameset_ok = false;
                }
            }
            Tok::Comment(_) | Tok::Pi(..) => self.insert_comment_like(tok, None),
            Tok::Doctype(_) | Tok::Eof => {}
            Tok::Start(tag) if breaks_out_of_foreign(tag) => self.break_out_of_foreign(tok),
            Tok::End(tag) if matches!(tag.name, Atom::BR | Atom::P) => {
                self.break_out_of_foreign(tok)
            }
            Tok::Start(tag) => {
                let Some((adjusted, _)) = self.adjusted_current() else {
                    return;
                };
                match adjusted.ns {
                    Namespace::MathMl => adjust_mathml_attributes(tag),
                    Namespace::Svg => {
                        tag.name = svg_tag_name(tag.name);
                        adjust_svg_attributes(tag);
                    }
                    _ => {}
                }
                adjust_foreign_attributes(tag);
                self.insert_element(tag, adjusted.ns);
                if tag.self_closing {
                    self.pop();
                }
            }
            Tok::End(tag) => {
                if tag.name == Atom::SCRIPT
                    && self.current().is_some_and(|open| {
                        open.name.ns == Namespace::Svg && open.name.local == Atom::SCRIPT
                    })
                {
                    self.pop();
                    return;
                }
                let name = tag.name;
                for i in (0..self.open.len()).rev() {
                    let Some(open) = self.open.get(i).copied() else {
                        return;
                    };
                    if i == 0 {
                        return;
                    }
                    if same_name_ignoring_case(open.name.local, name) {
                        self.open.truncate(i);
                        return;
                    }
                    if self
                        .open
                        .get(i - 1)
                        .is_some_and(|o| o.name.ns == Namespace::Html)
                    {
                        let mode = self.mode;
                        self.html(mode, tok);
                        return;
                    }
                }
            }
        }
    }

    fn break_out_of_foreign(&mut self, tok: &mut Tok<'_>) {
        while let Some(open) = self.current() {
            if open.name.ns == Namespace::Html
                || mathml_text_integration(open.name)
                || open.html_integration
            {
                break;
            }
            self.pop();
        }
        let mode = self.mode;
        self.html(mode, tok);
    }
}

fn same_name_ignoring_case(a: Atom, b: Atom) -> bool {
    a == b
        || matches!((a.static_name(), b.static_name()), (Some(x), Some(y)) if x.eq_ignore_ascii_case(y))
}

fn breaks_out_of_foreign(tag: &Tag) -> bool {
    match tag.name {
        Atom::B
        | Atom::BIG
        | Atom::BLOCKQUOTE
        | Atom::BODY
        | Atom::BR
        | Atom::CENTER
        | Atom::CODE
        | Atom::DD
        | Atom::DIV
        | Atom::DL
        | Atom::DT
        | Atom::EM
        | Atom::EMBED
        | Atom::H1
        | Atom::H2
        | Atom::H3
        | Atom::H4
        | Atom::H5
        | Atom::H6
        | Atom::HEAD
        | Atom::HR
        | Atom::I
        | Atom::IMG
        | Atom::LI
        | Atom::LISTING
        | Atom::MENU
        | Atom::META
        | Atom::NOBR
        | Atom::OL
        | Atom::P
        | Atom::PRE
        | Atom::RUBY
        | Atom::S
        | Atom::SMALL
        | Atom::SPAN
        | Atom::STRONG
        | Atom::STRIKE
        | Atom::SUB
        | Atom::SUP
        | Atom::TABLE
        | Atom::TT
        | Atom::U
        | Atom::UL
        | Atom::VAR => true,
        Atom::FONT => [Atom::COLOR, Atom::FACE, Atom::SIZE]
            .into_iter()
            .any(|name| tag.attribute(name).is_some()),
        _ => false,
    }
}
