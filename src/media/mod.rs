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

/// `gst::init()`, then a second try at the plugins that GStreamer's registry cache marks as broken (Linux). WebKit's
/// sandboxed web process shares the user's registry (`~/.cache/gstreamer-1.0`) but sees only WebKit's own libraries, so
/// when it scans first (after an install or update) it blacklists every plugin outside them, openh264enc and x264enc
/// among them. GStreamer never retries a blacklisted file that hasn't changed, and Nix store files never change.
/// Dropping those entries and rescanning (in the plugin scanner subprocess, as usual) brings back the ones that load here.
#[cfg(target_os = "linux")]
pub(crate) fn gst_init() -> Result<(), String> {
    use gstreamer as gst;
    gst::init().map_err(|e| format!("GStreamer: {e}"))?;
    static RETRY: std::sync::Once = std::sync::Once::new();
    RETRY.call_once(|| {
        let reg = gst::Registry::get();
        let blacklisted = || reg.plugins().into_iter().filter(|p| p.plugin_flags().contains(gst::PluginFlags::BLACKLISTED)).collect::<Vec<_>>();
        let broken = blacklisted();
        if broken.is_empty() { return; }
        for p in &broken { reg.remove_plugin(p); }
        match gst::update_registry() {
            Ok(()) => eprintln!("GStreamer: retried {} blacklisted plugins, {} still fail to load", broken.len(), blacklisted().len()),
            Err(e) => eprintln!("GStreamer: rescan of {} blacklisted plugins failed: {e}", broken.len()),
        }
    });
    Ok(())
}

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
