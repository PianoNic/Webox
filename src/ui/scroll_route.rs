//! Wheel routing for row overlays of a scrolling list.
//!
//! Some parts of a list row live in their own egui layer so they stay hovered while the pointer is on them: a message's
//! hover toolbar (reactions, edit, reply, forward, "···"), the join/leave popover, and every tooltip. egui's `ScrollArea`
//! takes the wheel only when the pointer is on the area's own layer, so the wheel would stop working over those overlays.
//! The overlays register here as "scroll pass-through" each frame; a list asks `wheel_passthrough` before it shows its
//! scroll area and applies the returned delta itself. Real overlays (menus, pickers, dialogs, the image viewer) do not
//! register and keep blocking the wheel.
use eframe::egui;
use egui::{Id, LayerId, Order, Pos2, Rect};

const K_PASS: &str = "scroll-pass-layers";

/// Marks `layer` (a row overlay of a list) as transparent for the mouse wheel in this and the next frame.
pub(crate) fn scroll_passthrough(ctx: &egui::Context, layer: LayerId) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let m = d.get_temp_mut_or_default::<Vec<(LayerId, u64)>>(Id::new(K_PASS));
        m.retain(|(l, p)| *l != layer && p + 2 >= pass);
        m.push((layer, pass));
    });
}

fn passthrough_layers(ctx: &egui::Context) -> Vec<LayerId> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Vec<(LayerId, u64)>>(Id::new(K_PASS))).unwrap_or_default().into_iter().filter(|(_, p)| p + 2 >= pass).map(|(l, _)| l).collect()
}

/// The layer that owns the wheel at a point: `stack` = the visible layers containing the point, topmost first. Tooltips and
/// pass-through layers are skipped; the first other layer gets the wheel (`None` when only skipped layers are there).
pub(crate) fn wheel_owner(stack: &[LayerId], is_pass: impl Fn(LayerId) -> bool) -> Option<LayerId> {
    stack.iter().copied().find(|l| l.order != Order::Tooltip && !is_pass(*l))
}

/// Visible layers that contain `p`, topmost first (egui's own order, as `Context::layer_id_at` walks it).
fn layers_at(ctx: &egui::Context, p: Pos2) -> Vec<LayerId> {
    ctx.memory(|m| {
        let ids: Vec<LayerId> = m.layer_ids().collect();
        ids.into_iter().rev().filter(|l| m.areas().is_visible(l) && m.area_rect(l.id).is_some_and(|r| r.contains(p))).collect()
    })
}

/// The wheel delta (egui's smoothed `smooth_scroll_delta.y`, positive = content moves down = scroll up) that belongs to the
/// list drawn in `ui` over `rect` but that its `ScrollArea` won't see, because the pointer is on a row overlay or a tooltip
/// above it. The delta is taken out of the input (no other area scrolls with it); 0 when the list gets the wheel itself or
/// something else (a menu, a dialog, ...) is on top.
pub(crate) fn wheel_passthrough(ui: &egui::Ui, rect: Rect) -> f32 {
    let ctx = ui.ctx();
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return 0.0 };
    if !ui.clip_rect().intersect(rect).contains(p) { return 0.0; }
    let top = ctx.layer_id_at(p); // honours modal layers
    if top == Some(ui.layer_id()) || top.is_none() { return 0.0; } // the scroll area handles it
    if !layer_reaches(ui, p) { return 0.0; }
    ctx.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y))
}

/// Whether pointer input at `p` belongs to `ui`'s layer: it is the top layer there, or only tooltips and row overlays of the
/// list lie above it (for the wheel and for middle-click autoscroll).
pub(crate) fn layer_reaches(ui: &egui::Ui, p: Pos2) -> bool {
    let ctx = ui.ctx();
    let mine = ui.layer_id();
    let top = ctx.layer_id_at(p);
    if top == Some(mine) { return true; }
    let pass = passthrough_layers(ctx);
    let is_pass = |l: LayerId| pass.contains(&l);
    if top.is_none_or(|t| t.order != Order::Tooltip && !is_pass(t)) { return false; } // a real overlay blocks
    wheel_owner(&layers_at(ctx, p), is_pass) == Some(mine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(o: Order, n: &str) -> LayerId { LayerId::new(o, Id::new(n)) }

    #[test]
    fn wheel_goes_through_toolbars_and_tooltips_to_the_list() {
        let list = LayerId::background();
        let bar = l(Order::Foreground, "msg-toolbar");
        let tip = l(Order::Tooltip, "tip");
        let pass = |x: LayerId| x == bar;
        assert_eq!(wheel_owner(&[bar, list], pass), Some(list));
        assert_eq!(wheel_owner(&[tip, bar, list], pass), Some(list));
        assert_eq!(wheel_owner(&[tip, list], pass), Some(list));
    }

    #[test]
    fn real_overlays_keep_the_wheel() {
        let list = LayerId::background();
        let bar = l(Order::Foreground, "msg-toolbar");
        let menu = l(Order::Foreground, "menu");
        let pass = |x: LayerId| x == bar;
        assert_eq!(wheel_owner(&[menu, bar, list], pass), Some(menu));
        assert_eq!(wheel_owner(&[l(Order::Tooltip, "t"), menu, list], pass), Some(menu)); // a tooltip of the menu
        assert_eq!(wheel_owner(&[bar], pass), None);
    }

    #[test]
    fn registration_expires() {
        let ctx = egui::Context::default();
        let bar = l(Order::Foreground, "msg-toolbar");
        let _ = ctx.run(Default::default(), |c| scroll_passthrough(c, bar));
        assert!(passthrough_layers(&ctx).contains(&bar));
        for _ in 0..3 { let _ = ctx.run(Default::default(), |_| {}); }
        assert!(!passthrough_layers(&ctx).contains(&bar));
    }
}
