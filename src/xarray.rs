use std::{collections::VecDeque, marker::PhantomData};

use crate::*;

pub(crate) const BITS_PER_LAYER: usize = 6;
pub(crate) const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
pub(crate) const SLOT_MASK: usize = SLOT_SIZE - 1;

/// The XArray is an abstract data type which behaves like a very large array of items.
/// Items here must be a 8 bytes object, like `Arc<T>` and `Box<T>`.
/// The alignment of the pointers stored by users must be at least 4.
/// It allows you to sensibly go to the next or previous entry in a cache-efficient manner.
/// It allows multiple concurrent reads of the XArray, but only permits one write operation on the XArray at any given time.
///
/// # Features
/// **Copy-on-write (COW).** If the item stored in `XArray` implemented `Clone` trait, the `XArray`
/// can achieve Clone with a COW mechanism. When cloning an XArray, initially the new XArray shares a head with the original XArray
/// without performing an actual clone. If either of the XArrays needs to perform a mutable operation, a substantive clone of the XNode to be modified is created before making the update.
/// This ensures that operations on the two XArrays do not affect each other.
/// **Reference.** All operations on XArray are performed through `Cursor` and `CursorMut`.
/// Cursor requires an immutable reference to XArray, while CursorMut requires a mutable reference.
/// Therefore, XArray can have multiple Cursors operating at the same time, whereas the operations of CursorMut are exclusive (similar to the relationship between & and &mut).
/// **Mark.** `XArray` supports the ability to add marks to any stored item to assist users.
/// By default, an item can be marked with up to three different marks, with each mark being independent of the others.
/// Marks for an item are typically enumerations that must implement the ValidMark trait.
/// Internal nodes can also be marked. When an intermediate node is marked, it signifies that it has child nodes that have been marked.
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
    head: XEntry<I>,
    _marker: PhantomData<(I, M)>,
}

impl<I: ItemEntry, M: ValidMark> XArray<I, M> {
    /// Make a new, empty XArray.
    pub const fn new() -> Self {
        Self {
            head: XEntry::EMPTY,
            _marker: PhantomData,
        }
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
        let entry = cursor.load();
        if entry.is_some_and(|entry| entry.is_item()) {
            entry.map(|entry| unsafe { &*(entry as *const XEntry<I> as *const I) })
        } else {
            None
        }
    }

    /// Stores the provided item in the `XArray` at the target index,
    /// and return the old item if it was previously stored in target index.
    pub fn store(&mut self, index: u64, item: I) -> Option<I> {
        let stored_entry = XEntry::from_item(item);
        let old_entry = self.cursor_mut(index).store(stored_entry);
        XEntry::into_item(old_entry)
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
        if entry.is_some_and(|entry| entry.is_item()) {
            entry.map(|entry| {
                (
                    unsafe { &*(entry as *const XEntry<I> as *const I) },
                    mark.unwrap(),
                )
            })
        } else {
            None
        }
    }

    /// Stores the provided item in the `XArray` at the target index and mark it with input `mark`.
    /// and return the old item if it was previously stored in target index.
    pub fn store_with_mark(&mut self, index: u64, item: I, mark: M) -> Option<I> {
        let stored_entry = XEntry::from_item(item);
        let mut cursor = self.cursor_mut(index);
        let old_entry = cursor.store(stored_entry);
        cursor.set_mark(mark).unwrap();
        XEntry::into_item(old_entry)
    }

    /// Mark the item at the target index in the `XArray` with the input `mark`.
    /// If the item does not exist, return an Error.
    pub fn set_mark(&mut self, index: u64, mark: M) -> Result<(), ()> {
        self.cursor_mut(index).set_mark(mark)
    }

    /// Unset the input `mark` for the item at the target index in the `XArray`.
    /// If the item does not exist, return an Error.
    pub fn unset_mark(&mut self, index: u64, mark: M) -> Result<(), ()> {
        self.cursor_mut(index).unset_mark(mark)
    }

    /// Obtain a reference to the XEntry from a pointer pointing to it.
    ///
    /// # Safety
    /// The user must ensure that the pointer remains valid for the duration of use of the target XEntry reference.
    pub(crate) unsafe fn ref_entry(&self, entry_ptr: *const XEntry<I>) -> &XEntry<I> {
        &*entry_ptr
    }

    /// Unset the input `mark` for all of the items in the `XArray`.
    pub fn unset_mark_all(&mut self, mark: M) {
        let mut handle_list = VecDeque::new();
        if let Some(node) = self.head.as_node_mut() {
            handle_list.push_back(node);
        }
        while !handle_list.is_empty() {
            let node = handle_list.pop_front().unwrap();
            let mut offset = 0;
            let node_mark = node.mark(mark.index());
            while (offset as usize) < SLOT_SIZE {
                if node_mark.is_marked(offset) {
                    // Safety: During this operation, the used XNode will not be removed and rge referenced XEntry must be valid.
                    let entry = unsafe { self.ref_entry(node.entry(offset)) };
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
        let old_entry = self.cursor_mut(index).remove();
        XEntry::into_item(old_entry)
    }

    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub(crate) fn cursor<'a>(&'a self, index: u64) -> Cursor<'a, I, M> {
        Cursor::new(self, index)
    }

    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub(crate) fn cursor_mut<'a>(&'a mut self, index: u64) -> CursorMut<'a, I, M> {
        CursorMut::new(self, index)
    }
}

impl<I: ItemEntry + Clone, M: ValidMark> Clone for XArray<I, M> {
    /// Clone with cow mechanism.
    fn clone(&self) -> Self {
        let cloned_head = self.head.clone();
        Self {
            head: cloned_head,
            _marker: PhantomData,
        }
    }
}
