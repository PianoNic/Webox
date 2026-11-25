//! Camera video in a call, the pictures the call window shows: the far end's camera (`video/main`, decoded like a received
//! share by `share_rx::Decode`, into `MAIN`) and our own camera as it is sent (`SELF`, made by `media::camera::send`).
//!
//! The UI reads `remote()` / `self_view()` each frame and uploads a picture only when its `seq` changed; the decoder and the
//! camera wake the UI once per picture, so nothing redraws faster than pictures arrive. With the camera off and no video
//! from the far end nothing here runs.
use super::share_rx::{Picture, Slot};
use std::sync::Arc;

/// The far end's camera, newest decoded picture.
pub static MAIN: Slot = Slot::new();
/// Our camera after the plugins' frame filters, as it goes to the encoder (small, not mirrored).
pub static SELF: Slot = Slot::new();

/// The far end's newest camera picture (None while none came, and after `clear_remote`).
pub fn remote() -> Option<Arc<Picture>> { MAIN.latest() }
/// Our own newest camera picture for the self view (None while the camera is off).
pub fn self_view() -> Option<Arc<Picture>> { SELF.latest() }
/// The far end stopped its camera, or the call ended.
pub fn clear_remote() { MAIN.clear() }
/// Our camera stopped.
pub fn clear_self() { SELF.clear() }
