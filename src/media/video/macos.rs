//! macOS: no video decoder yet, so a shared video shows its error card ("cannot play") and can still be downloaded and opened in
//! QuickTime. The native way would be AVFoundation: `AVAssetReader` with an `AVAssetReaderTrackOutput` per track (BGRA pictures,
//! f32 PCM), VideoToolbox decoding on the GPU.
use super::{Info, Source};
use std::path::Path;

pub(super) fn open(_path: &Path, _max: (u32, u32), _with_audio: bool) -> Result<(Box<dyn Source>, Info), String> {
    Err("Videowiedergabe ist unter macOS noch nicht verfügbar".into())
}
