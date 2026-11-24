//! The window, one file per part of the screen.
pub(crate) mod call;
pub(crate) mod typing; // typing indicators (protocol: cloud/typing.rs)
pub(crate) mod call_win;
pub(crate) mod call_toast;
#[cfg(target_os = "macos")]
pub(crate) mod call_note_macos; // the incoming-call banner (Annehmen / Ablehnen)
pub(crate) mod toast_pos;
pub(crate) mod chrome;
pub(crate) mod theme; // the Momentum themes and their colour tokens
pub(crate) mod conversation;
pub(crate) mod closed; // read-only notice of a closed chat
pub(crate) mod welcome; // empty-chat welcome item
pub(crate) mod create_space;
pub(crate) mod header_plus;
pub(crate) mod new_message;
pub(crate) mod invite;
pub(crate) mod frame;
pub(crate) mod login;
pub(crate) mod loading;
pub(crate) mod messages;
pub(crate) mod messaging;
pub(crate) mod pages;
pub(crate) mod settings;
pub(crate) mod customize; // Settings > Oberfläche anpassen: shown / hidden / ordered UI elements
pub(crate) mod settings_shortcuts;
pub(crate) mod space_menu;
pub(crate) mod widgets;
pub(crate) mod shot_input; // --screenshot: scripted pointer / wheel events
pub(crate) mod edit_overlay; // editing a message: an overlay over the conversation
pub(crate) mod autoscroll; // middle-click autoscroll of every vscroll list
pub(crate) mod scroll_route; // wheel over row overlays (hover toolbar, tooltips) scrolls the list under them
pub(crate) mod sample_photos;
pub(crate) mod emoji_picker;
pub(crate) mod emoji_text;
pub(crate) mod gif_picker;
pub(crate) mod menu;
pub(crate) mod credits;
pub(crate) mod page_kit;
pub(crate) mod calls_page;
pub(crate) mod meetings_page;
pub(crate) mod contacts_page;
pub(crate) mod teams_page;
pub(crate) mod composer;
pub(crate) mod composer_text;
pub(crate) mod type_focus; // typing with nothing focused goes to the composer
pub(crate) mod emoji_autocomplete;
pub(crate) mod mention_popup;
pub(crate) mod space_tabs;
pub(crate) mod global_search;
pub(crate) mod profile;
pub(crate) mod profile_menu;
pub(crate) mod app_dialogs;
pub(crate) mod status_time;
pub(crate) mod settings_effects;
pub(crate) mod attach;
pub(crate) mod screen_capture;
pub(crate) mod share_picker; // "Inhalte freigeben": the screen / window picker of screen sharing
pub(crate) mod msg_actions;
pub(crate) mod schedule_send;
pub(crate) mod unfurl;
pub(crate) mod meetcall;
pub(crate) mod spaces_ext;
pub(crate) mod presence; // the presence line under a person's name
pub(crate) mod update_dialog; // "Auf Updates prüfen" and the daily check

/// The main window's context, for background threads that hand a result to the UI (`wake`).
static UI_CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();
/// Called at the start of every frame (cheap after the first).
pub(crate) fn set_ui_ctx(ctx: &eframe::egui::Context) { if UI_CTX.get().is_none() { let _ = UI_CTX.set(ctx.clone()); } }
/// A background result is waiting for the UI: draw a frame now. The window has no periodic redraw, so every worker that
/// hands something to the UI calls this after sending it. No-op before the first frame.
pub(crate) fn wake() { if let Some(c) = UI_CTX.get() { c.request_repaint(); } }
/// Puts `text` on the clipboard (from code that has no `egui::Context` at hand, e.g. the plugin outbox).
pub(crate) fn copy_text(text: String) { if let Some(c) = UI_CTX.get() { c.copy_text(text); } }
