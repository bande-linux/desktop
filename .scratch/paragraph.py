p = 'crates/toolkit/graphics/src/text.rs'
s = open(p).read()
reps = [
('''        layout.align(alignment, AlignmentOptions::default());
        Paragraph {
            layout: Arc::new(layout),
        }''', '''        layout.align(alignment, AlignmentOptions::default());
        Paragraph::from_layout(layout, &text)'''),
('''pub struct Paragraph {
    layout: Arc<Layout<Color>>,
}''', '''pub struct Paragraph {
    layout: Arc<Layout<Color>>,
    text: Arc<str>,
}'''),
('''    pub fn from_layout(layout: Layout<Color>) -> Self {
        Self {
            layout: Arc::new(layout),
        }
    }''', '''    pub fn from_layout(layout: Layout<Color>, text: &str) -> Self {
        Self {
            layout: Arc::new(layout),
            text: Arc::from(text),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }'''),
]
for a, b in reps:
    assert a in s, a
    s = s.replace(a, b)
open(p, 'w').write(s)
p = 'crates/toolkit/widgets/src/editor.rs'
s = open(p).read()
s = s.replace('        Paragraph::from_layout(layout)\n', '        Paragraph::from_layout(layout, &content)\n')
open(p, 'w').write(s)
