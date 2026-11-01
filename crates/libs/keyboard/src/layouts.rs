use roxmltree::{Document, Node};

pub const EVDEV_RULES: &str = "/usr/share/X11/xkb/rules/evdev.xml";
const CUSTOM: &str = "custom";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub code: String,
    pub name: String,
    pub short: String,
    pub countries: Vec<String>,
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layouts(Vec<Layout>);

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

fn text<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name)?.text().map(str::trim)
}

fn codes(config: Node<'_, '_>, list: &str, item: &str) -> Vec<String> {
    child(config, list)
        .into_iter()
        .flat_map(|list| list.children().filter(|node| node.has_tag_name(item)))
        .filter_map(|node| node.text())
        .map(|code| code.trim().to_lowercase())
        .collect()
}

fn or_inherited(own: Vec<String>, inherited: &[String]) -> Vec<String> {
    if own.is_empty() {
        inherited.to_vec()
    } else {
        own
    }
}

fn layout(config: Node<'_, '_>, code: String, base: &Layout) -> Option<Layout> {
    Some(Layout {
        code,
        name: text(config, "description")?.to_owned(),
        short: text(config, "shortDescription")
            .unwrap_or(&base.short)
            .to_uppercase(),
        countries: or_inherited(codes(config, "countryList", "iso3166Id"), &base.countries),
        languages: or_inherited(codes(config, "languageList", "iso639Id"), &base.languages),
    })
}

impl Layout {
    fn matches(&self, word: &str) -> bool {
        self.name.to_lowercase().contains(word)
            || self.code == word
            || self.countries.iter().any(|country| country == word)
            || self.languages.iter().any(|language| language == word)
    }

    fn is_variant(&self) -> bool {
        self.code.contains('(')
    }
}

impl Layouts {
    pub fn parse(xml: &str) -> Self {
        let Ok(document) = Document::parse(xml) else {
            return Self::default();
        };
        let mut layouts = Vec::new();
        for node in document
            .descendants()
            .filter(|node| node.has_tag_name("layout"))
        {
            let Some(config) = child(node, "configItem") else {
                continue;
            };
            let Some(name) = text(config, "name").filter(|name| *name != CUSTOM) else {
                continue;
            };
            let root = Layout {
                code: name.to_owned(),
                name: String::new(),
                short: name.to_owned(),
                countries: Vec::new(),
                languages: Vec::new(),
            };
            let Some(base) = layout(config, name.to_owned(), &root) else {
                continue;
            };
            let variants = child(node, "variantList")
                .into_iter()
                .flat_map(|list| list.children().filter(|node| node.has_tag_name("variant")))
                .filter_map(|variant| child(variant, "configItem"));
            for variant in variants {
                if let Some(variant_name) = text(variant, "name") {
                    layouts.extend(layout(variant, format!("{name}({variant_name})"), &base));
                }
            }
            layouts.push(base);
        }
        layouts.sort_by_cached_key(|layout| layout.name.to_lowercase());
        Self(layouts)
    }

    pub fn load() -> Self {
        std::fs::read_to_string(EVDEV_RULES)
            .map(|xml| Self::parse(&xml))
            .unwrap_or_default()
    }

    pub fn all(&self) -> &[Layout] {
        &self.0
    }

    pub fn find(&self, code: &str) -> Option<&Layout> {
        self.0.iter().find(|layout| layout.code == code)
    }

    pub fn name(&self, code: &str) -> String {
        self.find(code)
            .map_or_else(|| code.to_owned(), |layout| layout.name.clone())
    }

    pub fn short(&self, code: &str) -> String {
        self.find(code).map_or_else(
            || {
                code.split('(')
                    .next()
                    .unwrap_or(code)
                    .chars()
                    .take(3)
                    .collect::<String>()
                    .to_uppercase()
            },
            |layout| layout.short.clone(),
        )
    }

    pub fn search(&self, query: &str) -> Vec<&Layout> {
        let query = query.trim().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }
        let (mut starting, containing): (Vec<&Layout>, Vec<&Layout>) = self
            .0
            .iter()
            .filter(|layout| words.iter().all(|word| layout.matches(word)))
            .partition(|layout| {
                layout.code == query || layout.name.to_lowercase().starts_with(&query)
            });
        starting.extend(containing);
        starting
    }

    pub fn suggested(&self, country: &str) -> Vec<&Layout> {
        let country = country.to_lowercase();
        let mut found: Vec<&Layout> = self
            .0
            .iter()
            .filter(|layout| layout.countries.contains(&country))
            .collect();
        found.sort_by_key(|layout| layout.is_variant());
        found
    }
}
