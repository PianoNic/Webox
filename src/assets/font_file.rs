//! Font files for egui, without copies on the heap.
//!
//! egui copies an owned font (`FontData::from_owned`) once more for every set of fonts it builds (epaint's
//! `ab_glyph_font_from_font_data` clones the bytes), so the ~32 MB of system fonts Webox uses (Segoe UI, the emoji font, the
//! CJK / Indic / symbol fallbacks) cost twice that in private memory. A `'static` slice is used in place instead. On Windows
//! the file is memory-mapped read-only: its pages belong to the file (shared with every other program using the font, and only
//! the parts ab_glyph actually reads are ever paged in), not to Webox's private bytes. Elsewhere the bytes are read once and
//! kept for the life of the process. Either way each file is loaded only once, however often the fonts are set.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static FILES: Mutex<Option<HashMap<PathBuf, &'static [u8]>>> = Mutex::new(None);

/// The contents of the font file at `path`, loaded once per process; `None` if it can't be read (or is empty).
pub fn load(path: &Path) -> Option<&'static [u8]> {
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let files = files.get_or_insert_with(HashMap::new);
    if let Some(b) = files.get(path) { return Some(b); }
    let bytes = map(path).or_else(|| std::fs::read(path).ok().filter(|b| !b.is_empty()).map(|b| &*Box::leak(b.into_boxed_slice())))?;
    files.insert(path.to_path_buf(), bytes);
    Some(bytes)
}

#[cfg(windows)]
fn map(path: &Path) -> Option<&'static [u8]> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, FILE_MAP_READ, PAGE_READONLY};
    let file = std::fs::File::open(path).ok()?;
    let len = usize::try_from(file.metadata().ok()?.len()).ok().filter(|&l| l > 0)?;
    // SAFETY: a read-only mapping of the whole file; the view is never unmapped, so the slice stays valid for the process.
    // The mapping handle may be closed once the view exists (the view keeps the section alive). Fonts in the system font
    // folder are not rewritten while in use (Windows itself keeps them open).
    unsafe {
        let section = CreateFileMappingW(HANDLE(file.as_raw_handle()), None, PAGE_READONLY, 0, 0, windows::core::PCWSTR::null()).ok()?;
        let view = MapViewOfFile(section, FILE_MAP_READ, 0, 0, 0);
        let _ = CloseHandle(section);
        if view.Value.is_null() { return None; }
        Some(std::slice::from_raw_parts(view.Value as *const u8, len))
    }
}

#[cfg(not(windows))]
fn map(_: &Path) -> Option<&'static [u8]> { None }

#[cfg(test)]
mod tests {
    #[test]
    fn same_file_loads_once_and_reads_whole() {
        let d = std::env::temp_dir().join(format!("webox-font-file-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.ttf");
        std::fs::write(&f, b"\0\x01\0\0abcdef").unwrap();
        let a = super::load(&f).unwrap();
        assert_eq!(a, b"\0\x01\0\0abcdef");
        assert_eq!(a.as_ptr(), super::load(&f).unwrap().as_ptr());
        assert!(super::load(&d.join("missing.ttf")).is_none());
    }
}
