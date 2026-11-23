//! The xdg-desktop-portal ScreenCast interface (ashpd): the desktop's own dialog picks a screen or a window, the portal
//! hands back a PipeWire node and a file descriptor to the PipeWire remote that carries it. The portal session lives on the
//! D-Bus runtime until the share ends (closing it stops the cast and the desktop's "sharing" indicator).
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A started cast. Dropping it closes the portal session.
pub(super) struct Cast { pub fd: OwnedFd, pub node: u32, pub size: Option<(u32, u32)>, stop: Arc<tokio::sync::Notify> }

impl Drop for Cast { fn drop(&mut self) { self.stop.notify_one(); } }

/// The last answer of the availability check, when it came, and whether one is running.
struct Check { known: Option<bool>, at: Option<Instant>, running: bool }
static CHECK: Mutex<Check> = Mutex::new(Check { known: None, at: None, running: false });
/// A "no" is asked again after this long (the portal may start after Webox, D-Bus activation can be slow).
const RECHECK: Duration = Duration::from_secs(5);

/// Whether the portal offers ScreenCast with screens (Flatpak, GNOME, KDE, wlroots portals). Never blocks: the check runs on
/// the D-Bus runtime and this answers None until its first result. A "yes" is kept, a "no" is checked again after `RECHECK`.
pub(super) fn available() -> Option<bool> {
    let mut c = CHECK.lock().unwrap_or_else(|e| e.into_inner());
    if c.known == Some(true) { return c.known; }
    if !c.running && c.at.map_or(true, |t| t.elapsed() >= RECHECK) {
        c.running = true;
        crate::platform::linux::rt().spawn(async {
            let ok = tokio::time::timeout(Duration::from_secs(5), async {
                let p = Screencast::new().await.ok()?;
                p.available_source_types().await.ok().map(|t| t.contains(SourceType::Monitor))
            }).await.ok().flatten().unwrap_or(false);
            let mut c = CHECK.lock().unwrap_or_else(|e| e.into_inner());
            *c = Check { known: Some(ok), at: Some(Instant::now()), running: false };
        });
    }
    c.known
}

/// Opens the portal dialog for screens (`windows` false) or windows and waits for the user's choice. Ok(None): cancelled,
/// in the dialog or through `stopped` (which closes the session, and with it the desktop's dialog).
pub(super) fn open(windows: bool, cursor: bool, stopped: &dyn Fn() -> bool) -> Result<Option<Cast>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    crate::platform::linux::rt().spawn(async move {
        let r: Result<_, ashpd::Error> = async {
            let proxy = Screencast::new().await?;
            let session = proxy.create_session().await?;
            let picked = tokio::select! {
                r = async {
                    let modes = proxy.available_cursor_modes().await.unwrap_or_default();
                    let mode = if cursor && modes.contains(CursorMode::Embedded) { CursorMode::Embedded } else { CursorMode::Hidden };
                    let types = if windows { SourceType::Window.into() } else { SourceType::Monitor.into() };
                    proxy.select_sources(&session, mode, types, false, None, PersistMode::DoNot).await?;
                    let resp = proxy.start(&session, None).await?.response()?;
                    let s = resp.streams().first().ok_or(ashpd::Error::NoResponse)?;
                    let (node, size) = (s.pipe_wire_node_id(), s.size().map(|(w, h)| (w.max(0) as u32, h.max(0) as u32)));
                    let fd = proxy.open_pipe_wire_remote(&session).await?;
                    Ok::<_, ashpd::Error>((fd, node, size))
                } => Some(r),
                _ = stop2.notified() => None, // the share was cancelled while the dialog is up
            };
            Ok((session, picked))
        }.await;
        match r {
            Ok((session, Some(Ok((fd, node, size))))) => {
                let _ = tx.send(Ok((fd, node, size)));
                stop2.notified().await;
                let _ = session.close().await;
            }
            Ok((session, None)) => { let _ = session.close().await; }
            Ok((session, Some(Err(e)))) => { let _ = session.close().await; let _ = tx.send(Err(err_text(e))); }
            Err(e) => { let _ = tx.send(Err(err_text(e))); }
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

/// None: the user cancelled the dialog.
fn err_text(e: ashpd::Error) -> Option<String> {
    match e { ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => None, e => Some(format!("desktop portal: {e}")) }
}
