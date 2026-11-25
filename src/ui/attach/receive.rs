//! Receiving files: download, save as, the file frame and the file menu.
use super::*;

impl App {
    /// "Open" / "Download" / "Show in folder": downloads the file behind `key` into the download folder (unless it
    /// already is there from this session), then opens it / shows it.
    pub(crate) fn file_fetch(&mut self, key: &str, name: &str, then: Then, now: f64) {
        if let Some(p) = local_path(key) {
            // A file of this computer (pending upload / sample data): no download.
            match then {
                Then::Nothing => { let dest = fs_::unique_path(&fs_::download_dir(&self.settings), name); self.file_write_local(key, name, &p, &dest, now); }
                t => cloud_finish(&p, t),
            }
            return;
        }
        if let Some(d) = fs_::download(key).filter(|d| d.phase == Phase::Done && d.path.exists()) {
            if then == Then::Nothing { self.toast(format!("{} – {}", d.name, tr("Download complete")), now); } else { cloud_finish(&d.path, then); }
            return;
        }
        if fs_::download(key).is_some_and(|d| matches!(d.phase, Phase::Preparing | Phase::Uploading)) { return; } // already on its way
        let dir = fs_::download_dir(&self.settings);
        let _ = std::fs::create_dir_all(&dir);
        let path = fs_::unique_path(&dir, name);
        self.file_fetch_to(key, name, &path, then, now);
    }

    /// Downloads into exactly `path` (the save dialog's choice).
    pub(crate) fn file_fetch_to(&mut self, key: &str, name: &str, path: &Path, then: Then, now: f64) {
        if let Some(p) = local_path(key) { self.file_write_local(key, name, &p, path, now); return; }
        fs_::download_begin(key, name, path, then);
        ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).remove(key);
        match &self.client {
            Some(c) => c.req(Req::DownloadFile { key: key.to_string(), path: path.to_path_buf(), then }),
            None => {
                // Sample data: the picture we show, or a small stand-in text file.
                let bytes = self.photos.image_bytes(&format!("full:{key}")).or_else(|| self.photos.image_bytes(key)).map(|b| b.to_vec())
                    .unwrap_or_else(|| format!("{name}\r\n\r\nSample file of the offline sample data (--sample).\r\n").into_bytes());
                match std::fs::write(path, &bytes) {
                    Ok(()) => { fs_::download_progress(key, bytes.len() as u64, bytes.len() as u64); fs_::download_phase(key, Phase::Done); cloud_finish(path, then); }
                    Err(e) => fs_::download_phase(key, Phase::Failed(e.to_string())),
                }
            }
        }
        let _ = now;
    }

    pub(crate) fn file_write_local(&mut self, key: &str, name: &str, src: &Path, dest: &Path, now: f64) {
        fs_::download_begin(key, name, dest, Then::Nothing);
        match std::fs::copy(src, dest) {
            Ok(n) => { fs_::download_progress(key, n, n); fs_::download_phase(key, Phase::Done); }
            Err(e) => { fs_::download_phase(key, Phase::Failed(e.to_string())); self.toast(tr("Download failed"), now); }
        }
    }

    /// "Save as": the save dialog (pre-filled with the name, in the download folder); the download starts when a path is chosen.
    pub(crate) fn file_save_as(&mut self, key: &str, name: &str) {
        if self.shot.is_some() { return; }
        fs_::save_as_async(key, name, &fs_::download_dir(&self.settings));
    }

    /// Once per frame (composer, content tab, viewer): start the downloads of finished save dialogs, collect the staged files of
    /// finished open dialogs, announce finished downloads ("Download complete" / "Download failed").
    pub(crate) fn files_frame(&mut self, ctx: &egui::Context, now: f64) {
        fs_::set_ctx(ctx);
        for (key, path) in fs_::take_saves() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            self.file_fetch_to(&key, &name, &path, Then::Nothing, now);
        }
        let mut toasts = vec![];
        {
            let mut guard = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
            let said = guard.get_or_insert_with(Default::default);
            for d in fs_::downloads() {
                if said.contains(&d.key) { continue; }
                match &d.phase {
                    Phase::Done if d.then == Then::Nothing => { said.insert(d.key.clone()); toasts.push(format!("{} – {}", d.name, tr("Download complete"))); }
                    Phase::Done => { said.insert(d.key.clone()); }
                    // the inline player shows its own message on the card
                    Phase::Failed(_) if d.then == Then::Play => { said.insert(d.key.clone()); }
                    Phase::Failed(_) => { said.insert(d.key.clone()); toasts.push(format!("{} – {}", d.name, tr("Download failed"))); }
                    _ => {}
                }
            }
        }
        for t in toasts { self.toast(t, now); }
    }

    /// The file actions as menu entries (right click on a file card / content row).
    pub(crate) fn file_menu_entries(&self, key: &str, show_in_chat: bool) -> Vec<super::super::menu::Entry> {
        use super::super::menu::{Entry, Item};
        let done = fs_::download(key).is_some_and(|d| d.phase == Phase::Done && d.path.exists()) || local_path(key).is_some();
        let mut v = vec![Item::new("file-open", tr("Open")).icon("open-pages-regular").entry(), Item::new("file-download", tr("Download")).icon("download-regular").entry(),
            Item::new("file-save", tr("Save as")).icon("save-regular").entry()];
        if done { v.push(Item::new("file-folder", tr("Show in folder")).icon("open-in-folder-regular").entry()); }
        if show_in_chat { v.push(Entry::Sep); v.push(Item::new("file-chat", tr("Show in conversation")).icon("chat-regular").entry()); }
        v
    }

    /// Runs a file menu action; returns false for ids that are not file actions.
    pub(crate) fn file_menu_do(&mut self, act: &str, key: &str, name: &str, now: f64) -> bool {
        match act {
            "file-open" => self.file_fetch(key, name, Then::Open, now),
            "file-download" => self.file_fetch(key, name, Then::Nothing, now),
            "file-save" => self.file_save_as(key, name),
            "file-folder" => self.file_fetch(key, name, Then::ShowInFolder, now),
            _ => return false,
        }
        true
    }
}
