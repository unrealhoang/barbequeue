#![feature(generic_associated_types)]

pub mod atomic_local;
pub mod locking;
pub mod lock_ptr;
pub mod lock_ptr_local;

pub trait ReadSlice: AsRef<[u8]> {
    fn commit_read(self, len: usize);
}

pub trait QueueReader {
    type ReadSlice<'a>: ReadSlice
    where
        Self: 'a;
    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a>;
}

pub trait WriteSlice: AsMut<[u8]> {
    fn commit_write(self, len: usize);
}

pub trait QueueWriter {
    type WriteSlice<'a>: WriteSlice
    where
        Self: 'a;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>>;
}

pub trait SpscQueue {
    type Reader: QueueReader + Send + 'static;
    type Writer: QueueWriter + Send + 'static;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer);
}
