//! Notifications: custom toast windows (not native Windows toasts) and the system tray.
//!
//! Layout (matches the Webex desktop notifications): cards 400 wide, 8 apart, radius 8, 1 px border (`modal-primary-background`
//! / `modal-primary-border`), 32 px avatar or 24 px icon at (16, 16), title 16 px bold (2 lines), message 16 px (3 lines for
//! chat / 5 otherwise), ghost XS round close button (`cancel-bold`) 8 px from the top right corner, small pill buttons
//! right-aligned 12 px below the text. The toasts (message, reminder, summary, recording, whiteboard) fade for 400 ms like
//! the incoming-call toast; they stack at the bottom right of the work area for [`DURATION`] seconds.
//!
//! One transparent, always-on-top, no-activate window (egui immediate viewport) holds the whole stack, newest at the bottom, so a toast
//! never steals the focus (`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, see `tray.rs`). Hover pauses the timeout, a click on the card focuses
//! the main window and opens the space, the X dismisses.
//!
//! Settings keys (in settings.json; the settings page writes them, everything defaults as stated):
//! * `notif_desktop` bool, default true - desktop (toast) notifications for messages
//! * `notif_mentions_only` bool, default false - only direct messages and @mentions to me (the settings page sets notif_desktop=false for "Aus")
//!   "Alle Nachrichten (Standard)" / "Direktnachrichten und @Erwähnungen an mich" / "Aus"
//! * `notif_preview` bool, default true - "Nachrichtenvorschau anzeigen"; off shows "Neue Nachricht" instead of the text
//! * `notif_taskbar_flash` bool, default true - "Benachrichtigungen in der Taskleiste" (flashing taskbar button + unread overlay badge)
//! * `meeting_toast_enabled` bool, default true, `meeting_toast_minutes` 5 (default) | 1 | 0 - meeting reminder
//! * `close_to_tray` bool, default true - the window's X hides the app to the tray (it keeps running there) instead of quitting
//! * Presence "Nicht stören" (`App::me.status == "DoNotDisturb"`) silences every toast and the flashing.
use crate::*;
use super::tray;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::Galley;
use std::sync::Arc;
use std::time::Instant;

pub(crate) const FADE: f64 = 0.4; // fade-out in seconds, like the incoming-call toast
/// How long a message toast stays (s); hover stops the timer, leaving restarts it (the stack's hover pause below).
pub(crate) const DURATION: f64 = 6.0;
/// Incoming-message toast layout: card 330 wide, margins 15 / 11 / 15 / 15, a 36 wide avatar column holding the 32 px avatar,
/// 12 px gap, 252 wide content; space name "Segoe UI Semibold" 11 pt after an 8 px spacer (min height 22), sender line 14 px
/// after 8 px (min height 18), message 11 pt after 2 px, at most 42 px high (2 lines). 11 pt at 96 dpi = 14.67 px.
/// The card has square corners, fill + 1 px border, no shadow (see card.rs).
mod msg_toast {
    pub const W: f32 = 330.0;
    pub const ML: f32 = 15.0; pub const MT: f32 = 11.0; pub const MR: f32 = 15.0; pub const MB: f32 = 15.0;
    pub const AVATAR_COL: f32 = 36.0; pub const AVATAR: f32 = 32.0; pub const GAP: f32 = 12.0; pub const COL_W: f32 = 252.0;
    pub const PT11: f32 = 11.0 * 96.0 / 72.0;
    pub const SENDER_PX: f32 = 14.0;
    pub const TITLE_MIN_H: f32 = 22.0; pub const SENDER_MIN_H: f32 = 18.0; pub const BODY_MAX_H: f32 = 42.0;
}
const W: f32 = 400.0; // card width (px)
const GAP: f32 = 8.0; // gap between cards (px)
const EDGE: f32 = 16.0; // distance of the stack from the work area's right / bottom edge (px)
const TITLE: &str = "Webox toast";
const MENU_TITLE: &str = "Webox tray menu";

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Kind { Message, Mention, MeetingReminder, MessageReminder, CallSummary, Recording, Whiteboard, Info }

/// Colour of the highlighted lead-in of a preview (`wx-incomingMessageToast-replyLabel` / `-fileLabel`, `message-bodyText-mention-self-text`).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone { Reply, File, Mention }

