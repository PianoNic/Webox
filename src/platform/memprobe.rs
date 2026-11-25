//! Heap accounting for memory work (`cargo build --release --features profile-mem`): a counting global allocator, so the
//! `WEBOX_PROBE=1` report and `mark` can tell Webox's own heap apart from what the OpenGL driver and the loaded libraries
//! hold in the same process. Without the feature everything here is a no-op and the system allocator is used directly.

#[cfg(feature = "profile-mem")]
mod imp {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    pub static LIVE: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK: AtomicUsize = AtomicUsize::new(0);
    struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = System.alloc(l); if !p.is_null() { add(l.size()) } p }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = System.alloc_zeroed(l); if !p.is_null() { add(l.size()) } p }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) { System.dealloc(p, l); LIVE.fetch_sub(l.size(), Relaxed); }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let q = System.realloc(p, l, n);
            if !q.is_null() { LIVE.fetch_sub(l.size(), Relaxed); add(n) }
            q
        }
    }
    fn add(n: usize) { let v = LIVE.fetch_add(n, Relaxed) + n; PEAK.fetch_max(v, Relaxed); }
    #[global_allocator]
    static A: Counting = Counting;
}

/// Live heap bytes and the peak so far (only with the `profile-mem` feature).
pub fn heap() -> Option<(usize, usize)> {
    #[cfg(feature = "profile-mem")]
    { use std::sync::atomic::Ordering::Relaxed; Some((imp::LIVE.load(Relaxed), imp::PEAK.load(Relaxed))) }
    #[cfg(not(feature = "profile-mem"))]
    None
}

/// With `profile-mem` and `WEBOX_PROBE` set: the live heap at a named point of the start (stderr).
pub fn mark(stage: &str) {
    if let (Some((live, peak)), true) = (heap(), std::env::var_os("WEBOX_PROBE").is_some()) {
        eprintln!("HEAP {stage}: live={:.1} MB peak={:.1} MB", live as f64 / 1048576.0, peak as f64 / 1048576.0);
    }
}
