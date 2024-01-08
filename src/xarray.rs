use std::marker::PhantomData;

use crate::*;

pub(crate) const NODE_HEIGHT: usize = 6;
pub(crate) const SLOT_SIZE: usize = 1 << NODE_HEIGHT;
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
pub struct XArray<I>
where
    I: ItemEntry,
{
    head: XEntry<I>,
    _marker: PhantomData<I>,
}

impl<I: ItemEntry> XArray<I> {
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
        // If it is, then it performs a copy-on-write by allocating a new head and using it,
        // to prevent the modification from affecting the read or write operations on other XArrays.
        self.copy_on_write(unsafe { &*(&self.head as *const XEntry<I>) }, 0)
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

    /// Removes the `XEntry` at the target index within the `XArray`,
    /// and return the removed item if it was previously stored in target index.
    pub fn remove(&mut self, index: u64) -> Option<I> {
        let old_entry = self.cursor_mut(index).remove();
        XEntry::into_item(old_entry)
    }

    /// Create an `Cursor` to perform read related operations on the `XArray`.
    fn cursor<'a>(&'a self, index: u64) -> Cursor<'a, I> {
        Cursor::new(self, index)
    }

    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    fn cursor_mut<'a>(&'a mut self, index: u64) -> CursorMut<'a, I> {
        CursorMut::new(self, index)
    }
}

impl<I: ItemEntry + Clone> Clone for XArray<I> {
    /// Clone with cow mechanism.
    fn clone(&self) -> Self {
        let cloned_head = self.head.clone();
        Self {
            head: cloned_head,
            _marker: PhantomData,
        }
    }
}
