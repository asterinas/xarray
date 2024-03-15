use alloc::collections::VecDeque;
use core::marker::PhantomData;

use crate::cow::Cow;
use crate::cursor::{Cursor, CursorMut};
use crate::entry::{ItemEntry, ItemRef, XEntry};
use crate::lock::XLock;
use crate::mark::{NoneMark, XMark};
use crate::range::Range;
#[cfg(feature = "std")]
use crate::StdMutex;

pub(super) const BITS_PER_LAYER: usize = 6;
pub(super) const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
pub(super) const SLOT_MASK: usize = SLOT_SIZE - 1;
pub(super) const MAX_HEIGHT: usize = 64 / BITS_PER_LAYER + 1;

/// `XArray` is an abstract data type functioning like an expansive array of items
/// where each item must be an 8-byte object, such as `Arc<T>` or `Box<T>`.
/// User-stored pointers must have a minimum alignment of 4 bytes.
/// `XArray` facilitates efficient sequential access to adjacent entries,
/// supporting multiple concurrent reads and exclusively allowing one write operation at a time.
///
/// # Features
/// **Copy-on-write (COW):** If items within `XArray` implement the `Clone` trait,
/// cloning can leverage a COW mechanism. A clone of an `XArray` initially shares the
/// head node with the original, avoiding immediate deep copying. If a mutable operation
/// is required on either `XArray`, a deep copy of the relevant `XNode` is made first,
/// ensuring isolated operations.
///
/// **Cursors:** Interaction with `XArray` is mediated through `Cursor` and `CursorMut`.
/// A `Cursor` requires an immutable reference, while `CursorMut` requires a mutable reference.
/// As such, multiple `Cursor` instances can coexist, but `CursorMut` operations are singular,
/// reflecting the behavior of shared (`&`) and exclusive (`&mut`) references.
/// Cursors offer precise index positioning and traversal capabilities in the `XArray`.
///
/// **Marking:** `XArray` enables marking of individual items or the `XArray` itself for user convenience.
/// Items and the `XArray` can have up to three distinct marks by default, with each mark independently maintained.
/// Users can use self-defined types as marks by implementing the `From<Type>` trait for XMark.
/// Marking is also applicable to internal nodes, indicating marked descendant nodes,
/// though such marking is not transparent to users.
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use std::sync::{Mutex, MutexGuard};
/// use xarray::*;
///
/// let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
/// let value = Arc::new(10);
/// xarray_arc.store(10, value);
/// assert!(*xarray_arc.load(10).unwrap().as_ref() == 10);
///
/// let mut xarray_clone = xarray_arc.clone();
/// assert!(*xarray_clone.load(10).unwrap().as_ref() == 10);
/// let value = Arc::new(100);
/// xarray_clone.store(10, value);
///
/// assert!(*xarray_arc.load(10).unwrap().as_ref() == 10);
/// assert!(*xarray_clone.load(10).unwrap().as_ref() == 100);
/// ```
///
/// The concepts XArray are originally introduced by Linux, which keeps the data structure of
/// Linux's radix tree [Linux Radix Trees](https://lwn.net/Articles/175432/).
pub struct XArray<
    I,
    #[cfg(feature = "std")] L = StdMutex,
    #[cfg(not(feature = "std"))] L,
    M = NoneMark,
> where
    I: ItemEntry,
    L: XLock,
    M: Into<XMark>,
{
    marks: [bool; 3],
    head: XEntry<I, L>,
    _marker: PhantomData<(I, M)>,
}

