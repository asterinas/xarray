#![feature(test)]

extern crate test;

use std::sync::Arc;

use test::{black_box, Bencher};
use xarray::XArray;

const DENSE_LEN: u64 = 100_000;
const SPARSE_LEN: u64 = 200_000;
const RANDOM_QUERIES: usize = 50_000;
const COW_WRITE_COUNT: u64 = 20_000;

fn build_dense(len: u64) -> XArray<Arc<u64>> {
    let mut xa: XArray<Arc<u64>> = XArray::new();
    for i in 0..len {
        xa.store(i, Arc::new(i));
    }
    xa
}

fn random_indices(len: u64, count: usize) -> Vec<u64> {
    let mut seed = 0x1234_5678_9abc_def0_u64;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        seed ^= seed << 7;
        seed ^= seed >> 9;
        seed ^= seed << 8;
        out.push(seed % len);
    }
    out
}

#[bench]
fn bench_store_dense(b: &mut Bencher) {
    b.iter(|| {
        let mut xa: XArray<Arc<u64>> = XArray::new();
        for i in 0..DENSE_LEN {
            xa.store(i, Arc::new(i));
        }
        black_box(xa);
    });
}

#[bench]
fn bench_cursor_load_dense(b: &mut Bencher) {
    let xa = build_dense(DENSE_LEN);

    b.iter(|| {
        let mut cursor = xa.cursor(0);
        let mut sum = 0_u64;
        for _ in 0..DENSE_LEN {
            if let Some(v) = cursor.load() {
                sum = sum.wrapping_add(*v.as_ref());
            }
            cursor.next();
        }
        black_box(sum);
    });
}

#[bench]
fn bench_load_random_dense(b: &mut Bencher) {
    let xa = build_dense(DENSE_LEN);
    let indices = random_indices(DENSE_LEN, RANDOM_QUERIES);

    b.iter(|| {
        let mut sum = 0_u64;
        for &idx in &indices {
            if let Some(v) = xa.load(idx) {
                sum = sum.wrapping_add(*v.as_ref());
            }
        }
        black_box(sum);
    });
}

#[bench]
fn bench_range_sparse_even(b: &mut Bencher) {
    let mut xa: XArray<Arc<u64>> = XArray::new();
    for i in 0..SPARSE_LEN {
        if i % 2 == 0 {
            xa.store(i, Arc::new(i));
        }
    }

    b.iter(|| {
        let mut sum = 0_u64;
        let mut cnt = 0_u64;
        for (index, item) in xa.range(0..SPARSE_LEN) {
            sum = sum.wrapping_add(index).wrapping_add(*item.as_ref());
            cnt += 1;
        }
        black_box((sum, cnt));
    });
}

#[bench]
fn bench_cow_clone_then_overwrite(b: &mut Bencher) {
    let xa = build_dense(DENSE_LEN);

    b.iter(|| {
        let mut cloned = xa.clone();
        for i in 0..COW_WRITE_COUNT {
            let idx = (i * 7) % DENSE_LEN;
            cloned.store(idx, Arc::new(i));
        }
        black_box(cloned.load(0));
    });
}
