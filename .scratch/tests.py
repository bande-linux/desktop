p = 'crates/shell/bar/tests/pseudo.rs'
s = open(p).read()
a = s.index('fn pseudo_locale()')
b = s.index('fn rail(expanded: bool)')
s = s[:a] + '''fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data")
}

fn problems(name: &str, scene: &Scene) -> Vec<String> {
    unbound_golden::pseudo_problems!(name, scene, &DATA)
}

''' + s[b:]
s = s.replace('    pseudo_locale();\n', '    unbound_golden::use_pseudo_locale();\n')
s = s.replace('#![allow(clippy::expect_used)]\n\nuse std::path::PathBuf;\nuse std::sync::Once;\n', '#![allow(clippy::expect_used)]\n\nuse std::path::PathBuf;\n')
s = s.replace('use unbound_toolkit::{CpuRenderer, Palette, Scene, Scheme, TextSystem};', 'use unbound_toolkit::{Palette, Scene, Scheme, TextSystem};')
open(p, 'w').write(s)

p = 'crates/apps/settings/tests/pseudo.rs'
s = open(p).read()
a = s.index('fn pseudo_locale()')
b = s.index('#[test]')
s = s[:a] + s[b:]
s = s.replace('    pseudo_locale();\n', '    unbound_golden::use_pseudo_locale();\n')
s = s.replace('use std::sync::Once;\n', '')
a = s.index('        let mut rgba = vec![0;')
b = s.index('    }\n    assert!(problems')
s = s[:a] + '''        let name = format!("settings-{}", pane.keyword());
        problems.extend(unbound_golden::pseudo_problems!(&name, &scene, &DATA));
''' + s[b:]
s = s.replace('''    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pseudo");
    std::fs::create_dir_all(&folder).expect("folder");
''', '')
s = s.replace('''    CpuRenderer, FrameRegions, HEADER_HEIGHT, Palette, Scheme, TextSize, TextSystem, framed_scene,''', '''    FrameRegions, HEADER_HEIGHT, Palette, Scheme, TextSize, TextSystem, framed_scene,''')
s = s.replace('#![allow(clippy::expect_used, clippy::panic)]\n\n', '')
open(p, 'w').write(s)
