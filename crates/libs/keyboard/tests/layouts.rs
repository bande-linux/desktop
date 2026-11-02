#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_keyboard::Layouts;

const RULES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xkbConfigRegistry SYSTEM "xkb.dtd">
<xkbConfigRegistry version="1.1">
  <layoutList>
    <layout>
      <configItem>
        <name>se</name>
        <!-- Keyboard indicator for Swedish layouts -->
        <shortDescription>sv</shortDescription>
        <description>Swedish</description>
        <countryList><iso3166Id>SE</iso3166Id></countryList>
      </configItem>
      <variantList>
        <variant>
          <configItem>
            <name>mac</name>
            <description>Swedish (Macintosh)</description>
          </configItem>
        </variant>
      </variantList>
    </layout>
  </layoutList>
</xkbConfigRegistry>
"#;

#[test]
fn the_xkb_rules_are_read_despite_their_doctype() {
    let layouts = Layouts::parse(RULES);
    let codes: Vec<&str> = layouts
        .all()
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(codes, ["se", "se(mac)"]);
    assert_eq!(layouts.name("se(mac)"), "Swedish (Macintosh)");
    assert_eq!(layouts.short("se(mac)"), "SV");
    let found: Vec<&str> = layouts
        .search("mac swedish")
        .iter()
        .map(|layout| layout.code.as_str())
        .collect();
    assert_eq!(found, ["se(mac)"]);
}
