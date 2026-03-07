//! Benchmarks for interval conversion utilities.
//!
//! Run with: `cargo bench`
#![allow(missing_docs)]
use criterion::{criterion_group, criterion_main, Criterion};
use quack_rs::interval::{interval_to_micros, interval_to_micros_saturating, DuckInterval};
use std::hint::black_box;

fn bench_interval_to_micros(c: &mut Criterion) {
    let iv = DuckInterval {
        months: 3,
        days: 15,
        micros: 1_234_567,
    };
    c.bench_function("interval_to_micros", |b| {
        b.iter(|| interval_to_micros(black_box(iv)));
    });
}

fn bench_interval_to_micros_saturating(c: &mut Criterion) {
    let iv = DuckInterval {
        months: 3,
        days: 15,
        micros: 1_234_567,
    };
    c.bench_function("interval_to_micros_saturating", |b| {
        b.iter(|| interval_to_micros_saturating(black_box(iv)));
    });
}

fn bench_interval_overflow(c: &mut Criterion) {
    let iv = DuckInterval {
        months: i32::MAX,
        days: i32::MAX,
        micros: i64::MAX,
    };
    c.bench_function("interval_to_micros_overflow", |b| {
        b.iter(|| interval_to_micros(black_box(iv)));
    });
}

criterion_group!(
    benches,
    bench_interval_to_micros,
    bench_interval_to_micros_saturating,
    bench_interval_overflow
);
criterion_main!(benches);
