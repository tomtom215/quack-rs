// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! `SecretEntry`'s zeroize-on-drop claim, checked against the process's real
//! memory rather than through an allocator hook.
//!
//! `tests/secret_zeroize.rs` inspects every buffer at `dealloc`; that proves
//! the buffers were wiped *before* they were handed back, but not that nothing
//! else still holds the bytes. This test searches every readable, writable
//! mapping of the live process (`/proc/self/maps` + `/proc/self/mem`) for a
//! runtime-generated marker, before and after the drop.
//!
//! The marker is never present in this file's data as a contiguous string: it
//! is generated at run time, and the scan compares each window against it
//! through an XOR key, so the needle itself is not a second copy to find.
//!
//! Linux only. One `#[test]` runs every scenario in sequence, so no other
//! thread of the test harness allocates the marker.

#![cfg(target_os = "linux")]
// A test harness: the casts narrow a clock reading and a region size to fit.
#![allow(clippy::cast_possible_truncation)]

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;

use quack_rs::secrets::SecretEntry;

const LEN: usize = 48;

/// A marker that exists nowhere in the binary: derived from the clock and the
/// address of a stack variable.
fn marker() -> [u8; LEN] {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
        ^ (std::ptr::addr_of!(seed_anchor) as usize as u64).rotate_left(17);
    let mut x = seed | 1;
    let mut out = [0u8; LEN];
    for b in &mut out {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = b'a' + (x % 26) as u8;
    }
    out
}

#[allow(non_upper_case_globals)]
static seed_anchor: u8 = 0;

/// A heap `String` holding the marker, built in place so no temporary copy of
/// the whole marker is made.
fn secret(m: &[u8; LEN]) -> String {
    let mut s = String::with_capacity(LEN);
    for &b in m {
        s.push(char::from(b));
    }
    s
}

/// The writable mappings worth searching.
fn regions() -> Vec<(u64, u64)> {
    let maps = std::fs::read_to_string("/proc/self/maps").expect("maps");
    maps.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let range = it.next()?;
            let perms = it.next()?;
            if !perms.starts_with("rw") {
                return None;
            }
            let (a, b) = range.split_once('-')?;
            Some((
                u64::from_str_radix(a, 16).ok()?,
                u64::from_str_radix(b, 16).ok()?,
            ))
        })
        .collect()
}

/// How many times the 24-byte tail of the marker occurs in writable memory.
/// (The first bytes of a freed block are overwritten by the allocator's own
/// free-list pointers, so the tail is what a leak would show.)
fn occurrences(m: &[u8; LEN]) -> usize {
    const KEY: u8 = 0x5a;
    let mut enc = [0u8; 24];
    for (e, b) in enc.iter_mut().zip(&m[LEN - 24..]) {
        *e = b ^ KEY;
    }
    let mem = File::open("/proc/self/mem").expect("mem");
    let mut count = 0;
    let mut buf = vec![0u8; 1 << 20];
    for (start, end) in regions() {
        let mut at = start;
        while at < end {
            let want = ((end - at) as usize).min(buf.len());
            let Ok(n) = mem.read_at(&mut buf[..want], at) else {
                break; // unreadable guard / vsyscall page
            };
            if n == 0 {
                break;
            }
            // Overlap the chunk boundary by one needle length.
            for w in buf[..n].windows(24) {
                if w.iter().zip(&enc).all(|(a, e)| a ^ KEY == *e) {
                    count += 1;
                }
            }
            at += n.saturating_sub(23).max(1) as u64;
        }
    }
    // `buf` itself never holds the marker unless memory did.
    let _ = File::open("/proc/self/stat").map(|mut f| f.read_to_end(&mut Vec::new()));
    count
}

/// Overwrites the small free blocks the allocator would hand out next, so a
/// copy left by an earlier step cannot be mistaken for one left by a later
/// step (a freed block is reused, with the same content, by the next
/// allocation of its size).
fn scrub() {
    let blocks: Vec<Vec<u8>> = (0..8192).map(|i| vec![b'Z'; 32 + (i % 64)]).collect();
    std::hint::black_box(&blocks);
}

#[test]
fn a_dropped_secret_entry_leaves_no_copy_in_process_memory() {
    let m = marker();

    // Control: the scan sees live and freed-but-unwiped copies, so a zero
    // below means something.
    let plain = secret(&m);
    let live = occurrences(&m);
    assert!(live >= 1, "the scan must see a live copy ({live})");
    drop(plain);
    let freed = occurrences(&m);
    assert!(
        freed >= 1,
        "control: a dropped plain String leaves its bytes in freed memory, so the scan can \
         detect a leak ({freed})"
    );
    scrub();
    let baseline = occurrences(&m);
    // `baseline` counts what this test itself holds (the marker array `m` on
    // its stack); every assertion below is relative to it.

    // Scenario 1: one field, dropped.
    let entry = SecretEntry::new("n", "t").with_field("token", secret(&m));
    assert!(occurrences(&m) > baseline, "a live entry holds the secret");
    drop(entry);
    assert_eq!(
        occurrences(&m),
        baseline,
        "a dropped SecretEntry left a copy of its secret"
    );

    // Scenario 2: a clone is a second copy; both go.
    let entry = SecretEntry::new("n", "t").with_field("token", secret(&m));
    let copy = entry.clone();
    assert!(
        occurrences(&m) >= baseline + 2,
        "the clone holds its own copy"
    );
    drop(entry);
    drop(copy);
    assert_eq!(occurrences(&m), baseline, "a dropped clone left a copy");

    // Scenario 3: a value replaced by `with_field`, and a grown map.
    let mut entry = SecretEntry::new("n", "t").with_field("token", secret(&m));
    for i in 0..200 {
        entry = entry.with_field(format!("k{i}"), format!("v{i}"));
    }
    entry = entry.with_field("token", String::from("rotated"));
    assert_eq!(occurrences(&m), baseline, "a replaced value left a copy");
    drop(entry);
}
