use unbound_widgets::{Key, Modifiers, NameEditor};

#[test]
fn renaming_starts_with_the_name_selected_without_its_extension() {
    let mut editor = NameEditor::new("report.final.pdf", false);
    assert_eq!(editor.selection(), 0..12);

    editor.insert("summary");
    assert_eq!(editor.text(), "summary.pdf");
    editor.key(Key::Backspace, Modifiers::default());
    editor.key(Key::Right, Modifiers::default());
    editor.key(
        Key::Left,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    editor.key(Key::Character('é'), Modifiers::default());
    assert_eq!(editor.text(), "summarépdf");
    assert_eq!(NameEditor::new("photos.2024", true).selection(), 0..11);
    assert_eq!(NameEditor::new(".bashrc", false).selection(), 0..7);
}
