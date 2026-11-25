//! Sound and video: audio devices and the capture / playback pipeline of calls (`audio`), the receive-side jitter buffer
//! of call audio (`jitter`), Smart Audio noise removal (`denoise`), cameras and the settings preview (`camera`), the
//! ringtones, message sounds and sound preferences (`sounds`), playback of shared videos with the platform's media
//! framework (`video`), screen sharing: capture and H.264 encoding of a screen or window (`share`).
pub mod audio;
pub mod camera;
pub mod denoise;
pub mod jitter;
pub mod mic;
pub mod share;
pub mod sounds;
pub mod video;

/// Tests: a CI runner (GitHub Actions and the like), where there is no speaker, microphone, camera or interactive desktop
/// and often no GPU or media codecs. Tests that need any of them skip there with the reason; the pure tests always run.
#[cfg(test)]
pub(crate) fn ci() -> bool { std::env::var_os("CI").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some() }

/// Keeps COM loaded for the rest of the process (Windows; elsewhere nothing). Libraries cache COM objects for the whole
/// process on whichever thread made them first: cpal's WASAPI device enumerator (in that thread's apartment), windows-rs'
/// WinRT activation factories. When that thread ends and its apartment was the last one, COM unloads the DLLs behind them
/// and the next use from another thread reads freed memory (an access violation, not an error). Holding the MTA open keeps
/// COM, and so those objects, alive. Called before the first use of either; cheap after the first call.
pub fn keep_com_loaded() {
    #[cfg(windows)]
    {
        static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        MTA.get_or_init(|| { let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() }; });
    }
}
