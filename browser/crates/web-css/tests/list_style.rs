use web_atoms::AtomTable;
use web_css::parse_style_attribute;
use web_css::properties::LonghandId;

#[test]
fn lone_none_sets_both_image_and_type() {
    let mut atoms = AtomTable::new();
    for css in ["list-style: none", "list-style: none outside"] {
        let block = parse_style_attribute(css, &mut atoms);
        for id in [LonghandId::ListStyleImage, LonghandId::ListStyleType] {
            let declaration = block.declarations.iter().find(|d| d.id == id);
            assert_eq!(
                declaration.map(|d| d.value.to_string()).as_deref(),
                Some("none"),
                "{css}"
            );
        }
    }
}
