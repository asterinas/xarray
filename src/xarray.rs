use alloc::collections::VecDeque;
use core::marker::PhantomData;

use crate::cursor::{Cursor, CursorMut};
use crate::entry::{ItemEntry, ItemRef, XEntry};
use crate::mark::{NoneMark, XMark};
use crate::node::{Height, XNode};
use crate::range::Range;

pub(super) const BITS_PER_LAYER: usize = 6;
pub(super) const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
pub(super) const SLOT_MASK: usize = SLOT_SIZE - 1;
pub(super) const MAX_HEIGHT: usize = 64 / BITS_PER_LAYER + 1;

/// `XArray` is an abstract data type functioning like an expansive array of items where each item
/// must be an 8-byte object, such as `Arc<T>` or `Box<T>`.
///
/// User-stored pointers must have a minimum alignment of 4 bytes. `XArray` facilitates efficient
/// sequential access to adjacent entries, supporting multiple concurrent reads and exclusively
/// allowing one write operation at a time.
///
/// # Features
/// **Copy-on-write (COW):** If items within `XArray` implement the `Clone` trait, cloning can
/// leverage a COW mechanism. A clone of an `XArray` initially shares the head node with the
/// original, avoiding immediate deep copying. If a mutable operation is required on either
/// `XArray`, a deep copy of the relevant nodes is made first, ensuring isolated operations.
///
/// **Cursors:** Interaction with `XArray` is mediated through [`Cursor`] and [`CursorMut`]. A
/// `Cursor` requires an immutable reference, while `CursorMut` requires a mutable reference. As
/// such, multiple `Cursor` instances can coexist, but `CursorMut` operations are singular,
/// reflecting the behavior of shared (`&`) and exclusive (`&mut`) references. Cursors offer
/// precise index positioning and traversal capabilities in the `XArray`.
///
/// **Marking:** `XArray` enables marking of individual items or the `XArray` itself for user
/// convenience. Items and the `XArray` can have up to three distinct marks by default, with each
/// mark independently maintained. Users can use self-defined types as marks by implementing the
/// `From<Type>` trait for [`XMark`]. Marking is also applicable to internal nodes, indicating
/// marked descendant nodes, though such marking is not transparent to users.
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
/// The XArray concept was originally introduced by Linux, which keeps the data structure of [Linux
/// Radix Trees](https://lwn.net/Articles/175432/).
pub struct XArray<I, M = NoneMark>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    marks: [bool; 3],
    head: XEntry<I>,
    _marker: PhantomData<M>,
}

impl<I: ItemEntry, M: Into<XMark>> XArray<I, M> {
    /// Makes a new, empty `XArray`.
    pub const fn new() -> Self {
        Self {
            marks: [false; 3],
            head: XEntry::EMPTY,
            _marker: PhantomData,
        }
    }

    /// Marks the `XArray` with the input `mark`.
    pub fn set_mark(&mut self, mark: M) {
        self.marks[mark.into().index()] = true;
    }

    /// Unsets the input `mark` for the `XArray`.
    pub fn unset_mark(&mut self, mark: M) {
        self.marks[mark.into().index()] = false;
    }

    /// Checks whether the `XArray` is marked with the input `mark`.
    pub fn is_marked(&self, mark: M) -> bool {
        self.marks[mark.into().index()]
    }

    /// Returns a shared reference to the head `XEntry`.
    pub(super) fn head(&self) -> &XEntry<I> {
        &self.head
    }

    /// Returns an exclusive reference to the head `XEntry`.
    pub(super) fn head_mut(&mut self) -> &mut XEntry<I> {
        &mut self.head
    }

    /// Increases the height of the `XArray` so that the `index`-th element can be stored.
    pub(super) fn reserve(&mut self, index: u64) {
        if self.head.is_null() {
            let height = Height::from_index(index);
            self.head = XEntry::from_node(XNode::new_root(height));
            return;
        }

        loop {
            let height = self.head.as_node_ref().unwrap().height();

            if height.max_index() >= index {
                return;
            }

            let old_entry = core::mem::replace(&mut self.head, XEntry::EMPTY);

            let mut new_node = XNode::new_root(height.go_root());
            new_node.set_entry(0, old_entry);

            self.head = XEntry::from_node(new_node);
        }
    }

    /// Calculates the maximum index of elements that can be stored with the current height of the
    /// `XArray`.
    pub(super) fn max_index(&self) -> u64 {
        self.head()
            .as_node_ref()
            .map(|node| node.height().max_index())
            .unwrap_or(0)
    }

    /// Loads the `index`-th item.
    ///
    /// If the target item exists, it will be returned with `Some(_)`, otherwise, `None` will be
    /// returned.
    pub fn load(&self, index: u64) -> Option<ItemRef<'_, I>> {
        let mut cursor = self.cursor(index);
        cursor.load()
    }

    /// Stores the provided item in the `XArray` at the target index, and returns the old item if
    /// some item was previously stored in the same position.
    pub fn store(&mut self, index: u64, item: I) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    /// Unsets the input `mark` for all of the items in the `XArray`.
    pub fn unset_mark_all(&mut self, mark: M) {
        let mut pending_nodes = VecDeque::new();

        if let Some(node) = self.head.as_node_mut_or_cow() {
            pending_nodes.push_back(node);
        }

        let mark_index = mark.into().index();

        while let Some(node) = pending_nodes.pop_front() {
            let node_mark = node.mark(mark_index);
            node.clear_mark(mark_index);

            node.entries_mut()
                .iter_mut()
                .enumerate()
                .filter(|(offset, _)| node_mark.is_marked(*offset as u8))
                .filter_map(|(_, next_entry)| next_entry.as_node_mut_or_cow())
                .for_each(|next_node| pending_nodes.push_back(next_node));
        }
    }

    /// Removes the `XEntry` in the `XArray` at the target index, and returns the removed item if
    /// some item was previously stored in the same position.
    pub fn remove(&mut self, index: u64) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    /// Creates a [`Cursor`] to perform read-related operations in the `XArray`.
    pub fn cursor(&self, index: u64) -> Cursor<'_, I, M> {
        Cursor::new(self, index)
    }

    /// Creates a [`CursorMut`] to perform read- and write-related operations in the `XArray`.
    pub fn cursor_mut(&mut self, index: u64) -> CursorMut<'_, I, M> {
        CursorMut::new(self, index)
    }

    /// Creates a [`Range`] which can be immutably iterated over the indexes corresponding to the
    /// specified `range`.
    pub fn range(&self, range: core::ops::Range<u64>) -> Range<'_, I, M> {
        let cursor = Cursor::new(self, range.start);
        Range::new(cursor, range.end)
    }
}

impl<I: ItemEntry + Clone, M: Into<XMark>> Clone for XArray<I, M> {
    /// Clones the `XArray` with the COW mechanism.
    fn clone(&self) -> Self {
        let cloned_head = self.head.clone();
        Self {
            marks: self.marks,
            head: cloned_head,
            _marker: PhantomData,
        }
    }
}
