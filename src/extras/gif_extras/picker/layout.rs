//! Sizes of the plugin's popup. Standard: the core picker's 652 x 406 rectangle with three columns. Compact (setting
//! "Compact size"): the smaller popup of earlier Webox versions, 350 x 390 above the GIF button with a caret pointing at it,
//! two 156 px columns with 6 px gaps and 76 px home tiles.
use super::{FOOT_H, HEAD_H, PAD};
use crate::ui::gif_picker::{col_width, COLS, GAP, GRID_PAD};
use crate::*;

/// The compact popup (px).
pub(crate) const COMPACT_W: f32 = 350.0;
pub(crate) const COMPACT_H: f32 = 390.0;
const COMPACT_COLS: usize = 2;
const COMPACT_GAP: f32 = 6.0;
const COMPACT_TILE_H: f32 = 76.0;
/// Room for the compact grid's scroll bar (px).
const COMPACT_BAR: f32 = 8.0;
/// Home tile height of the standard size (px).
const TILE_H: f32 = 96.0;
/// The compact popup's caret: 16 px wide, 8 px high, kept 14 px from the corners.
pub(super) const CARET_HW: f32 = 8.0;
pub(super) const CARET_H: f32 = 8.0;
const CARET_INSET: f32 = 14.0;

/// Where and how the popup is laid out this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Geo {
    pub rect: Rect,
    pub compact: bool,
    /// The scroll area of the tiles / results (below the search field, above the footer).
    pub grid: Rect,
    pub cols: usize,
    pub gap: f32,
    pub colw: f32,
    pub tile_h: f32,
}

impl Geo {
    /// Width of the tile columns with their gaps.
    pub(crate) fn content_w(&self) -> f32 { self.colw * self.cols as f32 + self.gap * (self.cols as f32 - 1.0) }
    /// A home tile's place in the scroll area (index `i`).
    pub(crate) fn home_tile(&self, i: usize) -> Rect {
        Rect::from_min_size(pos2((i % self.cols) as f32 * (self.colw + self.gap), (i / self.cols) as f32 * (self.tile_h + self.gap)), Vec2::new(self.colw, self.tile_h))
    }
    pub(crate) fn home_h(&self, n: usize) -> f32 { n.div_ceil(self.cols) as f32 * (self.tile_h + self.gap) }
    /// The caret's corners (base left, base right, tip) under the compact popup, pointing at `anchor`.
    pub(crate) fn caret(&self, anchor: Rect) -> Option<[egui::Pos2; 3]> {
        if !self.compact { return None }
        let r = self.rect;
        let cx = anchor.center().x.clamp(r.left() + CARET_INSET, r.right() - CARET_INSET);
        Some([pos2(cx - CARET_HW, r.bottom()), pos2(cx + CARET_HW, r.bottom()), pos2(cx, r.bottom() + CARET_H)])
    }
    /// Area that keeps clicks inside the popup (the compact one includes the caret).
    pub(crate) fn hit(&self) -> Rect { if self.compact { self.rect.expand2(Vec2::new(0.0, 12.0)) } else { self.rect } }
    /// Clip of the popup's painting (the compact one leaves room for the caret).
    pub(crate) fn clip(&self) -> Rect { if self.compact { self.rect.expand2(Vec2::new(2.0, 14.0)) } else { self.rect.expand(2.0) } }
}

/// The compact popup above the GIF button: starts 40 px left of the button's centre, 10 px above it, kept 8 px inside the window.
pub(crate) fn compact_rect(screen: Rect, anchor: Rect) -> Rect {
    let left = (anchor.center().x - 40.0).clamp(8.0, (screen.right() - COMPACT_W - 8.0).max(8.0));
    let top = (anchor.top() - 10.0 - COMPACT_H).max(8.0);
    Rect::from_min_size(pos2(left, top), Vec2::new(COMPACT_W, COMPACT_H))
}

/// The layout: `rect` is the core picker's rectangle (standard) or `compact_rect` (compact).
pub(crate) fn geo(rect: Rect, compact: bool) -> Geo {
    let grid = Rect::from_min_max(pos2(rect.left() + PAD, rect.top() + HEAD_H + 28.0 + 8.0), pos2(rect.right() - 2.0, rect.bottom() - FOOT_H));
    if compact {
        let colw = ((rect.width() - 2.0 * PAD - COMPACT_BAR - COMPACT_GAP) / COMPACT_COLS as f32).floor();
        Geo { rect, compact, grid, cols: COMPACT_COLS, gap: COMPACT_GAP, colw, tile_h: COMPACT_TILE_H }
    } else {
        let colw = col_width(grid.width() + 2.0 * GRID_PAD - PAD, COLS);
        Geo { rect, compact, grid, cols: COLS, gap: GAP, colw, tile_h: TILE_H }
    }
}
