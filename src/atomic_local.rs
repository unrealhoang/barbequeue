use std::{
    fmt::Debug,
    slice,
    sync::{
        atomic::{AtomicPtr, Ordering},
        Arc,
    },
};

use cache_padded::CachePadded;

use crate::{QueueReader, QueueWriter, ReadSlice, SpscQueue, WriteSlice};

pub struct AtomicLocal;
impl SpscQueue for AtomicLocal {
    type Reader = Reader;
    type Writer = Writer;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer) {
        with_len(len)
    }
}

impl QueueReader for Reader {
    type ReadSlice<'a> = ReadableSlice<'a>;

    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a> {
        self.readable()
    }
}

impl QueueWriter for Writer {
    type WriteSlice<'a> = WritableSlice<'a>;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>> {
        self.reserve(len)
    }
}

/// BipBuffer
/// buf -> [ | | | | | | | | | | ]
/// read          ^
/// write                 ^
/// last    ^
struct BipBuf {
    owned: Box<[u8]>,
    /// read ptr
    read: CachePadded<AtomicPtr<u8>>,
    write: CachePadded<AtomicPtr<u8>>,
    last: CachePadded<AtomicPtr<u8>>,

    buf: *mut u8,
    buf_end: *mut u8,
    len: usize,
}

impl BipBuf {
    fn from_box(mut s: Box<[u8]>) -> Self {
        let len = s.len();
        let buf_range = s.as_mut_ptr_range();
        let buf = buf_range.start;
        let buf_end = buf_range.end;
        let read = AtomicPtr::new(buf).into();
        let write = AtomicPtr::new(buf).into();
        let last = AtomicPtr::new(buf).into();

        Self {
            owned: s,
            read,
            write,
            last,
            buf,
            buf_end,
            len,
        }
    }

    fn split(self) -> (Reader, Writer) {
        let read = self.read.load(Ordering::SeqCst);
        let write = self.write.load(Ordering::SeqCst);

        let arc_read = Arc::new(self);
        let arc_write = Arc::clone(&arc_read);

        (
            Reader {
                inner: arc_read,
                read,
                last: None,
            },
            Writer {
                inner: arc_write,
                read,
                write,
            },
        )
    }
}

pub struct Reader {
    inner: Arc<BipBuf>,
    read: *const u8,
    last: Option<*mut u8>,
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let read_offset = unsafe { self.read.offset_from(self.inner.buf) };
        let last_offset = unsafe { self.last.map(|s| s.offset_from(self.inner.buf)) };
        f.debug_struct("Reader")
            .field("read", &read_offset)
            .field("last", &last_offset)
            .finish()
    }
}

pub struct ReadableSlice<'a> {
    rd: &'a mut Reader,
    start: *const u8,
    end: *const u8,
}

impl<'a> ReadableSlice<'a> {
    fn new(start: *const u8, end: *const u8, rd: &'a mut Reader) -> Self {
        let start_offset = unsafe { start.offset_from(rd.inner.buf) };
        let end_offset = unsafe { end.offset_from(rd.inner.buf) };
        debug_assert!(end >= start, "end = {}, start = {}", end_offset, start_offset);
        Self {
            rd, start, end
        }
    }
}

impl<'a> AsRef<[u8]> for ReadableSlice<'a> {
    fn as_ref(&self) -> &[u8] {
        let len = unsafe { self.end.offset_from(self.start) } as usize;
        unsafe { slice::from_raw_parts(self.start, len) }
    }
}

impl<'a> ReadSlice for ReadableSlice<'a> {
    fn commit_read(self, len: usize) {
        if len > unsafe { self.end.offset_from(self.start) as usize } {
            panic!("commit read larger than readable range");
        }
        self.rd.read = unsafe { self.start.add(len) };
        self.rd
            .inner
            .read
            .store(self.rd.read as *mut u8, Ordering::SeqCst)
    }
}

impl<'a> Debug for ReadableSlice<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start_offset = unsafe { self.start.offset_from(self.rd.inner.buf) };
        let end_offset = unsafe { self.end.offset_from(self.rd.inner.buf) };
        f.debug_struct("ReadableSlice")
            .field("start", &start_offset)
            .field("end", &end_offset)
            .finish()
    }
}

