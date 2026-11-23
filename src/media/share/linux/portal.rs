//! The xdg-desktop-portal ScreenCast interface (ashpd): the desktop's own dialog picks a screen or a window, the portal
//! hands back a PipeWire node and a file descriptor to the PipeWire remote that carries it. The portal session lives on the
//! D-Bus runtime until the share ends (closing it stops the cast and the desktop's "sharing" indicator).
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;
use std::os::fd::OwnedFd;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// A started cast. Dropping it closes the portal session.
pub(super) struct Cast { pub fd: OwnedFd, pub node: u32, pub size: Option<(u32, u32)>, stop: Arc<tokio::sync::Notify> }

impl Drop for Cast { fn drop(&mut self) { self.stop.notify_one(); } }

/// The portal offers ScreenCast with screens and windows (checked once; Flatpak, GNOME, KDE, wlroots portals).
pub(super) fn available() -> bool {
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| {
        crate::platform::linux::block(2000, async {
            let p = Screencast::new().await.ok()?;
            p.available_source_types().await.ok().map(|t| t.contains(SourceType::Monitor))
        }).flatten().unwrap_or(false)
    })
}

/// Opens the portal dialog for screens (`windows` false) or windows and waits for the user's choice. Ok(None): cancelled,
/// in the dialog or through `stopped`.
pub(super) fn open(windows: bool, cursor: bool, stopped: &dyn Fn() -> bool) -> Result<Option<Cast>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    crate::platform::linux::rt().spawn(async move {
        let r: Result<_, ashpd::Error> = async {
            let proxy = Screencast::new().await?;
            let session = proxy.create_session().await?;
            let modes = proxy.available_cursor_modes().await.unwrap_or_default();
            let mode = if cursor && modes.contains(CursorMode::Embedded) { CursorMode::Embedded } else { CursorMode::Hidden };
            let types = if windows { SourceType::Window.into() } else { SourceType::Monitor.into() };
            proxy.select_sources(&session, mode, types, false, None, PersistMode::DoNot).await?;
            let resp = proxy.start(&session, None).await?.response()?;
            let s = resp.streams().first().ok_or(ashpd::Error::NoResponse)?;
            let (node, size) = (s.pipe_wire_node_id(), s.size().map(|(w, h)| (w.max(0) as u32, h.max(0) as u32)));
            let fd = proxy.open_pipe_wire_remote(&session).await?;
            Ok((session, fd, node, size))
        }.await;
        match r {
            Ok((session, fd, node, size)) => {
                let _ = tx.send(Ok((fd, node, size)));
                stop2.notified().await;
                let _ = session.close().await;
            }
            Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => { let _ = tx.send(Err(None)); }
            Err(e) => { let _ = tx.send(Err(Some(format!("desktop portal: {e}")))); }
        }
    });
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok((fd, node, size))) => return Ok(Some(Cast { fd, node, size, stop })),
            Ok(Err(None)) => return Ok(None),
            Ok(Err(Some(e))) => return Err(e),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => if stopped() { stop.notify_one(); return Ok(None); },
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("desktop portal: no answer".into()),
        }
    }
}
