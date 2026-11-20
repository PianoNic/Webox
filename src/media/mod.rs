//! Sound and video: audio devices and the capture / playback pipeline of calls (`audio`), Smart Audio noise removal (`denoise`),
//! cameras and the settings preview (`camera`), the ringtones, message sounds and sound preferences (`sounds`), playback of shared
//! videos with the platform's media framework (`video`).
pub mod audio;
pub mod camera;
pub mod denoise;
pub mod mic;
pub mod sounds;
pub mod video;
