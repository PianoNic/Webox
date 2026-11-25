//! Built-in resources: the Momentum icons and emoji (`icons`), the Webex client files embedded at build time (`cisco_assets`),
//! decoded profile pictures and message images (`photos`) and the translations (`i18n`).
pub mod icons;
pub mod cisco_assets;
pub mod photos;
pub mod font_file; // system font files without heap copies (mapped on Windows)
pub mod i18n;
