// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Typed `COPY TO` state and a file kept open across callbacks (eighth
//! review: AUTHT-2, AUTHT-3, APIGAPS-8).
//!
//! The author-tables reviewer could write this writer only with
//! `Box::into_raw`, two hand-written `extern "C"` destructors, unchecked
//! `cast::<T>()` reads, `mem::forget` + `from_raw` and an `unsafe impl Send`
//! wrapper around the `FileHandle`. Here the state is typed and tag-checked,
//! the destructors are generated, and the handle is `Send`, so a
//! `Mutex<FileHandle>` in the global state needs no `unsafe impl`.
//!
//! Requires `duckdb-1-5` (declared on the `mod` line).

use std::ffi::CString;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use quack_rs::client_context::ClientContext;
use quack_rs::copy_function::{
    CopyBindInfo, CopyFinalizeInfo, CopyFunctionBuilder, CopyGlobalInitInfo, CopySinkInfo,
};
use quack_rs::data_chunk::DataChunk;
use quack_rs::file_system::{FileHandle, FileOpenOptions, FileSystem};

use super::Fixture;

/// The copy function's extra info: the line prefix, where each test counts
/// its drops, and whether the sink should fail.
struct Config {
    prefix: &'static str,
    bind_drops: &'static AtomicUsize,
    global_drops: &'static AtomicUsize,
    fail_sink: bool,
}

