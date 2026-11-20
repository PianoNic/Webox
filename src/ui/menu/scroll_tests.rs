use super::*;

#[test]
fn wheel_offset_is_clamped() {
    assert_eq!(wheel_offset(0.0, -40.0, 100.0), 40.0); // wheel down: the offset grows
    assert_eq!(wheel_offset(90.0, -40.0, 100.0), 100.0);
    assert_eq!(wheel_offset(10.0, 40.0, 100.0), 0.0);
    assert_eq!(wheel_offset(5.0, -5.0, 0.0), 0.0); // nothing to scroll
}

#[test]
fn rows_are_scrolled_into_view_minimally() {
    assert_eq!(scroll_into_view(0.0, 10.0, 40.0, 200.0, 300.0), 0.0); // already visible
    assert_eq!(scroll_into_view(0.0, 220.0, 40.0, 200.0, 300.0), 60.0); // below: its bottom edge becomes the view's bottom
    assert_eq!(scroll_into_view(100.0, 60.0, 40.0, 200.0, 300.0), 60.0); // above: its top becomes the view's top
    assert_eq!(scroll_into_view(0.0, 900.0, 40.0, 200.0, 300.0), 300.0); // clamped
}
