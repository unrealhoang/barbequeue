#![feature(generic_associated_types)]

use std::fmt::Debug;

pub mod atomic_local;
pub mod locking;

trait ReadSlice: AsRef<[u8]> {
    fn commit_read(self, len: usize);
}

trait QueueReader {
    type ReadSlice<'a>: ReadSlice + Debug
    where
        Self: 'a;
    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a>;
}

trait WriteSlice: AsMut<[u8]> {
    fn commit_write(self, len: usize);
}

trait QueueWriter {
    type WriteSlice<'a>: WriteSlice + Debug
    where
        Self: 'a;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>>;
}

trait SpscQueue {
    type Reader: QueueReader + Send + Debug + 'static;
    type Writer: QueueWriter + Send + Debug + 'static;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer);
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Instant};
    use super::*;

    fn test_read_write<S: SpscQueue>() {
        let (mut reader, mut writer) = S::with_len(1000);
        let t = Instant::now();
        let write_thread = thread::spawn(move || {
            for i in 0..10_000_000 {
                let mut buf = loop {
                    if let Some(b) = writer.reserve(11) {
                        break b;
                    }
                };
                // println!("{i} dbg write slice: {:?}", buf);
                buf.as_mut().copy_from_slice(b"12345678901");
                buf.commit_write(11);
                // println!("{i} dbg write: {:?}", writer);
            }
        });

        let read_thread = thread::spawn(move || {
            for i in 0..10_000_000 {
                let buf = loop {
                    let r = reader.readable();
                    if r.as_ref().len() >= 11 {
                        break r;
                    }
                };
                // println!("{i} dbg read slice: {:?}", buf);
                assert_eq!(&buf.as_ref()[0..11], b"12345678901");
                buf.commit_read(11);
                // println!("{i} dbg read: {:?}", reader);
            }
            println!("done read");
        });
        write_thread.join().unwrap();
        read_thread.join().unwrap();
        println!("elapsed: {:?}", t.elapsed());
    }

    #[test]
    fn locking_bbq_read_write() {
        // test_read_write::<locking::Locking>();
        test_read_write::<atomic_local::AtomicLocal>();
    }
}
