#![allow(clippy::unwrap_used)]

use bande_address_book::vcard::{Birthday, Kind, Label, Labelled, Version, parse_cards};

const CARDS: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:3.0\r\n",
    "UID:ann\r\n",
    "FN:Ann Lindqvist\r\n",
    "N:Lindqvist;Ann;;Dr.\\, PhD;\r\n",
    "ORG:Fjord\\; Partners;Research\r\n",
    "item1.EMAIL;TYPE=INTERNET:ann@fjord.example\r\n",
    "item1.X-ABLabel:Lab\r\n",
    "EMAIL;TYPE=work,pref:ann.lindqvist@fjord.example\r\n",
    "TEL;TYPE=CELL:+46 70 123 45 67\r\n",
    "NOTE:Met at the con\r\n",
    " ference\\, room 4\\nBring the bl\u{e5}\r\n",
    " b\u{e4}r jam\r\n",
    "X-SOCIAL;SERVICE=fediverse:@ann@fjord.example\r\n",
    "BDAY;X-APPLE-OMIT-YEAR=1604:1604-05-12\r\n",
    "END:VCARD\r\n",
    "BEGIN:VCARD\r\n",
    "VERSION:2.1\r\n",
    "N;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:J=F8rgensen;Bj=\r\n",
    "=F8rn\r\n",
    "TEL;WORK;VOICE:+45 33 12 34 56\r\n",
    "ADR;HOME;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:;;Gade 1;K=C3=B8benhavn;;1000;Danmark\r\n",
    "END:VCARD\r\n",
    "BEGIN:VCARD\r\n",
    "VERSION:4.0\r\n",
    "this line has no colon\r\n",
    "END:VCARD\r\n",
    "BEGIN:VCARD\r\n",
    "VERSION:4.0\r\n",
    "UID:urn:uuid:team\r\n",
    "KIND:group\r\n",
    "FN:Team\r\n",
    "MEMBER:urn:uuid:ann\r\n",
    "BDAY:--0229\r\n",
    "PHOTO:data:image/png;base64,iVBORw0K\r\n",
    " GgoAAAANSUhEUg==\r\n",
    "END:VCARD\r\n",
);

#[test]
fn tricky_vcards_are_read_kept_whole_and_written_back_with_only_the_edit_changed() {
    let parsed = parse_cards(CARDS);
    assert_eq!(
        parsed.skipped, 1,
        "the card with a broken line is skipped alone"
    );
    let [ann, bjorn, team] = &parsed.cards[..] else {
        panic!("{} cards", parsed.cards.len());
    };

    let contact = ann.contact();
    assert_eq!(contact.name.family, "Lindqvist");
    assert_eq!(contact.name.prefix, "Dr., PhD");
    assert_eq!(contact.organisation, "Fjord; Partners, Research");
    assert_eq!(
        contact.note,
        "Met at the conference, room 4\nBring the blåbär jam"
    );
    assert_eq!(contact.emails[0].value, "ann.lindqvist@fjord.example");
    assert_eq!(contact.emails[0].label, Label::Work);
    assert_eq!(contact.phones[0].label, Label::Mobile);
    assert_eq!(
        contact.birthday,
        Some(Birthday {
            year: None,
            month: 5,
            day: 12
        })
    );

    let bjorn = bjorn.contact();
    assert_eq!(bjorn.name.family, "Jørgensen");
    assert_eq!(bjorn.name.given, "Bjørn");
    assert_eq!(bjorn.phones[0].label, Label::Work);
    assert_eq!(bjorn.addresses[0].locality, "København");

    assert_eq!(team.kind(), Kind::Group);
    assert_eq!(team.version(), Version::V4);
    assert_eq!(team.uid(), "team");
    assert_eq!(team.members(), ["ann"]);
    let team = team.contact();
    assert_eq!(team.birthday.map(|day| (day.month, day.day)), Some((2, 29)));
    assert_eq!(team.photo.unwrap().bytes[..4], *b"\x89PNG");

    let mut edited = ann.clone();
    let mut changed = contact.clone();
    changed.phones.push(Labelled {
        label: Label::Home,
        value: "+46 8 765 43 21".to_owned(),
    });
    edited.apply(&contact, &changed);
    let written = edited.to_vcf();
    for kept in [
        "item1.EMAIL;TYPE=INTERNET:ann@fjord.example\r\n",
        "item1.X-ABLABEL:Lab\r\n",
        "X-SOCIAL;SERVICE=fediverse:@ann@fjord.example\r\n",
        "N:Lindqvist;Ann;;Dr.\\, PhD;\r\n",
    ] {
        assert!(written.contains(kept), "{kept:?} is lost from {written}");
    }
    assert!(written.lines().all(|line| line.len() <= 75));
    let again = parse_cards(&written);
    assert_eq!(again.cards[0].contact(), changed);
}
