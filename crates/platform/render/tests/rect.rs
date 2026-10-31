use bande_render::{Rect, disjoint};

const OUTPUT: Rect = Rect::new(0, 0, 100, 100);

#[test]
fn damage_is_clipped_to_the_output_and_overlaps_merge_into_disjoint_rects() {
    let cases: [(&str, Vec<Rect>, Vec<Rect>); 4] = [
        (
            "overlapping damage is painted once",
            vec![Rect::new(0, 0, 20, 20), Rect::new(10, 10, 20, 20)],
            vec![Rect::new(0, 0, 30, 30)],
        ),
        (
            "separate damage stays separate",
            vec![Rect::new(0, 0, 10, 10), Rect::new(50, 50, 10, 10)],
            vec![Rect::new(0, 0, 10, 10), Rect::new(50, 50, 10, 10)],
        ),
        (
            "damage outside the bounds is dropped",
            vec![Rect::new(90, 90, 20, 20), Rect::new(200, 0, 5, 5)],
            vec![Rect::new(90, 90, 10, 10)],
        ),
        (
            "a merge that reaches an earlier rect absorbs it",
            vec![
                Rect::new(0, 0, 10, 10),
                Rect::new(40, 0, 10, 10),
                Rect::new(5, 5, 40, 2),
            ],
            vec![Rect::new(0, 0, 50, 10)],
        ),
    ];
    for (case, damage, expected) in cases {
        assert_eq!(disjoint(damage, OUTPUT), expected, "{case}");
    }
}

#[test]
fn a_rect_contains_itself_and_not_a_larger_one() {
    let window = Rect::new(10, 10, 20, 20);

    assert!(window.contains(&window));
    assert!(window.contains(&Rect::new(15, 15, 5, 5)));
    assert!(!window.contains(&Rect::new(5, 10, 20, 20)));
}
