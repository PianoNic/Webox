use super::*;

fn g(w: u32, h: u32) -> Gif { Gif { thumb_w: w, thumb_h: h, ..Default::default() } }

#[test]
fn masonry_fills_the_shortest_of_three_columns() {
    let (r, total) = masonry(&[g(200, 200), g(200, 100), g(200, 150), g(200, 100), g(200, 100)], 100.0, 3);
    assert_eq!(r[0].min, pos2(0.0, 0.0));
    assert_eq!(r[1].min, pos2(100.0 + GAP, 0.0), "second tile goes to the next empty column");
    assert_eq!(r[2].min, pos2(2.0 * (100.0 + GAP), 0.0), "third fills the third column");
    assert_eq!(r[3].min, pos2(100.0 + GAP, 50.0 + GAP), "fourth under the shortest (middle) column");
    assert_eq!(r[4].min, pos2(2.0 * (100.0 + GAP), 75.0 + GAP), "fifth under the now shortest (right) column");
    assert_eq!(total, 75.0 + 50.0 + 2.0 * GAP, "the tallest column (right), trailing gap included");
}

#[test]
fn masonry_keeps_aspect_ratio() {
    let (r, _) = masonry(&[g(200, 150)], 150.0, COLS);
    assert_eq!(r[0].size(), Vec2::new(150.0, 113.0));
}

#[test]
fn original_layout_matches_the_decoded_widget() {
    // Fixed 652 x 406, search row 52, 40 px side spacers, 28 x 28 buttons, footer 32.
    let rect = Rect::from_min_size(pos2(100.0, 50.0), Vec2::new(W, H));
    assert_eq!((W, H), (652.0, 406.0));
    let l = layout(rect);
    assert_eq!(l.search.height(), 52.0);
    assert_eq!(l.icon, Rect::from_min_size(pos2(140.0, 62.0), Vec2::splat(28.0)));
    assert_eq!(l.cancel.right(), rect.right() - 40.0);
    assert!(l.field.width() >= 464.0, "the search field's minimum width");
    assert_eq!(l.foot.height(), 32.0);
    assert_eq!(l.grid.top(), l.search.bottom());
    assert_eq!(l.grid.bottom(), l.foot.top());
    assert_eq!(l.logo_x.0, rect.left() + 16.0 + 75.0);
    // Three columns of about 200 px.
    let c = col_width(l.grid.width(), COLS);
    assert!((195.0..=205.0).contains(&c), "column width {c}");
    assert!(3.0 * c + 2.0 * GAP + 2.0 * GRID_PAD <= W);
}

#[test]
fn keyboard_moves_through_the_grid() {
    let (t, _) = masonry(&[g(200, 200), g(200, 100), g(200, 100), g(200, 100), g(200, 100)], 100.0, 3);
    assert_eq!(grid_step(&t, None, Key::ArrowDown), Some(0), "the first key selects the first GIF");
    assert_eq!(grid_step(&t, Some(0), Key::ArrowRight), Some(1));
    assert_eq!(grid_step(&t, Some(1), Key::ArrowLeft), Some(0));
    assert_eq!(grid_step(&t, Some(1), Key::ArrowDown), Some(3), "down stays in the column");
    assert_eq!(grid_step(&t, Some(3), Key::ArrowUp), Some(1));
    assert_eq!(grid_step(&t, Some(0), Key::ArrowUp), Some(0), "nothing above: stays");
    assert_eq!(grid_step(&t, Some(4), Key::ArrowRight), Some(4), "last stays last");
    assert_eq!(grid_step(&[], None, Key::ArrowDown), None);
}

#[test]
fn original_grid_has_three_200px_columns_next_to_the_arrow_bar() {
    let l = layout(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(W, H)));
    let c = grid_col_width(l.grid.width());
    assert_eq!(c, 200.0);
    assert_eq!(PAD_L + 3.0 * c + 2.0 * GAP + PAD_R + crate::ui::autoscroll::BAR_W, W, "tiles + gaps + paddings + bar fill the width");
}

#[test]
fn popupwidget_is_centred_over_the_button_with_the_caret_down() {
    // A window far bigger than the popup, the GIF button (32 x 32) in the middle of the composer toolbar.
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1600.0, 1000.0));
    let btn = Rect::from_min_size(pos2(784.0, 900.0), Vec2::splat(32.0));
    let p = place(screen, btn);
    // Popup = content 652 x 406 + 1 px border + 10 px caret strip + 4 px shadow margins.
    assert_eq!(p.window.size(), Vec2::new(662.0, 426.0));
    assert_eq!(p.content.size(), Vec2::new(W, H));
    assert_eq!(p.window.center().x, btn.center().x, "alignment 1 = centred on the button");
    assert_eq!(p.window.bottom(), btn.top(), "arrow 3: the widget ends at the button's top");
    assert_eq!(p.tip, pos2(800.0, btn.top() - SHADOW_M), "the caret's tip at the button's centre, 4 px (the shadow margin) above it");
    assert_eq!(p.body.bottom(), p.tip.y - CARET_H);
    assert_eq!(p.content.min, p.body.min + Vec2::splat(1.0), "content inside the 1 px border");
    assert_eq!(p.body.bottom() - p.content.bottom(), 1.0);
    assert!(p.contains(pos2(800.0, p.tip.y - 2.0)) && !p.contains(pos2(812.0, p.tip.y - 2.0)), "the caret is part of the popup");
}

#[test]
fn popupwidget_moves_back_into_the_window_by_at_most_half_its_size() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1600.0, 1000.0));
    // Button near the left edge: centred, the popup would start at -251; moved right by 251 (< half the width, 331).
    let btn = Rect::from_min_size(pos2(64.0, 900.0), Vec2::splat(32.0));
    let p = place(screen, btn);
    assert_eq!(p.window.left(), 0.0);
    assert_eq!(p.tip.x, 80.0, "the caret still points at the button");
    // Right edge.
    let p = place(screen, Rect::from_min_size(pos2(1560.0, 900.0), Vec2::splat(32.0)));
    assert_eq!(p.window.right(), 1600.0);
    // A button so far out that half the width is not enough: moved by half only, the caret kept 20 px from the right.
    let p = place(screen, Rect::from_min_size(pos2(1900.0, 900.0), Vec2::splat(32.0)));
    assert_eq!(p.window.left(), 1916.0 - 331.0 - 331.0);
    assert_eq!(p.tip.x, p.window.right() - SHADOW_M - CARET_W);
    // Not enough room above: moved down by at most half the height.
    let p = place(screen, Rect::from_min_size(pos2(784.0, 100.0), Vec2::splat(32.0)));
    assert_eq!(p.window.top(), 100.0 - 426.0 + 213.0);
}

#[test]
fn popup_sits_above_the_button_inside_the_window() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(900.0, 800.0));
    let r = popup_rect(screen, Rect::from_min_size(pos2(850.0, 700.0), Vec2::splat(28.0)));
    assert_eq!(r.size(), Vec2::new(W, H));
    assert!(r.right() <= screen.right() - 8.0 && r.bottom() <= 700.0);
}
