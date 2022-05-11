#![feature(generic_associated_types)]

pub mod atomic_local;
pub mod locking;
pub mod lock_ptr;

trait ReadSlice: AsRef<[u8]> {
    fn commit_read(self, len: usize);
}

trait QueueReader {
    type ReadSlice<'a>: ReadSlice
    where
        Self: 'a;
    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a>;
}

trait WriteSlice: AsMut<[u8]> {
    fn commit_write(self, len: usize);
}

trait QueueWriter {
    type WriteSlice<'a>: WriteSlice
    where
        Self: 'a;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>>;
}

trait SpscQueue {
    type Reader: QueueReader + Send + 'static;
    type Writer: QueueWriter + Send + 'static;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer);
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Instant};
    use super::*;

    fn test_read_write<S: SpscQueue>(buf_size: usize, iter: usize) {
        let (mut reader, mut writer) = S::with_len(buf_size);
        let i = Instant::now();
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
            for i in 0..iter {
                let buf = loop {
                    let r = reader.readable();
                    if r.as_ref().len() >= 11 {
                        break r;
                    }
                };
                assert_eq!(&buf.as_ref()[0..11], b"12345678901");
                buf.commit_read(11);
            }
        });
        write_thread.join().unwrap();
        read_thread.join().unwrap();
        println!("Elapsed: {:?}", i.elapsed());
    }

    #[test]
    fn locking_bbq_read_write() {
        test_read_write::<locking::Locking>(1000, 10_000_000);
    }

    #[test]
    fn locking_ptrs_bbq_read_write() {
        test_read_write::<lock_ptr::LockingPtr>(1000, 10_000_000);
    }

    #[test]
    fn atomic_local_bbq_read_write() {
        test_read_write::<atomic_local::AtomicLocal>(1000, 10_000_000);
    }
}
