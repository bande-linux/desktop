use std::fmt::Write;
use std::path::PathBuf;

include!("properties.rs");

fn camel(name: &str) -> String {
    name.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn snake(name: &str) -> String {
    name.trim_start_matches('-').replace('-', "_")
}

fn longhand_variant(name: &str) -> String {
    camel(name)
}

fn generate() -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    let o = &mut out;

    writeln!(
        o,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]"
    )?;
    writeln!(o, "#[repr(u16)]\npub enum LonghandId {{")?;
    for longhand in LONGHANDS {
        writeln!(o, "    {},", longhand_variant(longhand.name))?;
    }
    writeln!(o, "}}\n")?;
    writeln!(o, "impl LonghandId {{")?;
    writeln!(o, "    pub const COUNT: usize = {};", LONGHANDS.len())?;
    write!(
        o,
        "    pub const ALL: [LonghandId; {}] = [",
        LONGHANDS.len()
    )?;
    for longhand in LONGHANDS {
        write!(o, "LonghandId::{}, ", longhand_variant(longhand.name))?;
    }
    writeln!(o, "];\n")?;
    for (function, ty) in [
        ("name", "&'static str"),
        ("initial_css", "&'static str"),
        ("inherited", "bool"),
    ] {
        writeln!(
            o,
            "    pub fn {function}(self) -> {ty} {{\n        match self {{"
        )?;
        for longhand in LONGHANDS {
            let value = match function {
                "name" => format!("{:?}", longhand.name),
                "initial_css" => format!("{:?}", longhand.initial),
                _ => longhand.inherited.to_string(),
            };
            writeln!(
                o,
                "            LonghandId::{} => {value},",
                longhand_variant(longhand.name)
            )?;
        }
        writeln!(o, "        }}\n    }}\n")?;
    }
    writeln!(o, "}}\n")?;

    writeln!(
        o,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum ShorthandId {{"
    )?;
    for shorthand in SHORTHANDS {
        writeln!(o, "    {},", camel(shorthand.name))?;
    }
    writeln!(o, "}}\n\nimpl ShorthandId {{")?;
    writeln!(
        o,
        "    pub fn name(self) -> &'static str {{\n        match self {{"
    )?;
    for shorthand in SHORTHANDS {
        writeln!(
            o,
            "            ShorthandId::{} => {:?},",
            camel(shorthand.name),
            shorthand.name
        )?;
    }
    writeln!(o, "        }}\n    }}\n")?;
    writeln!(
        o,
        "    pub fn longhands(self) -> &'static [LonghandId] {{\n        match self {{"
    )?;
    for shorthand in SHORTHANDS {
        let ids: Vec<String> = shorthand
            .longhands
            .iter()
            .map(|name| format!("LonghandId::{}", longhand_variant(name)))
            .collect();
        writeln!(
            o,
            "            ShorthandId::{} => &[{}],",
            camel(shorthand.name),
            ids.join(", ")
        )?;
    }
    writeln!(o, "        }}\n    }}\n}}\n")?;

    writeln!(o, "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]")?;
    writeln!(
        o,
        "pub enum PropertyId {{\n    Longhand(LonghandId),\n    Shorthand(ShorthandId),\n}}\n"
    )?;
    writeln!(
        o,
        "impl PropertyId {{\n    fn from_lowercase(name: &str) -> Option<PropertyId> {{"
    )?;
    writeln!(o, "        Some(match name {{")?;
    for longhand in LONGHANDS {
        writeln!(
            o,
            "            {:?} => PropertyId::Longhand(LonghandId::{}),",
            longhand.name,
            longhand_variant(longhand.name)
        )?;
    }
    for shorthand in SHORTHANDS {
        writeln!(
            o,
            "            {:?} => PropertyId::Shorthand(ShorthandId::{}),",
            shorthand.name,
            camel(shorthand.name)
        )?;
    }
    for (alias, target) in ALIASES {
        let target_id = if LONGHANDS.iter().any(|l| l.name == *target) {
            format!(
                "PropertyId::Longhand(LonghandId::{})",
                longhand_variant(target)
            )
        } else {
            format!("PropertyId::Shorthand(ShorthandId::{})", camel(target))
        };
        writeln!(o, "            {alias:?} => {target_id},")?;
    }
    writeln!(
        o,
        "            _ => return None,\n        }})\n    }}\n}}\n"
    )?;

    for keyword_type in KEYWORD_TYPES {
        let name = keyword_type.name;
        writeln!(
            o,
            "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum {name} {{"
        )?;
        for keyword in keyword_type.keywords {
            writeln!(o, "    {},", camel(keyword))?;
        }
        writeln!(o, "}}\n\nimpl {name} {{")?;
        writeln!(o, "    pub fn from_ident(ident: &str) -> Option<{name}> {{")?;
        for keyword in keyword_type.keywords {
            writeln!(
                o,
                "        if ident.eq_ignore_ascii_case({keyword:?}) {{ return Some({name}::{}); }}",
                camel(keyword)
            )?;
        }
        writeln!(o, "        None\n    }}\n")?;
        writeln!(
            o,
            "    pub fn name(self) -> &'static str {{\n        match self {{"
        )?;
        for keyword in keyword_type.keywords {
            writeln!(o, "            {name}::{} => {keyword:?},", camel(keyword))?;
        }
        writeln!(o, "        }}\n    }}\n}}\n")?;
        writeln!(o, "impl Parse for {name} {{")?;
        writeln!(
            o,
            "    fn parse(input: &mut Parser<'_, '_>) -> ParseResult<Self> {{"
        )?;
        writeln!(
            o,
            "        {name}::from_ident(&input.expect_ident()?).ok_or(ParseError::Invalid)"
        )?;
        writeln!(o, "    }}\n}}\n")?;
        writeln!(o, "impl fmt::Display for {name} {{")?;
        writeln!(
            o,
            "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{ f.write_str(self.name()) }}"
        )?;
        writeln!(o, "}}\n")?;
    }

    let mut types: Vec<&str> = LONGHANDS.iter().map(|l| l.value).collect();
    types.sort_unstable();
    types.dedup();
    writeln!(
        o,
        "#[derive(Clone, Debug, PartialEq)]\npub enum PropertyValue {{"
    )?;
    for ty in &types {
        writeln!(o, "    {ty}({ty}),")?;
    }
    writeln!(o, "}}\n\nimpl fmt::Display for PropertyValue {{")?;
    writeln!(
        o,
        "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{\n        match self {{"
    )?;
    for ty in &types {
        writeln!(o, "            PropertyValue::{ty}(value) => value.fmt(f),")?;
    }
    writeln!(o, "        }}\n    }}\n}}\n")?;

    writeln!(
        o,
        "pub fn parse_longhand(id: LonghandId, input: &mut Parser<'_, '_>) -> ParseResult<PropertyValue> {{"
    )?;
    writeln!(o, "    match id {{")?;
    for longhand in LONGHANDS {
        writeln!(
            o,
            "        LonghandId::{} => <{ty} as Parse>::parse(input).map(PropertyValue::{ty}),",
            longhand_variant(longhand.name),
            ty = longhand.value
        )?;
    }
    writeln!(o, "    }}\n}}\n")?;

    writeln!(
        o,
        "pub fn parse_shorthand(id: ShorthandId, input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {{"
    )?;
    writeln!(o, "    let ids = id.longhands();\n    match id {{")?;
    for shorthand in SHORTHANDS {
        let call = match shorthand.kind {
            ShorthandKind::FourSides => "shorthands::four_sides(input, ids, out)".to_owned(),
            ShorthandKind::Pair => "shorthands::pair(input, ids, out)".to_owned(),
            ShorthandKind::AnyOrder => "shorthands::any_order(input, ids, out)".to_owned(),
            ShorthandKind::Custom => format!("shorthands::{}(input, out)", snake(shorthand.name)),
        };
        writeln!(
            o,
            "        ShorthandId::{} => {call},",
            camel(shorthand.name)
        )?;
    }
    writeln!(o, "    }}\n}}")?;
    Ok(out)
}

fn main() {
    println!("cargo::rerun-if-changed=properties.rs");
    let Some(dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    match generate() {
        Ok(code) => {
            if let Err(error) = std::fs::write(dir.join("properties.rs"), code) {
                println!("cargo::error=writing properties.rs: {error}");
            }
        }
        Err(error) => println!("cargo::error=generating properties.rs: {error}"),
    }
}
