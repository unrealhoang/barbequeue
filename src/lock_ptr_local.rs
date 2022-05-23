use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::{QueueReader, QueueWriter, ReadSlice, SpscQueue, WriteSlice};

pub struct LockingPtrLocal;
impl SpscQueue for LockingPtrLocal {
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
    read: Mutex<usize>,
    write: Mutex<usize>,
    watermark: Mutex<usize>,
}

struct BipBuf {
    owned: Box<[u8]>,
    len: usize,
    buf: *mut u8,
    ptrs: BufPtrs,
}

impl BipBuf {
    fn from_box(mut s: Box<[u8]>) -> Self {
        let buf = s.as_mut_ptr();
        let len = s.len();
        let ptrs = BufPtrs {
            read: Mutex::new(0),
            write: Mutex::new(0),
            watermark: Mutex::new(0),
        };

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
    read: usize,
}

pub struct Writer {
    inner: Arc<BipBuf>,
    write: usize,
    cache_read: usize,
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
        let mut lock = self.reader.inner.ptrs.read.lock().unwrap();
        self.reader.read = self.range.start + len;
        *lock = self.reader.read;
    }
}

impl Reader {
    fn readable(&mut self) -> ReadableSlice<'_> {
        // always lock write before lock watermark
        let lock_write = self.inner.ptrs.write.lock().unwrap();
        if self.read <= *lock_write {
            let range = self.read..*lock_write;
            drop(lock_write);
            ReadableSlice {
                reader: self,
                range,
            }
        } else {
            let lock_watermark = self.inner.ptrs.watermark.lock().unwrap();
            // read > write -> wrapped around, readable from read to watermark
            if self.read == *lock_watermark {
                let range = 0..*lock_write;
                drop(lock_watermark);
                drop(lock_write);
                ReadableSlice {
                    reader: self,
                    range,
                }
            } else {
                let range = self.read..*lock_watermark;
                drop(lock_watermark);
                drop(lock_write);
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
        let buf_len = self.inner.len;
        if len > buf_len / 2 {
            panic!("reserve len is larger than half of buffer len");
        }

        if self.cache_read <= self.write {
            if self.write + len < buf_len {
                let range = self.write..self.write + len;
                let watermark = None;
                Some(WritableSlice {
                    range,
                    watermark,
                    writer: self,
                })
            } else {
                if len < self.cache_read {
                    let range = 0..len;
                    let watermark = Some(self.write);
                    Some(WritableSlice {
                        range,
                        watermark,
                        writer: self,
                    })
                } else {
                    let read = self.inner.ptrs.read.lock().unwrap();
                    self.cache_read = *read;
                    drop(read);

                    if len < self.cache_read {
                        let range = 0..len;
                        let watermark = Some(self.write);
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
        } else {
            // write is before read
            if self.write + len < self.cache_read {
                let range = self.write..self.write + len;
                let watermark = None;
                Some(WritableSlice {
                    range,
                    watermark,
                    writer: self,
                })
            } else {
                let read = self.inner.ptrs.read.lock().unwrap();
                self.cache_read = *read;
                drop(read);
                if self.write + len < self.cache_read {
                    let range = self.write..self.write + len;
                    let watermark = None;
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
}

impl WritableSlice<'_> {
    fn commit_write(self, len: usize) {
        if len > self.range.end - self.range.start {
            panic!("commit write range larger than reserved");
        }
        // always lock write before lock watermark
        let mut lock_write = self.writer.inner.ptrs.write.lock().unwrap();
        if let Some(watermark) = self.watermark {
            let mut lock_watermark = self.writer.inner.ptrs.watermark.lock().unwrap();
            *lock_watermark = watermark;
        }
        self.writer.write = self.range.start + len;
        *lock_write = self.writer.write;
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
    let reader = Reader { inner: buf.clone(), read: 0 };
    let writer = Writer { inner: buf, write: 0, cache_read: 0 };

    (reader, writer)
}
