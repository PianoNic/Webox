//! macOS: no screen sharing yet. The native pieces would be ScreenCaptureKit (`SCShareableContent` for the picker's screens and
//! windows, `SCStream` for the pictures; macOS 12.3+, needs the Screen Recording permission) and VideoToolbox
//! (`VTCompressionSession`, H.264 Constrained Baseline) for the encoder.
use super::{Opts, Out, ShareSource};

pub(super) fn sources() -> Vec<ShareSource> { vec![] }

pub(super) fn thumbnail(_s: &ShareSource, _max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> { None }

/// The tests treat "no H.264 encoder" as a missing system piece and skip.
pub(super) fn run(_src: &ShareSource, _opts: Opts, _out: &Out) -> Result<(), String> {
    Err("no H.264 encoder: screen sharing is not available on macOS yet".into())
}
