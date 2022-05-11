use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::{QueueReader, QueueWriter, ReadSlice, SpscQueue, WriteSlice};

pub struct LockingPtr;
impl SpscQueue for LockingPtr {
    type Reader = Reader;
    type Writer = Writer;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer) {
        with_len(len)
    }
}

unsafe impl Send for Reader {}
unsafe impl Send for Writer {}
impl QueueReader for Reader {
    type ReadSlice<'a> = ReadableSlice<'a>;

    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a> {
        self.readable()
    }
}

impl ReadSlice for ReadableSlice<'_> {
    fn commit_read(self, len: usize) {
        self.commit_read(len)
    }
}

impl QueueWriter for Writer {
    type WriteSlice<'a> = WritableSlice<'a>;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>> {
        self.reserve(len)
    }
}

impl<'a> WriteSlice for WritableSlice<'a> {
    fn commit_write(self, len: usize) {
        self.commit_write(len)
    }
}

struct BufPtrs {
    read: usize,
    write: usize,
    last: usize,
}

struct BipBuf {
    owned: Box<[u8]>,
    len: usize,
    buf: *mut u8,
    ptrs: Mutex<BufPtrs>,
}

impl BipBuf {
    fn from_box(mut s: Box<[u8]>) -> Self {
        let buf = s.as_mut_ptr();
        let len = s.len();
        let ptrs = Mutex::new(BufPtrs {
            read: 0,
            write: 0,
            last: 0,
        });

        Self {
            owned: s,
            len,
            buf,
            ptrs,
        }
    }
}

pub struct Reader {
    inner: Arc<BipBuf>,
}

pub struct Writer {
    inner: Arc<BipBuf>,
}

pub struct ReadableSlice<'a> {
    reader: &'a mut Reader,
    range: Range<usize>,
}

impl<'a> AsRef<[u8]> for ReadableSlice<'a> {
    fn as_ref(&self) -> &[u8] {
        let slice_len = self.range.end - self.range.start;
        let start_ptr = self.reader.inner.buf.wrapping_add(self.range.start);
        unsafe { std::slice::from_raw_parts(start_ptr, slice_len) }
    }
}

impl ReadableSlice<'_> {
    fn commit_read(mut self, len: usize) {
        if len > self.range.end - self.range.start {
            panic!("commit read larger than readable range");
        }
        let mut lock = self.reader.inner.ptrs.lock().unwrap();
        lock.read = self.range.start + len;
    }
}

impl Reader {
    fn readable(&mut self) -> ReadableSlice<'_> {
        let lock = self.inner.ptrs.lock().unwrap();
        if lock.read <= lock.write {
            let range = lock.read..lock.write;
            drop(lock);
            ReadableSlice {
                reader: self,
                range,
            }
        } else {
            // read > write -> wrapped around, readable from read to last
            if lock.read == lock.last {
                let range = 0..lock.write;
                drop(lock);
                ReadableSlice {
                    reader: self,
                    range,
                }
            } else {
                let range = lock.read..lock.last;
                drop(lock);
                ReadableSlice {
                    reader: self,
                    range,
                }
            }
        }
    }
}

pub struct WritableSlice<'a> {
    writer: &'a mut Writer,
    range: Range<usize>,
    watermark: Option<usize>,
}

impl Writer {
    fn reserve(&mut self, len: usize) -> Option<WritableSlice<'_>> {
        let lock = self.inner.ptrs.lock().unwrap();
        let buf_len = self.inner.len;
        if len > buf_len / 2 {
            panic!("reserve len is larger than half of buffer len");
        }
        if lock.read <= lock.write {
            if lock.write + len < buf_len {
                let range = lock.write..lock.write + len;
                let watermark = None;
                drop(lock);
                Some(WritableSlice {
                    range,
                    watermark,
                    writer: self,
                })
            } else {
                if len < lock.read {
                    let range = 0..len;
                    let watermark = Some(lock.write);
                    drop(lock);
                    Some(WritableSlice {
                        range,
                        watermark,
                        writer: self,
                    })
                } else {
                    None
                }
            }
        } else {
            // write is before read
            if lock.write + len < lock.read {
                let range = lock.write..lock.write + len;
                let watermark = None;
                drop(lock);
                Some(WritableSlice {
                    range,
                    watermark,
                    writer: self,
                })
            } else {
                None
            }
        }
    }
}

impl WritableSlice<'_> {
    fn commit_write(self, len: usize) {
        if len > self.range.end - self.range.start {
            panic!("commit write range larger than reserved");
        }
        let mut lock = self.writer.inner.ptrs.lock().unwrap();
        lock.write = self.range.start + len;
        if let Some(last) = self.watermark {
            lock.last = last;
        }
    }
}

impl AsMut<[u8]> for WritableSlice<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        let slice_len = self.range.end - self.range.start;
        let start_ptr = self.writer.inner.buf.wrapping_add(self.range.start);
        unsafe { std::slice::from_raw_parts_mut(start_ptr, slice_len) }
    }
}

pub fn with_len(len: usize) -> (Reader, Writer) {
    let owned = vec![0; len].into_boxed_slice();
    let buf = Arc::new(BipBuf::from_box(owned));
    let reader = Reader { inner: buf.clone() };
    let writer = Writer { inner: buf };

    (reader, writer)
}
