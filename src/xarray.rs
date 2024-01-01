use std::{
    marker::PhantomData,
    sync::{Arc, Mutex, RwLock},
};

use crate::*;
pub const CHUNK_SHIFT: usize = 6;
pub const CHUNK_SIZE: usize = 1 << CHUNK_SHIFT;
pub const CHUNK_MASK: usize = CHUNK_SIZE - 1;

/// The XArray is an abstract data type which behaves like a very large array of items.
/// Items here must be a 8 bytes object, like pointers or `u64`.
/// it allows you to sensibly go to the next or previous entry in a cache-efficient manner.
/// Normal pointers may be stored in the XArray directly. They must be 4-byte aligned
///
/// # Example
///
/// ```
/// let mut xarray_arc: XArray<Arc<i32>, XMarkDemo> = XArray::new();
/// let v1 = Arc::new(10);
/// xarray_arc.store(333, v1);
/// assert!(xarray_arc.load(333).unwrap() == 10);
/// ```
///
/// It can support storing a range of indices at once （Multi-index entries, TODO）.
/// It also supports a marking function, allowing for the stored contents to be marked with the required labels (TODO).
pub struct XArray<I: ItemEntry, M: XMark> {
    head: XEntry,
    _marker: PhantomData<(I, M)>,
}

impl<I: ItemEntry, M: XMark> XArray<I, M> {
    pub const fn new() -> Self {
        Self {
            head: XEntry::EMPTY,
            _marker: PhantomData,
        }
    }

    pub(crate) fn head(&self) -> &XEntry {
        &self.head
    }

    pub(crate) fn set_head(&mut self, head: OwnedEntry<I, Node>) -> Option<OwnedEntry<I, Node>> {
        let old_head = OwnedEntry::<I, Node>::from_raw(self.head);
        self.head = OwnedEntry::<I, Node>::into_raw(head);
        old_head
    }

    pub fn load<'a>(&'a self, index: u64) -> Option<I::Target<'a>> {
        let mut cursor = self.cursor(index);
        let entry = cursor.current();
        unsafe { Some(I::load_item(entry)) }
    }

    pub fn store(&mut self, index: u64, value: I) {
        self.cursor_mut(index).store(value);
    }

    // pub fn remove(&mut self, index: u64) {
    //     self.cursor_mut(index).remove();
    // }

    pub fn cursor<'a>(&'a self, index: u64) -> Cursor<'a, I, M> {
        Cursor {
            xa: self,
            xas: State::new(index),
        }
    }

    pub fn cursor_mut<'a>(&'a mut self, index: u64) -> CursorMut<'a, I, M> {
        CursorMut {
            xa: self,
            xas: State::new(index),
        }
    }
}

pub struct Cursor<'a, I: ItemEntry, M: XMark> {
    xa: &'a XArray<I, M>,
    xas: State<I, M>,
}

impl<'a, I: ItemEntry, M: XMark> Cursor<'a, I, M> {
    pub fn current(&mut self) -> &'a XEntry {
        let Self { xa, xas } = self;
        xas.load(xa)
    }

    pub fn key(&mut self) -> u64 {
        self.xas.index
    }
}

pub struct CursorMut<'a, I: ItemEntry, M: XMark> {
    xa: &'a mut XArray<I, M>,
    xas: State<I, M>,
}

impl<'a, I: ItemEntry, M: XMark> CursorMut<'a, I, M> {
    pub fn current(&'a mut self) -> &'a XEntry {
        let Self { xa, xas } = self;
        xas.load(xa)
    }

    pub fn store(&mut self, item: I) {
        let Self { xa, xas } = self;
        xas.store(xa, OwnedEntry::from_item(item));
    }

    pub fn key(&mut self) -> u64 {
        self.xas.index
    }
}

impl<I: ItemEntry, M: XMark> Drop for XArray<I, M> {
    fn drop(&mut self) {
        if self.head.is_node() {
            OwnedEntry::<I, Node>::from_raw(self.head);
        }
    }
}