impl Reader {
    pub fn readable(&mut self) -> ReadableSlice<'_> {
        let read = self.read;
        let write = self.inner.write.load(Ordering::SeqCst);
        if read <= write {
            ReadableSlice::new(read, write, self)
        } else {
            let last = self.inner.last.load(Ordering::SeqCst);
            self.last = Some(last);
            if read == last {
                ReadableSlice::new(self.inner.buf, write, self)
            } else {
                if read > last {
                    let read_offset = unsafe { read.offset_from(self.inner.buf) };
                    let write_offset = unsafe { write.offset_from(self.inner.buf) };
                    let last_offset = unsafe { last.offset_from(self.inner.buf) };
                    dbg!(read_offset, write_offset, last_offset);
                }
                ReadableSlice::new(read, last, self)
            }
        }
    }
}

pub struct Writer {
    inner: Arc<BipBuf>,
    write: *mut u8,
    read: *mut u8,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let write_offset = unsafe { self.write.offset_from(self.inner.buf) };
        let read_offset = unsafe { self.read.offset_from(self.inner.buf) };
        f.debug_struct("Writer")
            .field("write", &write_offset)
            .field("read", &read_offset)
            .finish()
    }
}

pub struct WritableSlice<'a> {
    wr: &'a mut Writer,
    start: *mut u8,
    end: *mut u8,
    watermark: Option<*mut u8>,
}

impl<'a> WritableSlice<'a> {
    fn new(start: *mut u8, end: *mut u8, watermark: Option<*mut u8>, wr: &'a mut Writer) -> Self {
        let start_offset = unsafe { start.offset_from(wr.inner.buf) };
        let end_offset = unsafe { end.offset_from(wr.inner.buf) };
        debug_assert!(end >= start, "end = {}, start = {}", end_offset, start_offset);
        WritableSlice {
            wr, start, end, watermark
        }
    }
}

impl<'a> Debug for WritableSlice<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let start_offset = unsafe { self.start.offset_from(self.wr.inner.buf) };
        let end_offset = unsafe { self.end.offset_from(self.wr.inner.buf) };
        let watermark_offset = unsafe { self.watermark.map(|s| s.offset_from(self.wr.inner.buf)) };
        f.debug_struct("WritableSlice")
            .field("start", &start_offset)
            .field("end", &end_offset)
            .field("watermark", &watermark_offset)
            .finish()
    }
}

impl<'a> WriteSlice for WritableSlice<'a> {
    fn commit_write(self, len: usize) {
        if len > unsafe { self.end.offset_from(self.start) as usize } {
            panic!("commit write larger than reserved range");
        }

        if let Some(last) = self.watermark {
            self.wr.inner.last.store(last, Ordering::SeqCst);
        }
        self.wr.inner.write.store(self.end, Ordering::SeqCst);
        self.wr.write = self.end;
    }
}

impl<'a> AsMut<[u8]> for WritableSlice<'a> {
    fn as_mut(&mut self) -> &mut [u8] {
        let len = unsafe { self.end.offset_from(self.start) } as usize;
        unsafe { slice::from_raw_parts_mut(self.start, len) }
    }
}

impl Writer {
    pub fn reserve(&mut self, len: usize) -> Option<WritableSlice<'_>> {
        let write = self.write;
        let mut read = self.read;
        let to = unsafe { write.add(len) };
        if read <= write {
            if to < self.inner.buf_end {
                return Some(WritableSlice::new(write, to, None, self));
            }

            // not enough space at the end, wrap around
            let start_to = unsafe { self.inner.buf.add(len) };
            if start_to < read {
                return Some(WritableSlice::new(self.inner.buf, start_to, Some(write), self));
            }

            // update local read and try again
            self.read = self.inner.read.load(Ordering::SeqCst);
            read = self.read;
            if start_to < read {
                return Some(WritableSlice::new(self.inner.buf, start_to, Some(write), self));
            }

            return None;
        } else {
            // write is before read
            if to < read {
                return Some(WritableSlice::new(write, to, None, self));
            }

            // update local read and try again
            self.read = self.inner.read.load(Ordering::SeqCst);
            read = self.read;
            if to < read {
                return Some(WritableSlice::new(write, to, None, self));
            }

            return None;
        }
    }
}

pub fn with_len(len: usize) -> (Reader, Writer) {
    let storage = vec![0; len];
    let bb = BipBuf::from_box(storage.into_boxed_slice());

    bb.split()
}

unsafe impl Send for Reader {}
unsafe impl Send for Writer {}
