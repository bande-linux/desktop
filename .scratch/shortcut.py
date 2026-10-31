p = 'crates/toolkit/widgets/src/shortcut.rs'
s = open(p).read()
s = s.replace('''const BODY_SIZE: f32 = 13.0;''', '''const KEY_NAMES: [(&str, &str); 2] = [
    ("Space", "key-space"),
    ("Mission Control", "key-mission-control"),
];

const BODY_SIZE: f32 = 13.0;''', 1)
a = '''        None if key.chars().count() == 1 => key.chars().flat_map(char::to_uppercase).collect(),
        None => key.to_owned(),
    };'''
b = '''        None if key.chars().count() == 1 => key.chars().flat_map(char::to_uppercase).collect(),
        None => KEY_NAMES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map_or_else(|| key.to_owned(), |(_, message)| L10N.tr(message)),
    };'''
assert a in s
s = s.replace(a, b)
s = s.replace('''use super::{Context, Size, Widget, WidgetId};''', '''use super::{Context, L10N, Size, Widget, WidgetId};''')
s = s.replace('''        assert_eq!(shortcut_keys("Space"), keys("", false, "Space"));''', '''        assert_eq!(shortcut_keys("Space"), keys("", false, "Space"));
        assert_eq!(
            shortcut_keys("Super+Mission Control"),
            keys("", true, "Mission Control")
        );''')
open(p, 'w').write(s)
p = 'crates/toolkit/widgets/locales/en-US/unbound-widgets.ftl'
s = open(p).read()
s = s.rstrip('\n') + '\nkey-space = Space\nkey-mission-control = Mission Control\n'
open(p, 'w').write(s)
