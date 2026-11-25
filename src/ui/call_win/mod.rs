//! Separate OS windows of the call UI: the call window and the calling dock (mini call window); the incoming
//! toast lives in `call_toast.rs`. All three are egui *immediate* viewports shown from `App::call_overlay`, so their UI closures
//! borrow `&mut App` directly (no Arc/Mutex/channels needed) and run once per main-window frame.
//!
//! Webox's own choices: call-window minimum size, control-bar colour and auto-hide delay, dock body colours, the call
//! window's theme (global theme used), when "covered by other apps" counts (here: no Webox window focused).
//!
//! Settings keys read / written (the Settings page can surface them): `call_separate_windows` (bool, default true),
//! `call_autohide_controls` (bool, default true = `desktop-auto-hide-call-control-enabled`), `call_win_rect` ([x,y,w,h]),
//! `dock_auto_covered` (bool, default false: "Automatically show my Webex call or meeting
//! in a floating mini window when other apps cover it"), `dock_keep_in_front` (bool, default false: "Keep the calling dock in
//! front" = the pin button), `calling_dock` (bool, default true: "Enable the calling dock"; off = no automatic dock),
//! `dock_dont_ask` (bool: "Don't ask me again"), `dock_rect` ([x,y,w,h]).
use super::call::{state_text, CallActions, CallView};
use super::call_toast::{ghost_at, round_at};
use super::menu::{menu_toggle, webex_menu, Align, Entry, Item, Side};
use super::toast_pos::{round_by_title, work_area};
use crate::cloud::locus::CallState;
use crate::*;

// Sizes (px).
const TITLE_BAR_H: f32 = 40.0; // title bar height
const WIN_BTN_W: f32 = 40.0; // caption button width
const BAR_H: f32 = 56.0; // floating control bar height
const BAR_SIDE: f32 = 4.0; // control bar side margin
const BAR_GAP: f32 = 8.0; // control bar bottom gap
const BAR_OPACITY: f32 = 0.8; // control bar opacity
#[cfg(windows)] const MAX_OVERFLOW: f32 = 10.0; // a maximised frameless window overflows the screen by 10 px top / right (Windows only)
#[cfg(not(windows))] const MAX_OVERFLOW: f32 = 0.0;
const MAX_BAR_INSET: f32 = 5.0; // control bar inset while maximised
const INTERSTITIAL: Vec2 = Vec2::new(788.0, 560.0); // pre-join page size
const AUTOHIDE_SECS: f64 = 3.0; // control bar auto-hide delay
const CALL_MIN: Vec2 = Vec2::new(480.0, 360.0); // call window minimum size
const DOCK_DEFAULT: Vec2 = Vec2::new(446.0, 500.0); // calling dock default size
const DOCK_MIN: Vec2 = Vec2::new(446.0, 480.0);
const DOCK_MAX: Vec2 = Vec2::new(588.0, 990.0);
const DOCK_THUMB: f32 = 60.0; // docked thumbnail size
const DOCK_SNAP: f32 = 20.0; // docking distance from a screen edge
const CALL_TITLE: &str = "Webox - Call"; // window titles are shown through tr()
const DOCK_TITLE: &str = "Webox - Calling dock";

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum DockSide { Left, Right, Top }
/// Dock states: Normal, Docked (60x60 thumbnail on a screen edge), DockedHovered (expanded while the pointer is over it).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum DockMode { #[default] Normal, Docked(DockSide), Hovered(DockSide) }

#[derive(Default)]
pub(crate) struct WinState {
    // Toast (call_toast.rs).
    pub toast_closed: Option<f64>, pub toast_pos: Option<egui::Pos2>,
    /// The incoming-call toast's window, once made see-through (`tray::see_through`; None again when the call stops ringing).
    pub toast_hwnd: Option<isize>,
    // Call window.
    call_b: Option<(egui::Pos2, Vec2)>, last_move: f64, call_saved: Option<[f32; 4]>,
    // Dock.
    pub dock_open: bool, dock_suppress: bool, pub dock_shown: bool,
    dock_b: Option<(egui::Pos2, Vec2)>, dock_mode: DockMode, dock_actual: Option<(egui::Pos2, Vec2)>, dock_dragging: bool, dock_last_move: f64,
    dock_last: Option<egui::Pos2>, dock_restore: Option<Vec2>,
    dock_hover_out: Option<f64>, dock_confirm: bool, dock_chk: bool, dock_saved: Option<[f32; 4]>,
    // Focus of our windows in the previous frame (main, call window, dock), for "covered by other apps".
    focus_call: bool, focus_dock: bool,
    pub sample_done: bool, rounded: u8, save_t: f64,
    /// "Call statistics" panel (stats.rs); a full-screen toggle asked for (title bar button, Alt+Enter; full_screen.rs).
    pub(crate) stats: stats::StatsUi, pub(crate) full_toggle: bool,
    /// The participants panel (`roster.rs`).
    pub roster: roster::RosterUi,
}

fn rect_setting(s: &serde_json::Value, key: &str) -> Option<[f32; 4]> {
    let a = s[key].as_array()?;
    let v: Vec<f32> = a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
    (v.len() == 4 && v[2] > 100.0 && v[3] > 100.0).then(|| [v[0], v[1], v[2], v[3]])
}

impl WinState {
    fn dock_last_pos(&self) -> Option<egui::Pos2> { self.dock_last }
    fn set_dock_last_pos(&mut self, p: egui::Pos2) { self.dock_last = Some(p); }
    /// Strg+Umschalt+X: a shown dock closes and stays closed for this call, a hidden one opens.
    fn toggle_dock(&mut self) {
        if self.dock_shown { self.dock_open = false; self.dock_suppress = true; } else { self.dock_open = true; self.dock_suppress = false; }
    }
}

mod call;
pub(crate) use call::{fill_uv, fit_size};
mod dock;
mod full_screen;
mod stats;
mod roster;
