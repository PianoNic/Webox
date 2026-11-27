//! The memory / CPU floor of an eframe (glow) window on this machine: the same window options as Webox, one label, nothing
//! else. Compare its private bytes with Webox's (`--sample`, `WEBOX_PROBE=1`) to tell the OpenGL driver's share from Webox's own.
//! `cargo run --release --example blank_window`
use eframe::egui;

fn main() -> eframe::Result<()> {
    let vp = egui::ViewportBuilder::default().with_title("blank").with_inner_size([1280.0, 800.0]).with_decorations(false);
    eframe::run_native("blank", eframe::NativeOptions { viewport: vp, ..Default::default() }, Box::new(|_| Ok(Box::new(Blank))))
}

struct Blank;
impl eframe::App for Blank {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| ui.label("blank"));
    }
}
