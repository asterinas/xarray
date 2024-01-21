use std::{collections::VecDeque, marker::PhantomData};

use super::*;

pub(crate) const BITS_PER_LAYER: usize = 6;
pub(crate) const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
pub(crate) const SLOT_MASK: usize = SLOT_SIZE - 1;
pub(crate) const MAX_LAYER: usize = 64 / BITS_PER_LAYER + 1;

/// `XArray` is an abstract data type functioning like an expansive array of items where each item must be an 8-byte object, such as `Arc<T>` or `Box<T>`.
/// User-stored pointers must have a minimum alignment of 4 bytes. `XArray` facilitates efficient sequential access to adjacent entries,
/// supporting multiple concurrent reads and exclusively allowing one write operation at a time.
///
/// # Features
/// **Copy-on-write (COW):** If items within `XArray` implement the `Clone` trait, cloning can leverage a COW mechanism.
/// A clone of an `XArray` initially shares the head node with the original, avoiding immediate deep copying.
/// If a mutable operation is required on either `XArray`, a deep copy of the relevant `XNode` is made first, ensuring isolated operations.
///
/// **Cursors:** Interaction with `XArray` is mediated through `Cursor` and `CursorMut`.
/// A `Cursor` requires an immutable reference, while `CursorMut` requires a mutable reference.
/// As such, multiple `Cursor` instances can coexist, but `CursorMut` operations are singular,
/// reflecting the behavior of shared (`&`) and exclusive (`&mut`) references.
/// Cursors offer precise index positioning and traversal capabilities in the `XArray`.
///
/// **Marking:** `XArray` enables marking of individual items or the `XArray` itself for user convenience.
/// Items and the `XArray` can have up to three distinct marks by default, with each mark independently maintained.
/// Marks are generally enums implementing the `ValidMark` trait. Marking is also applicable to internal nodes,
/// indicating marked descendant nodes, though such marking remains transparent to users.
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use xarray::*;
///
/// let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
/// let value = Arc::new(10);
/// xarray_arc.store(333, value);
/// assert!(*xarray_arc.load(333).unwrap().as_ref() == 10);
///
/// let mut xarray_clone = xarray_arc.clone();
/// assert!(*xarray_clone.load(333).unwrap().as_ref() == 10);
/// let value = Arc::new(100);
/// xarray_clone.store(333, value);
///
/// assert!(*xarray_arc.load(333).unwrap().as_ref() == 10);
/// assert!(*xarray_clone.load(333).unwrap().as_ref() == 100);
/// ```
///
/// The concepts XArray are originally introduced by Linux, which keeps the data structure of Linux's radix tree
/// [Linux Radix Trees](https://lwn.net/Articles/175432/).
pub struct XArray<I, M = NoneMark>
where
    I: ItemEntry,
    M: ValidMark,
{
    marks: [bool; 3],
    head: XEntry<I>,
    _marker: PhantomData<(I, M)>,
}

impl<I: ItemEntry, M: ValidMark> XArray<I, M> {
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
        self.marks[mark.index()] = true;
    }

    /// Unset the input `mark` for the `XArray`.
    pub fn unset_mark(&mut self, mark: M) {
        self.marks[mark.index()] = false;
    }

    /// Judge if the `XArray` is marked with the input `mark`.
    pub fn is_marked(&self, mark: M) -> bool {
        self.marks[mark.index()]
    }

    /// Return a reference to the head entry, and later will not modify the XNode pointed to by the `head`.
    pub(crate) fn head(&self) -> &XEntry<I> {
        &self.head
    }

    /// Return a reference to the head entry, and later will modify the XNode pointed to by the `head`.
    pub(crate) fn head_mut(&mut self) -> &XEntry<I> {
        // When a modification to the head is needed, it first checks whether the head is shared with other XArrays.
        // If it is, then it performs COW by allocating a new head and using it,
        // to prevent the modification from affecting the read or write operations on other XArrays.
        if let Some(new_head) = self.copy_if_shared(&self.head) {
            self.set_head(new_head);
        }
        &self.head
    }

    pub(crate) fn max_index(&self) -> u64 {
        if let Some(node) = self.head.as_node() {
            node.layer().max_index()
        } else {
            0
        }
    }

    /// Set the head of the `XArray` with the new `XEntry`, and return the old `head`.
    pub(crate) fn set_head(&mut self, head: XEntry<I>) -> XEntry<I> {
        let old_head = core::mem::replace(&mut self.head, head);
        old_head
    }

    /// Attempts to load the item at the target index within the `XArray`.
    /// If the target item exists, return it with `Some`, Otherwise, return `None`.
    pub fn load(&self, index: u64) -> Option<&I> {
        let mut cursor = self.cursor(index);
        cursor.load()
    }

    /// Stores the provided item in the `XArray` at the target index,
    /// and return the old item if it was previously stored in target index.
    pub fn store(&mut self, index: u64, item: I) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    /// Attempts to load the item and its mark information about input `mark` at the target index within the `XArray`.
    /// If the target item exists, return it with `Some`, Otherwise, return `None`.
    pub fn load_with_mark(&self, index: u64, mark: M) -> Option<(&I, bool)> {
        let mut cursor = self.cursor(index);
        let entry = cursor.load();
        let mark = if entry.is_some() {
            cursor.is_marked(mark)
        } else {
            None
        };
        entry.map(|entry| (entry, mark.unwrap()))
    }

    /// Stores the provided item in the `XArray` at the target index and mark it with input `mark`.
    /// and return the old item if it was previously stored in target index.
    pub fn store_with_mark(&mut self, index: u64, item: I, mark: M) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        let old_item = cursor.store(item);
        cursor.set_mark(mark).unwrap();
        old_item
    }

    /// Unset the input `mark` for all of the items in the `XArray`.
    pub fn unset_mark_all(&mut self, mark: M) {
        let mut handle_list = VecDeque::new();
        if let Some(node) = self.head_mut().as_node_mut() {
            handle_list.push_back(node);
        }
        while !handle_list.is_empty() {
            let node = handle_list.pop_front().unwrap();
            let mut offset = 0;
            let node_mark = node.mark(mark.index());
            while (offset as usize) < SLOT_SIZE {
                if node_mark.is_marked(offset) {
                    let entry = node.ref_node_entry(offset);
                    if let Some(node) = entry.as_node_mut() {
                        handle_list.push_back(node);
                    }
                }
                offset += 1;
            }
            node.clear_mark(mark.index());
        }
    }

    /// Removes the `XEntry` at the target index within the `XArray`,
    /// and return the removed item if it was previously stored in target index.
    pub fn remove(&mut self, index: u64) -> Option<I> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub fn cursor<'a>(&'a self, index: u64) -> Cursor<'a, I, M> {
        Cursor::new(self, index)
    }

    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub fn cursor_mut<'a>(&'a mut self, index: u64) -> CursorMut<'a, I, M> {
        CursorMut::new(self, index)
    }
}

impl<I: ItemEntry + Clone, M: ValidMark> Clone for XArray<I, M> {
    /// Clone with cow mechanism.
    fn clone(&self) -> Self {
        let cloned_head = self.head.clone();
        Self {
            marks: self.marks,
            head: cloned_head,
            _marker: PhantomData,
        }
    }
}
