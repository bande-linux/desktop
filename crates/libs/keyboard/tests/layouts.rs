#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_keyboard::{Area, Layouts, SHORTCUTS, shortcuts_in};

const RULES: &str = r#"<?xml version="1.0"?>
<xkbConfigRegistry version="1.1">
  <layoutList>
    <layout>
      <configItem>
        <name>se</name>
        <!-- Keyboard indicator for Swedish layouts -->
        <shortDescription>sv</shortDescription>
        <description>Swedish</description>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>dvorak</name>
            <description>Swedish (Dvorak)</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>de</name>
        <shortDescription>de</shortDescription>
        <description>German</description>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>nodeadkeys</name>
            <shortDescription>dn</shortDescription>
            <description>German (no dead keys)</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
  </layoutList>
</xkbConfigRegistry>
"#;

#[test]
fn layouts_and_variants_get_readable_names_and_short_labels_sorted_by_name() {
    let layouts = Layouts::parse(RULES);
    let codes: Vec<&str> = layouts
        .all()
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(codes, ["de", "de(nodeadkeys)", "se", "se(dvorak)"]);
    assert_eq!(layouts.name("se(dvorak)"), "Swedish (Dvorak)");
    assert_eq!(layouts.short("se(dvorak)"), "SV");
    assert_eq!(layouts.short("de(nodeadkeys)"), "DN");
    assert_eq!(layouts.name("fi"), "fi");
    assert_eq!(layouts.short("fi(mac)"), "FI");
}

#[test]
fn search_puts_names_starting_with_the_query_first() {
    let layouts = Layouts::parse(RULES);
    let found: Vec<&str> = layouts
        .search("dvo")
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(found, ["se(dvorak)"]);
    let found: Vec<&str> = layouts
        .search("Ger")
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(found, ["de", "de(nodeadkeys)"]);
    assert_eq!(
        layouts
            .search("se")
            .first()
            .map(|layout| layout.code.as_str()),
        Some("se")
    );
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
