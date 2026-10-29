use unbound_render::{Rect, disjoint};

const OUTPUT: Rect = Rect::new(0, 0, 100, 100);

#[test]
fn overlapping_damage_is_painted_once() {
    let merged = disjoint([Rect::new(0, 0, 20, 20), Rect::new(10, 10, 20, 20)], OUTPUT);

    assert_eq!(merged, vec![Rect::new(0, 0, 30, 30)]);
}

#[test]
fn separate_damage_stays_separate() {
    let merged = disjoint([Rect::new(0, 0, 10, 10), Rect::new(50, 50, 10, 10)], OUTPUT);

    assert_eq!(merged.iter().map(Rect::area).sum::<u64>(), 200);
}

#[test]
fn damage_outside_the_bounds_is_dropped() {
    let merged = disjoint([Rect::new(90, 90, 20, 20), Rect::new(200, 0, 5, 5)], OUTPUT);

    assert_eq!(merged, vec![Rect::new(90, 90, 10, 10)]);
}

#[test]
fn a_merge_that_reaches_an_earlier_rect_absorbs_it() {
    let merged = disjoint(
        [
            Rect::new(0, 0, 10, 10),
            Rect::new(40, 0, 10, 10),
            Rect::new(5, 5, 40, 2),
        ],
        OUTPUT,
    );

    assert_eq!(merged, vec![Rect::new(0, 0, 50, 10)]);
}

#[test]
fn a_rect_contains_itself_and_not_a_larger_one() {
    let window = Rect::new(10, 10, 20, 20);

    assert!(window.contains(&window));
    assert!(window.contains(&Rect::new(15, 15, 5, 5)));
    assert!(!window.contains(&Rect::new(5, 10, 20, 20)));
}