#[derive(Clone)]
pub(crate) struct Toast {
    id: u64, kind: Kind,
    /// Small line above the title (e.g. "Meeting-Erinnerung").
    kicker: String, title: String, lead: Option<(String, Tone)>, body: String,
    /// (name, photo key) of the avatar; none = the kind's icon.
    avatar: Option<(String, String)>,
    space: Option<String>, link: Option<String>, key: String,
    /// (id, label, primary)
    buttons: Vec<(&'static str, String, bool)>,
    born: f64, deadline: f64, hovered: bool, dismissed: bool,
    /// The notification server failed it once: it stays the app's own toast (Linux, `native_toasts`).
    native_failed: bool,
}

impl Toast {
    pub(crate) fn new(kind: Kind, title: impl Into<String>, body: impl Into<String>) -> Self {
        Toast { id: 0, kind, kicker: String::new(), title: title.into(), lead: None, body: body.into(), avatar: None, space: None, link: None, key: String::new(),
            buttons: vec![], born: 0.0, deadline: 0.0, hovered: false, dismissed: false, native_failed: false }
    }
    fn kicker(mut self, s: &str) -> Self { self.kicker = s.into(); self }
    fn lead(mut self, s: impl Into<String>, t: Tone) -> Self { self.lead = Some((s.into(), t)); self }
    fn avatar(mut self, name: &str, key: &str) -> Self { self.avatar = Some((name.into(), key.into())); self }
    fn space(mut self, s: &str) -> Self { self.space = Some(s.into()); self }
    fn button(mut self, id: &'static str, label: &str, primary: bool) -> Self { self.buttons.push((id, label.into(), primary)); self }
    fn icon(&self) -> &'static str {
        match self.kind {
            Kind::Message | Kind::Mention => "chat-filled", Kind::MeetingReminder => "calendar-month-filled", Kind::MessageReminder => "alarm-filled",
            Kind::CallSummary => "sparkle-filled", Kind::Recording => "play-filled", Kind::Whiteboard => "whiteboard-content-filled", Kind::Info => "info-circle-filled",
        }
    }
}

enum Out { Open(Option<String>), Button(Toast, &'static str), #[cfg(target_os = "linux")] Menu(String) }

struct TrayMenu { pos: (i32, i32), opened: f64, focused: bool, hwnd: Option<isize>, placed: Option<(i32, i32, i32, i32)> }

#[derive(Default)]
pub(crate) struct Hub {
    t0: Option<Instant>, toasts: Vec<Toast>, next: u64, last: f64, out: Vec<Out>,
    /// Main window state (updated every frame).
    focused: bool, minimized: bool,
    /// The user quit on purpose (tray "Verlassen", Strg+Q, --screenshot): the X must not hide to the tray then.
    pub(crate) quitting: bool,
    tray: Option<tray::Tray>, tray_tried: bool, menu: Option<TrayMenu>,
    hwnd: Option<isize>, placed: Option<(i32, i32, i32, i32)>, measured: f32,
    /// The card rectangles the window was last cut to (no-compositor fallback, tray::popup_shape).
    shaped: Option<Vec<(i32, i32, i32, i32)>>,
    shown_unread: Option<usize>, want_flash: bool, flashing: bool,
    fired: HashSet<String>, snoozed: HashMap<String, u64>,
    started: bool, sample_at: Option<(f64, String)>, hide_at: Option<f64>,
    /// Toasts handed to the desktop's notification server (Linux/Wayland), by its notification id.
    #[cfg(target_os = "linux")]
    native: HashMap<u32, Toast>,
    /// Toasts the notification server was asked to show and hasn't answered for yet (notifications::send doesn't block).
    #[cfg(target_os = "linux")]
    native_wait: Vec<(crate::platform::linux::notifications::Sent, Toast)>,
}

impl Hub {
    fn clock(&mut self) -> f64 { self.t0.get_or_insert_with(Instant::now).elapsed().as_secs_f64() }
    pub(crate) fn push(&mut self, mut t: Toast) {
        let now = self.clock();
        self.next += 1;
        t.id = self.next; t.born = now;
        t.deadline = if std::env::var_os("WEBOX_TOAST_SHOT").is_some() { f64::MAX / 4.0 } else { now + DURATION };
        // A newer toast replaces an older one with the same key (e.g. the same meeting reminded again).
        if !t.key.is_empty() { self.toasts.retain(|o| o.key != t.key); }
        self.toasts.push(t);
        if self.toasts.len() > 5 { self.toasts.remove(0); }
    }
    /// The space was read elsewhere / left: its message toasts go.
    pub(crate) fn withdraw_space(&mut self, space: &str) {
        self.toasts.retain(|t| !(matches!(t.kind, Kind::Message | Kind::Mention) && t.space.as_deref() == Some(space)));
    }
}

// Text layout.

fn lead_color(t: &Th, tone: Tone) -> Color32 {
    let light = ui::menu::light(t);
    match tone { // tokens-light / tokens-dark
        Tone::Reply => if light { hex(0x005E7D) } else { hex(0x07C1F5) },       // wx-incomingMessageToast-replyLabel
        Tone::File => if light { hex(0x7D4705) } else { hex(0xFC9D03) },        // wx-incomingMessageToast-fileLabel
        // Mentions use wx-mentionsHighlight-text.
        Tone::Mention => if light { hex(0x12615A) } else { hex(0x30C9B0) },
    }
}

fn semibold(px: f32) -> FontId { FontId::new(px, egui::FontFamily::Name("semibold".into())) }

fn job(width: f32, rows: usize) -> LayoutJob {
    LayoutJob { wrap: TextWrapping { max_width: width, max_rows: rows, break_anywhere: false, overflow_character: Some('…') }, ..Default::default() }
}

struct Lay { h: f32, kicker: Option<Arc<Galley>>, title: Option<Arc<Galley>>, body: Option<Arc<Galley>>, emoji: Vec<(usize, String)>,
    col_x: f32, buttons: Vec<(Arc<Galley>, f32)>,
    /// Card width, and for the message toast its own rows: (sender line, y of title / sender / body from the top).
    w: f32, sender: Option<Arc<Galley>>, ys: Option<(f32, f32, f32)> }

// The app side.

fn epoch() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

mod card;
#[allow(unused_imports)]
pub(crate) use card::*;
mod hub;