impl<'a, I: ItemEntry, L: XLock, M: Into<XMark>> XArray<I, L, M> {
    /// Make a new, empty XArray.
    pub const fn new() -> Self {
        Self {
            marks: [false; 3],
            head: XEntry::EMPTY,
            _marker: PhantomData,
        }
    }

    /// Mark the `XArray` with the input `mark`.
    pub fn set_mark(&mut self, mark: M) {
        self.marks[mark.into().index()] = true;
    }

    /// Unset the input `mark` for the `XArray`.
    pub fn unset_mark(&mut self, mark: M) {
        self.marks[mark.into().index()] = false;
    }

    /// Judge if the `XArray` is marked with the input `mark`.
    pub fn is_marked(&self, mark: M) -> bool {
        self.marks[mark.into().index()]
    }

    /// Return a reference to the head entry, and later will not modify the XNode pointed to by the `head`.
    pub(super) fn head(&self) -> &XEntry<I, L> {
        &self.head
    }

    /// Ensure current head in the XArray is exclusive.
    ///
    /// If it is shared with other XArrays, it will perform COW by allocating a new head and using it
    /// to prevent the modification from affecting the read or write operations on other XArrays.
    pub(super) fn ensure_head_exclusive(&mut self) {
        if let Some(new_head) = self.head.copy_if_shared() {
            self.set_head(new_head);
        }
    }

    /// Calculate the max index that can stored in the XArray with current height.
    pub(super) fn max_index(&self) -> u64 {
        if let Some(node) = self.head.as_node() {
            node.height().max_index()
        } else {
            0
        }
    }

    /// Set the head of the `XArray` with the new `XEntry`, and return the old `head`.
    pub(super) fn set_head(&mut self, head: XEntry<I, L>) -> XEntry<I, L> {
        let old_head = core::mem::replace(&mut self.head, head);
        old_head
    }

    /// Attempts to load the item at the target index within the `XArray`.
    /// If the target item exists, return it with `Some`, Otherwise, return `None`.
    pub fn load(&'a self, index: u64) -> Option<ItemRef<'a, I>> {
        let cursor = self.cursor(index);
        cursor.load()
    }

    /// Stores the provided item in the `XArray` at the target index,
    /// and return the old item if it was previously stored in target index.
    pub fn store(&mut self, index: u64, item: I) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    /// Unset the input `mark` for all of the items in the `XArray`.
    pub fn unset_mark_all(&mut self, mark: M) {
        let mut handle_list = VecDeque::new();
        self.ensure_head_exclusive();
        if let Some(node) = self.head().as_node_mut() {
            handle_list.push_back(node);
        }
        let mark_index = mark.into().index();
        while !handle_list.is_empty() {
            let node = handle_list.pop_front().unwrap();
            let mut offset = 0;
            let node_mark = node.mark(mark_index);
            while (offset as usize) < SLOT_SIZE {
                if node_mark.is_marked(offset) {
                    // SAFETY: This function will not modify any slots of the XNode.
                    let entry = unsafe { node.ref_node_entry(true, offset) };
                    if let Some(node) = entry.as_node_mut() {
                        handle_list.push_back(node);
                    }
                }
                offset += 1;
            }
            node.clear_mark(mark_index);
        }
    }

    /// Removes the `XEntry` at the target index within the `XArray`,
    /// and return the removed item if it was previously stored in target index.
    pub fn remove(&mut self, index: u64) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub fn cursor(&self, index: u64) -> Cursor<'_, I, L, M> {
        Cursor::new(self, index)
    }

    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub fn cursor_mut(&mut self, index: u64) -> CursorMut<'_, I, L, M> {
        CursorMut::new(self, index)
    }

    /// Create a `Range` which can be immutably iterated over the index corresponding to the `range`
    /// in `XArray`.
    pub fn range(&self, range: core::ops::Range<u64>) -> Range<'_, I, L, M> {
        let cursor = Cursor::new(self, range.start);
        Range::new(cursor, range.start, range.end)
    }
}

impl<I: ItemEntry + Clone, L: XLock, M: Into<XMark>> Clone for XArray<I, L, M> {
    /// Clone with COW mechanism.
    fn clone(&self) -> Self {
        let cloned_head = self.head.clone();
        Self {
            marks: self.marks,
            head: cloned_head,
            _marker: PhantomData,
        }
    }
}
