use std::{sync::{Mutex, Arc, MutexGuard}, ops::Range};

use crate::{SpscQueue, QueueReader, ReadSlice, QueueWriter, WriteSlice};

pub struct Locking;
/*
impl SpscQueue for Locking {
    type Reader = Reader;
    type Writer = Writer;

    fn with_len(len: usize) -> (Self::Reader, Self::Writer) {
        with_len(len)
    }
}
*/

/*
impl QueueReader for Reader {
    type ReadSlice<'a> = ReadableSlice<'a>;

    fn readable<'a>(&'a mut self) -> Self::ReadSlice<'a> {
        self.readable()
    }
}
*/

impl ReadSlice for ReadableSlice<'_> {
    fn commit_read(self, len: usize) {
        self.commit_read(len)
    }
}

/*
impl QueueWriter for Writer {
    type WriteSlice<'a> = WritableSlice<'a>;

    fn reserve<'a>(&'a mut self, len: usize) -> Option<Self::WriteSlice<'a>> {
        self.reserve(len)
    }
}
*/

impl<'a> WriteSlice for WritableSlice<'a> {
    fn commit_write(self, len: usize) {
        self.commit_write(len)
    }
}

struct BipBuf {
    owned: Box<[u8]>,
    /// owning ptr to underlying buf
    read: usize,
    write: usize,
    last: usize,
}

impl BipBuf {
    fn from_box(mut s: Box<[u8]>) -> Self {
        Self {
            owned: s,
            read: 0,
            write: 0,
            last: 0,
        }
    }
}

pub struct Reader {
    inner: Arc<Mutex<BipBuf>>,
}

pub struct Writer {
    inner: Arc<Mutex<BipBuf>>,
}

pub struct ReadableSlice<'a> {
    unlocked: MutexGuard<'a, BipBuf>,
    range: Range<usize>,
}

impl<'a> AsRef<[u8]> for ReadableSlice<'a> {
    fn as_ref(&self) -> &[u8] {
        &self.unlocked.owned[self.range.clone()]
    }
}

impl ReadableSlice<'_> {
    fn commit_read(mut self, len: usize) {
        if len > self.range.end - self.range.start {
            panic!("commit read larger than readable range");
        }
        self.unlocked.read = self.range.start + len;
    }
}

impl Reader {
    fn readable(&mut self) -> ReadableSlice<'_> {
        let inner = self.inner.lock().unwrap();
        if inner.read <= inner.write {
            let range = inner.read..inner.write;
            ReadableSlice {
                unlocked: inner,
                range,
            }
        } else {
            // read > write -> wrapped around, readable from read to last
            if inner.read == inner.last {
                let range = 0..inner.write;
                ReadableSlice {
                    unlocked: inner,
                    range,
                }
            } else {
                let range = inner.read..inner.last;
                ReadableSlice {
                    unlocked: inner,
                    range,
                }
            }
        }
    }
}

pub struct WritableSlice<'a> {
    unlocked: MutexGuard<'a, BipBuf>,
    range: Range<usize>,
    watermark: Option<usize>,
}

impl Writer {
    fn reserve(&mut self, len: usize) -> Option<WritableSlice<'_>> {
        let inner = self.inner.lock().unwrap();
        if len > inner.owned.len() / 2 {
            panic!("reserve len is larger than half of buffer len");
        }
        if inner.read <= inner.write {
            if inner.write + len < inner.owned.len() {
                let range = inner.write..inner.write+len;
                let watermark = None;
                Some(WritableSlice {
                    unlocked: inner,
                    range,
                    watermark,
                })
            } else {
                if len < inner.read {
                    let range = 0..len;
                    let watermark = Some(inner.write);
                    Some(WritableSlice {
                        unlocked: inner,
                        range,
                        watermark,
                    })
                } else {
                    None
                }
            }
        } else {
            // write is before read
            if inner.write + len < inner.read {
                let range = inner.write..inner.write+len;
                let watermark = None;
                Some(WritableSlice {
                    unlocked: inner,
                    range,
                    watermark,
                })
            } else {
                None
            }
        }
    }
}

impl WritableSlice<'_> {
    fn commit_write(mut self, len: usize) {
        if len > self.range.end - self.range.start {
            panic!("commit write range larger than reserved");
        }
        self.unlocked.write = self.range.start + len;
        if let Some(last) = self.watermark {
            self.unlocked.last = last;
        }
    }
}

impl AsMut<[u8]> for WritableSlice<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.unlocked.owned[self.range.clone()]
    }
}

pub fn with_len(len: usize) -> (Reader, Writer) {
    let owned = vec![0; len].into_boxed_slice();
    let buf = Arc::new(Mutex::new(BipBuf {
        owned,
        read: 0,
        write: 0,
        last: 0,
    }));
    let reader = Reader {
        inner: buf.clone(),
    };
    let writer = Writer {
        inner: buf,
    };

    (reader, writer)
}