/// Bind data: the line prefix, copied from the extra info.
struct Bind {
    prefix: String,
    drops: &'static AtomicUsize,
}
impl Drop for Bind {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// Global state: the open file, written by every sink call.
struct Writer {
    file: Mutex<FileHandle<'static>>,
    lines: AtomicUsize,
    fail_sink: bool,
    drops: &'static AtomicUsize,
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

quack_rs::copy_bind_callback!(typed_bind, |info| {
    // SAFETY: `info` is the live bind info.
    let bind = unsafe { CopyBindInfo::new(info) };
    // SAFETY: the extra info is only set with `extra_info_typed`.
    let Some(config) = (unsafe { bind.extra_info_ref::<Config>() }) else {
        bind.set_error("extra info is not a Config");
        return;
    };
    // A wrong type is `None`, not a reinterpretation.
    // SAFETY: as above.
    if unsafe { bind.extra_info_ref::<Bind>() }.is_some() {
        bind.set_error("a Config read as a Bind");
        return;
    }
    bind.set_bind_data_typed(Bind {
        prefix: config.prefix.to_owned(),
        drops: config.bind_drops,
    });
});

quack_rs::copy_global_init_callback!(typed_init, |info| {
    // SAFETY: `info` is the live global-init info.
    let init = unsafe { CopyGlobalInitInfo::new(info) };
    // SAFETY: the extra info is only set with `extra_info_typed`.
    let Some(config) = (unsafe { init.extra_info_ref::<Config>() }) else {
        init.set_error("extra info is not a Config");
        return;
    };
    // SAFETY: the context is only used inside this callback.
    let ctx = unsafe { init.get_client_context() };
    let Some(fs) = FileSystem::from_client_context(&ctx) else {
        init.set_error("no file system");
        return;
    };
    // SAFETY: global-init callback.
    let path = match unsafe { init.get_file_path() } {
        Ok(path) => path,
        Err(e) => return init.set_error(&e.to_string()),
    };
    let Ok(path) = CString::new(path) else {
        return init.set_error("NUL in path");
    };
    let file = match fs.open(&path, &FileOpenOptions::write_create()) {
        Ok(file) => file,
        Err(e) => return init.set_error(&e.to_string()),
    };
    // SAFETY: the handle goes into this statement's global state, which
    // DuckDB destroys before the statement ends, while the connection and
    // database are open. The `FileSystem` wrapper and the context are
    // dropped at the end of this callback; the handle does not use them.
    let file: FileHandle<'static> = unsafe { FileHandle::from_raw(file.into_raw()) };
    init.set_global_state_typed(Writer {
        file: Mutex::new(file),
        lines: AtomicUsize::new(0),
        fail_sink: config.fail_sink,
        drops: config.global_drops,
    });
});

quack_rs::copy_sink_callback!(typed_sink, |info, chunk| {
    // SAFETY: `info` is the live sink info.
    let sink = unsafe { CopySinkInfo::new(info) };
    // SAFETY: bind data and global state are only set with the typed setters.
    let (Some(bind), Some(writer)) = (unsafe { sink.bind_data_ref::<Bind>() }, unsafe {
        sink.global_state_ref::<Writer>()
    }) else {
        sink.set_error("bind data or global state missing");
        return;
    };
    // SAFETY: as above.
    if unsafe { sink.bind_data_ref::<Writer>() }.is_some()
        || unsafe { sink.global_state_ref::<Bind>() }.is_some()
    {
        sink.set_error("a state read as the other type");
        return;
    }
    if writer.fail_sink {
        sink.set_error("sink failed on purpose");
        return;
    }
    // SAFETY: `chunk` is the live input chunk with one BIGINT column.
    let chunk = unsafe { DataChunk::from_raw(chunk) };
    // SAFETY: column 0 exists.
    let reader = unsafe { chunk.reader(0) };
    let mut text = String::new();
    for row in 0..chunk.size() {
        // SAFETY: `row` is in range.
        let value = unsafe { reader.read_i64(row) };
        let _ = writeln!(text, "{}{value}", bind.prefix);
    }
    let written = writer
        .file
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .write_all(text.as_bytes());
    if let Err(e) = written {
        sink.set_error(&e.to_string());
        return;
    }
    writer.lines.fetch_add(chunk.size(), Ordering::SeqCst);
});

quack_rs::copy_finalize_callback!(typed_finalize, |info| {
    // SAFETY: `info` is the live finalize info.
    let finalize = unsafe { CopyFinalizeInfo::new(info) };
    // SAFETY: the typed setters are the only ones used.
    let (Some(_bind), Some(writer)) = (unsafe { finalize.bind_data_ref::<Bind>() }, unsafe {
        finalize.global_state_ref::<Writer>()
    }) else {
        finalize.set_error("state missing in finalize");
        return;
    };
    // SAFETY: as above.
    if unsafe { finalize.extra_info_ref::<Config>() }.is_none() {
        finalize.set_error("extra info missing in finalize");
        return;
    }
    let file = writer
        .file
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Err(e) = file.sync().and_then(|()| file.close()) {
        finalize.set_error(&e.to_string());
    }
});

fn register(fx: &Fixture, name: &str, config: Config) {
    // SAFETY: `con` is open; every callback matches its signature.
    unsafe {
        CopyFunctionBuilder::try_new(name)
            .expect("name")
            .bind(typed_bind)
            .global_init(typed_init)
            .sink(typed_sink)
            .finalize(typed_finalize)
            .extra_info_typed(config)
            .register(fx.con())
    }
    .unwrap_or_else(|e| panic!("register {name}: {e}"));
}

#[test]
fn a_copy_writer_keeps_typed_state_and_a_file_across_its_callbacks() {
    static BIND_DROPS: AtomicUsize = AtomicUsize::new(0);
    static GLOBAL_DROPS: AtomicUsize = AtomicUsize::new(0);
    let fx = Fixture::open();
    register(
        &fx,
        "typed_lines",
        Config {
            prefix: "n=",
            bind_drops: &BIND_DROPS,
            global_drops: &GLOBAL_DROPS,
            fail_sink: false,
        },
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out.lines");
    // Enough rows for several sink calls, and threads to run them on.
    fx.query("SET threads = 4");
    fx.query(&format!(
        "COPY (SELECT range AS x FROM range(5000)) TO '{}' (FORMAT typed_lines)",
        path.display()
    ));
    let written = std::fs::read_to_string(&path).expect("the file was written");
    let mut values: Vec<i64> = written
        .lines()
        .map(|line| {
            line.strip_prefix("n=")
                .unwrap_or_else(|| panic!("line without prefix: {line:?}"))
                .parse()
                .expect("number")
        })
        .collect();
    values.sort_unstable();
    assert_eq!(values, (0..5000).collect::<Vec<_>>());
    assert_eq!(
        BIND_DROPS.load(Ordering::SeqCst),
        1,
        "bind data dropped once"
    );
    assert_eq!(
        GLOBAL_DROPS.load(Ordering::SeqCst),
        1,
        "global state dropped once"
    );
}

/// The generated destructors run after a callback fails too: `DuckDB`
/// hands the pointers to its own state before it checks for the error.
#[test]
fn typed_copy_state_is_dropped_once_after_a_sink_error() {
    static BIND_DROPS: AtomicUsize = AtomicUsize::new(0);
    static GLOBAL_DROPS: AtomicUsize = AtomicUsize::new(0);
    let fx = Fixture::open();
    register(
        &fx,
        "typed_fail",
        Config {
            prefix: "",
            bind_drops: &BIND_DROPS,
            global_drops: &GLOBAL_DROPS,
            fail_sink: true,
        },
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let sql = format!(
        "COPY (SELECT 1::BIGINT AS x) TO '{}' (FORMAT typed_fail)",
        dir.path().join("out.fail").display()
    );
    // SAFETY: `con` is open.
    let err = unsafe { quack_rs::query::query(fx.con(), &sql) }
        .map(drop)
        .expect_err("the sink fails");
    assert!(err.as_str().contains("sink failed on purpose"), "{err}");
    assert_eq!(BIND_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(GLOBAL_DROPS.load(Ordering::SeqCst), 1);
    // The connection is still usable.
    fx.query("SELECT 1");
}

/// AUTHT-2: a `FileHandle` is `Send`, so it can be opened on one thread and
/// written, read, closed and dropped on another.
#[test]
fn a_file_handle_is_used_and_dropped_on_another_thread() {
    let fx = Fixture::open();
    // SAFETY: the fixture's connection is open for the whole test.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("context");
    let fs = FileSystem::from_client_context(&ctx).expect("file system");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("thread.bin");
    let c_path = CString::new(path.to_str().expect("utf-8 path")).expect("no NUL");
    let opener = std::thread::current().id();

    let handle = fs
        .open(&c_path, &FileOpenOptions::write_create())
        .expect("open");
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                assert_ne!(std::thread::current().id(), opener);
                handle
                    .write_all(b"written on another thread")
                    .expect("write");
                handle.sync().expect("sync");
                handle.close().expect("close");
                drop(handle);
            })
            .join()
            .expect("writer thread");
    });
    assert_eq!(
        std::fs::read(&path).expect("read back"),
        b"written on another thread"
    );

    let handle = fs
        .open(&c_path, &FileOpenOptions::read_only())
        .expect("reopen");
    let read = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let mut buf = Vec::new();
                handle.read_to_end(&mut buf).expect("read");
                buf
            })
            .join()
            .expect("reader thread")
    });
    assert_eq!(read, b"written on another thread");
}

/// AUTHT-2: a `ClientContext` is `Send`; one moved to another thread reads
/// the same connection there, and is destroyed there.
#[test]
fn a_client_context_is_used_and_dropped_on_another_thread() {
    let fx = Fixture::open();
    // SAFETY: the fixture's connection outlives the thread, which is joined.
    let ctx = unsafe { ClientContext::from_connection(fx.con()) }.expect("context");
    let id = ctx.connection_id();
    let (moved_id, threads) = std::thread::spawn(move || {
        let seen = (ctx.connection_id(), ctx.config_option(c"threads"));
        drop(ctx);
        seen
    })
    .join()
    .expect("context thread");
    assert_eq!(moved_id, id);
    assert!(threads.is_some_and(|t| t.parse::<u64>().is_ok()));
}
