p = 'crates/testing/golden/src/text.rs'
s = open(p).read()
reps = [
('''    bounds: Rect,
    visible: Rect,
}''', '''    bounds: Rect,
    visible: Rect,
    clip: Rect,
}'''),
('''                        visible: intersection(bounds, placement.clip),
                    });''', '''                        visible: intersection(bounds, placement.clip),
                        clip: placement.clip,
                    });'''),
]
for a, b in reps:
    assert a in s, a
    s = s.replace(a, b)
a = s.index('pub(crate) fn clipped_texts')
b = s.index('pub(crate) fn overlapping_texts')
s = s[:a] + '''pub(crate) fn clipped_texts(scene: &Scene) -> Vec<String> {
    placed_texts(scene)
        .into_iter()
        .filter(|placed| {
            let shown = placed.visible.width > 0.0 && placed.visible.height > 0.0;
            let cut_across = placed.bounds.width - placed.visible.width > SLACK_ACROSS;
            let tight = placed.clip.height < 2.0 * placed.bounds.height;
            let cut_down = tight && placed.bounds.height - placed.visible.height > SLACK_DOWN;
            shown && (cut_across || cut_down)
        })
        .map(|placed| placed.text)
        .collect()
}

''' + s[b:]
s = s.replace('''    fn text_cut_by_its_clip_or_the_window_edge_is_reported() {''', '''    fn text_cut_by_its_clip_or_the_window_edge_is_reported_but_not_text_scrolled_away() {''')
s = s.replace('''        assert_eq!(
            clipped_texts(&scene(vec![fits, cut, shifted])),''', '''        let scrolled = Item::Clip {
            shape: Shape::rect(Rect::new(0.0, 0.0, 200.0, 80.0)),
            items: vec![
                text(&mut system, "Below the fold", Point::new(4.0, 76.0)),
                text(&mut system, "Far below", Point::new(4.0, 300.0)),
            ],
        };
        assert_eq!(
            clipped_texts(&scene(vec![fits, cut, shifted, scrolled])),''')
open(p, 'w').write(s)
