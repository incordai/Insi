//! Measure live Rust heap bytes independently of RSS high-water marks.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Wrap System without changing pointer or layout semantics.
struct Tracking;
static LIVE: AtomicUsize = AtomicUsize::new(0);

// Track successful allocations and their matching releases.
unsafe impl GlobalAlloc for Tracking {
    // Record allocation size only after the underlying allocator succeeds.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the caller's valid layout directly to System.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    // Release the same pointer and layout that System originally allocated.
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: pointer and layout are supplied by the matching allocation.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

// Hold workload constant while changing whether ownership is forgotten or dropped.
fn main() {
    let before = LIVE.load(Ordering::Relaxed);
    for _ in 0..2048 {
        let value = Box::new([0u8; 4096]);
        std::hint::black_box(&value);
        #[cfg(leaking)]
        std::mem::forget(value);
        #[cfg(not(leaking))]
        drop(value);
    }
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    println!("{{\"retained_bytes\":{retained}}}");
}
