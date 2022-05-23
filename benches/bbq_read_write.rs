use std::thread;

use barbequeue as bbq;
use bbq::{QueueReader, QueueWriter, ReadSlice, SpscQueue, WriteSlice};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn do_some_reading(buf: &[u8], times: usize) -> u64 {
    let mut s = 0;
    for i in 0..times {
        for b in buf {
            s += (*b - b'0') as u64 ^ i as u64;
        }
    }
    s
}

fn test_read_write<S: SpscQueue>(buf_size: usize, iter: usize, read_times: usize) -> u64 {
    let (mut reader, mut writer) = S::with_len(buf_size);
    let write_thread = thread::spawn(move || {
        for i in 0..iter {
            let mut buf = loop {
                if let Some(b) = writer.reserve(11) {
                    break b;
                }
            };
            buf.as_mut().copy_from_slice(b"12345678901");
            buf.commit_write(11);
        }
    });

    let read_thread = thread::spawn(move || {
        let mut sum = 0;
        for i in 0..iter {
            let buf = loop {
                let r = reader.readable();
                if r.as_ref().len() >= 11 {
                    break r;
                }
            };
            sum += do_some_reading(&buf.as_ref()[0..11], read_times);
            assert_eq!(&buf.as_ref()[0..11], b"12345678901");
            buf.commit_read(11);
        }
        sum
    });

    write_thread.join().unwrap();
    read_thread.join().unwrap()
}

fn bench_locking_bbq(c: &mut Criterion) {
    c.bench_function("locking_bbq_small_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::locking::Locking>(1000, 100_000, 2);
            black_box(r);
        });
    });
    c.bench_function("locking_bbq_big_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::locking::Locking>(1000, 100_000, 50);
            black_box(r);
        });
    });
}

fn bench_locking_ptrs_bbq(c: &mut Criterion) {
    c.bench_function("locking_bbq_ptr_small_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::lock_ptr::LockingPtr>(1000, 100_000, 2);
            black_box(r);
        });
    });
    c.bench_function("locking_bbq_ptr_big_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::lock_ptr::LockingPtr>(1000, 100_000, 50);
            black_box(r);
        });
    });
}

fn bench_locking_ptrs_local_bbq(c: &mut Criterion) {
    c.bench_function("locking_bbq_ptr_local_small_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::lock_ptr_local::LockingPtrLocal>(1000, 100_000, 2);
            black_box(r);
        });
    });
    c.bench_function("locking_bbq_ptr_local_big_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::lock_ptr_local::LockingPtrLocal>(1000, 100_000, 50);
            black_box(r);
        });
    });
}

fn bench_atomic_local_bbq(c: &mut Criterion) {
    c.bench_function("atomic_local_small_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::atomic_local::AtomicLocal>(1000, 100_000, 2);
            black_box(r);
        });
    });
    c.bench_function("atomic_local_big_read", |b| {
        b.iter(|| {
            let r = test_read_write::<bbq::atomic_local::AtomicLocal>(1000, 100_000, 50);
            black_box(r);
        });
    });
}

criterion_group!(
    benches,
    bench_locking_bbq,
    bench_locking_ptrs_bbq,
    bench_locking_ptrs_local_bbq,
    bench_atomic_local_bbq
);

criterion_main!(benches);
