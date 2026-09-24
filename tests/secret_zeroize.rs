// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `SecretEntry` must not hand the allocator a buffer that still holds
//! secret material — including buffers it replaces, not just the ones it
//! drops.
//!
//! A counting global allocator inspects every buffer at `dealloc`, *before*
//! passing it to the system allocator (so the read is of live, owned memory),
//! and counts the buffers that still contain a marker. This is its own test
//! binary because the allocator is process-wide, and one `#[test]` runs every
//! scenario in sequence so nothing else allocates the marker concurrently.
//!
//! Audit F-V7: replacing a field with `with_field`, or the provider or scope
//! with `with_provider` / `with_scope`, freed the old value unzeroized
//! (`HashMap::insert` returns it; the assignment drops it).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use quack_rs::secrets::SecretEntry;

const MARK: &[u8] = b"TOPSECRET";

struct Inspecting;

static ARMED: AtomicBool = AtomicBool::new(false);
static HITS: AtomicUsize = AtomicUsize::new(0);
static FREED: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to `System` with its arguments unchanged;
// `dealloc` additionally reads the `layout.size()` bytes at `ptr`, which the
// caller guarantees are a live allocation of that layout until it is freed.
unsafe impl GlobalAlloc for Inspecting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::SeqCst) {
            FREED.fetch_add(1, Ordering::SeqCst);
            // SAFETY: `ptr` is live for `layout.size()` bytes until the
            // `System.dealloc` below.
            let bytes = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            if bytes.windows(MARK.len()).any(|w| w == MARK) {
                HITS.fetch_add(1, Ordering::SeqCst);
            }
        }
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Inspecting = Inspecting;

/// Runs `scenario` with inspection on; returns how many freed buffers still
/// held the marker, and how many buffers were freed at all.
fn freed_with_marker(scenario: impl FnOnce()) -> (usize, usize) {
    HITS.store(0, Ordering::SeqCst);
    FREED.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
    scenario();
    ARMED.store(false, Ordering::SeqCst);
    (HITS.load(Ordering::SeqCst), FREED.load(Ordering::SeqCst))
}

/// A heap `String` holding the marker (a literal would live in read-only
/// memory and never reach the allocator).
fn secret(suffix: &str) -> String {
    // Sized up front: a `realloc` frees its old buffer without `dealloc`
    // seeing it.
    let mut s = String::with_capacity(MARK.len() + suffix.len());
    s.push_str(std::str::from_utf8(MARK).expect("ASCII"));
    s.push_str(suffix);
    s
}

#[test]
fn no_freed_buffer_still_holds_secret_material() {
    // Control: the inspection sees a buffer that is freed with the marker.
    let (hits, _) = freed_with_marker(|| drop(secret("-control")));
    assert_eq!(hits, 1, "the allocator inspection works");

    let cases: [(&str, fn()); 5] = [
        ("drop with one field", || {
            drop(SecretEntry::new("n", "t").with_field("token", secret("-1")));
        }),
        ("overwrite a field", || {
            let entry = SecretEntry::new("n", "t")
                .with_field("token", secret("-old"))
                .with_field("token", String::from("rotated"));
            drop(entry);
        }),
        ("replace the scope", || {
            let entry = SecretEntry::new("n", "t")
                .with_scope(secret("-bucket"))
                .with_scope(String::from("s3://other/"));
            drop(entry);
        }),
        // The fourth audit's T10: only `len` bytes were zeroized, so a
        // secret truncated before it was stored kept its tail in the spare
        // capacity.
        ("store a truncated secret", || {
            // The whole marker sits past `len`, in the spare capacity.
            let mut value = String::with_capacity(4 + MARK.len());
            value.push_str("key=");
            value.push_str(std::str::from_utf8(MARK).expect("ASCII"));
            value.truncate(4);
            drop(SecretEntry::new("n", "t").with_field("token", value));
        }),
        ("replace the provider", || {
            let entry = SecretEntry::new("n", "t")
                .with_provider(secret("-provider"))
                .with_provider(String::from("config"));
            drop(entry);
        }),
    ];
    let mut leaked = Vec::new();
    for (name, scenario) in cases {
        let (hits, freed) = freed_with_marker(scenario);
        assert!(freed > 0, "{name}: the scenario freed its buffers");
        if hits != 0 {
            leaked.push(format!("{name}: {hits} buffer(s)"));
        }
    }
    assert!(
        leaked.is_empty(),
        "freed buffers still holding the secret: {leaked:?}"
    );
}
