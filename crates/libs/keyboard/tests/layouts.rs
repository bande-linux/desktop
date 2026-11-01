#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_keyboard::{Area, Layouts, SHORTCUTS, shortcuts_in};

const RULES: &str = r#"<?xml version="1.0"?>
<xkbConfigRegistry version="1.1">
  <layoutList>
    <layout>
      <configItem>
        <name>se</name>
        <!-- Keyboard indicator for Swedish layouts -->
        <shortDescription>sv</shortDescription>
        <description>Swedish</description>
        <countryList>
          <iso3166Id>SE</iso3166Id>
        </countryList>
        <languageList>
          <iso639Id>swe</iso639Id>
        </languageList>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>dvorak</name>
            <description>Swedish (Dvorak)</description>
          </configItem>
        </variant>
        <variant>
          <configItem>
            <name>mac</name>
            <description>Swedish (Macintosh)</description>
          </configItem>
        </variant>
        <variant>
          <configItem>
            <name>smi</name>
            <description>Northern Saami (Sweden)</description>
            <languageList>
              <iso639Id>sme</iso639Id>
            </languageList>
          </configItem>
        </variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>de</name>
        <shortDescription>de</shortDescription>
        <description>German</description>
        <countryList>
          <iso3166Id>DE</iso3166Id>
        </countryList>
        <languageList>
          <iso639Id>ger</iso639Id>
        </languageList>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>nodeadkeys</name>
            <shortDescription>dn</shortDescription>
            <description>German (no dead keys)</description>
          </configItem>
        </variant>
        <variant>
          <configItem>
            <name>mac</name>
            <description>German (Macintosh)</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>custom</name>
        <shortDescription>custom</shortDescription>
        <description>A user-defined custom Layout</description>
        <languageList>
          <iso639Id>und</iso639Id>
        </languageList>
      </configItem>
      <variantList/>
    </layout>
  </layoutList>
</xkbConfigRegistry>
"#;

fn found<'a>(layouts: &'a Layouts, query: &str) -> Vec<&'a str> {
    layouts
        .search(query)
        .iter()
        .map(|layout| layout.code.as_str())
        .collect()
}

#[test]
fn layouts_and_variants_get_readable_names_and_short_labels_sorted_by_name_without_custom() {
    let layouts = Layouts::parse(RULES);
    let codes: Vec<&str> = layouts
        .all()
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "de",
            "de(mac)",
            "de(nodeadkeys)",
            "se(smi)",
            "se",
            "se(dvorak)",
            "se(mac)"
        ]
    );
    assert_eq!(layouts.name("se(dvorak)"), "Swedish (Dvorak)");
    assert_eq!(layouts.short("se(dvorak)"), "SV");
    assert_eq!(layouts.short("de(nodeadkeys)"), "DN");
    assert_eq!(layouts.name("fi"), "fi");
    assert_eq!(layouts.short("fi(mac)"), "FI");
}

#[test]
fn search_matches_every_word_against_names_codes_countries_and_languages() {
    let layouts = Layouts::parse(RULES);
    assert_eq!(found(&layouts, "swedish mac"), ["se(mac)"]);
    assert_eq!(found(&layouts, "mac swedish"), ["se(mac)"]);
    assert_eq!(found(&layouts, "Ger"), ["de", "de(mac)", "de(nodeadkeys)"]);
    assert_eq!(found(&layouts, "dvo"), ["se(dvorak)"]);
    assert_eq!(found(&layouts, "se").first(), Some(&"se"));
    assert!(found(&layouts, "se").contains(&"se(mac)"));
    assert!(found(&layouts, "swe").contains(&"se(dvorak)"));
    assert_eq!(found(&layouts, "sweden saami"), ["se(smi)"]);
    assert!(found(&layouts, "custom").is_empty());
    assert!(found(&layouts, "  ").is_empty());
}

#[test]
fn suggestions_for_a_country_start_with_its_base_layout() {
    let layouts = Layouts::parse(RULES);
    let suggested: Vec<&str> = layouts
        .suggested("SE")
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(suggested, ["se", "se(smi)", "se(dvorak)", "se(mac)"]);
    assert!(layouts.suggested("FI").is_empty());
}

#[test]
fn every_shortcut_belongs_to_exactly_one_listed_area_and_keys_are_unique() {
    let listed: usize = Area::ALL
        .iter()
        .map(|area| shortcuts_in(*area).count())
        .sum();
    assert_eq!(listed, SHORTCUTS.len());
    for (index, shortcut) in SHORTCUTS.iter().enumerate() {
        assert!(
            SHORTCUTS[index + 1..]
                .iter()
                .all(|other| other.keys != shortcut.keys),
            "{} is listed twice",
            shortcut.keys
        );
    }
}
