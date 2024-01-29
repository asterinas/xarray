use alloc::boxed::Box;
use alloc::sync::Arc;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;

use super::*;

/// A trait that should be implemented for the types users wish to store in an `XArray`.
/// Items stored in an XArray are required to be 8 bytes in size, Currently it can be various pointer types.
///
/// # Safety
/// Users must ensure that the produced `usize` of `into_raw()` meets the requirements for an item entry in the XArray. Specifically,
/// if the original type is a pointer, the last two bits should be 00; if the original
/// type is a value like usize, the last bit should be 1 (TODO).
pub unsafe trait ItemEntry {
    /// Converts the original type into a `usize`, consuming the ownership of the original type.
    ///
    /// This `usize` should be directly stored in an XArray's XEntry.
    fn into_raw(self) -> usize;

    /// Recovers the original type from a usize, reclaiming ownership.
    ///
    /// # Safety
    /// The raw value passed must have been produced by the corresponding `into_raw` method in this trait
    /// from the same type.
    unsafe fn from_raw(raw: usize) -> Self;
}

unsafe impl<T> ItemEntry for Arc<T> {
    fn into_raw(self) -> usize {
        let raw_ptr = unsafe { core::intrinsics::transmute::<Arc<T>, *const u8>(self) };
        debug_assert!(raw_ptr.is_aligned_to(4));
        raw_ptr as usize
    }

    unsafe fn from_raw(raw: usize) -> Self {
        let arc = core::intrinsics::transmute::<usize, Arc<T>>(raw);
        arc
    }
}

unsafe impl<T> ItemEntry for Box<T> {
    fn into_raw(self) -> usize {
        let raw_ptr = Box::into_raw(self) as *const u8;
        debug_assert!(raw_ptr.is_aligned_to(4));
        raw_ptr as usize
    }

    unsafe fn from_raw(raw: usize) -> Self {
        Box::from_raw(raw as *mut _)
    }
}

/// The type stored in the head of `XArray` and the slots of `XNode`s, which is the basic unit of storage within an XArray.
/// There are the following types of `XEntry`:
/// - Internal entries: These are invisible to users and have the last two bits set to 10. Currently `XArray` only have node
/// entries as internal entries, which are entries that point to XNodes.
/// - Item entries: Items stored by the user. Currently stored items can only be pointers and the last two bits
/// of these item entries are 00.
///
/// `XEntry` have the ownership. Once it generated from an item or a XNode, the ownership of the item or the XNode
/// will be transferred to the `XEntry`. If the stored item in the XArray implemented Clone trait, then the XEntry
/// in the XArray can also implement Clone trait.
#[derive(Eq, Debug)]
pub(super) struct XEntry<I, L>
where
    I: ItemEntry,
    L: XLock,
{
    raw: usize,
    _marker: core::marker::PhantomData<(I, L)>,
}

impl<I: ItemEntry, L: XLock> Drop for XEntry<I, L> {
    fn drop(&mut self) {
        if self.is_item() {
            unsafe {
                I::from_raw(self.raw);
            }
        }
        if self.is_node() {
            unsafe {
                Arc::from_raw((self.raw - 2) as *const XNode<I, L>);
            }
        }
    }
}

impl<I: ItemEntry + Clone, L: XLock> Clone for XEntry<I, L> {
    fn clone(&self) -> Self {
        if self.is_item() {
            let cloned_entry = unsafe {
                let item_entry = ManuallyDrop::new(I::from_raw(self.raw));
                XEntry::from_item((*item_entry).clone())
            };
            cloned_entry
        } else {
            if self.is_node() {
                unsafe {
                    Arc::increment_strong_count((self.raw - 2) as *const XNode<I, L>);
                }
            }
            Self {
                raw: self.raw,
                _marker: core::marker::PhantomData,
            }
        }
    }
}

impl<I: ItemEntry, L: XLock> PartialEq for XEntry<I, L> {
    fn eq(&self, o: &Self) -> bool {
        self.raw == o.raw
    }
}

impl<I: ItemEntry, L: XLock> XEntry<I, L> {
    pub fn raw(&self) -> usize {
        self.raw
    }

    pub const EMPTY: Self = unsafe { Self::new(0) };

    pub const unsafe fn new(raw: usize) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    pub fn is_null(&self) -> bool {
        self.raw == 0
    }

    pub fn is_internal(&self) -> bool {
        self.raw & 3 == 2
    }

    pub fn is_item(&self) -> bool {
        !self.is_null() && !self.is_internal()
    }

    pub fn is_node(&self) -> bool {
        self.is_internal() && self.raw > (SLOT_SIZE << 2)
    }

    pub fn from_item(item: I) -> Self {
        let raw = I::into_raw(item);
        unsafe { Self::new(raw as usize) }
    }

    pub fn into_item(self) -> Option<I> {
        if self.is_item() {
            let item = unsafe { I::from_raw(self.raw) };
            core::mem::forget(self);
            Some(item)
        } else {
            None
        }
    }

    pub fn from_node<Operation>(node: XNode<I, L, Operation>) -> Self {
        let node_ptr = {
            let arc_node = Arc::new(node);
            Arc::into_raw(arc_node)
        };
        unsafe { Self::new(node_ptr as usize | 2) }
    }

    pub fn as_node(&self) -> Option<&XNode<I, L>> {
        if self.is_node() {
            unsafe {
                let node_ref = &*((self.raw - 2) as *const XNode<I, L>);
                Some(node_ref)
            }
        } else {
            None
        }
    }

    pub fn as_node_mut<'a>(&self) -> Option<&'a XNode<I, L, ReadWrite>> {
        if self.is_node() {
            unsafe {
                let node_ref = &*((self.raw - 2) as *const XNode<I, L, ReadWrite>);
                Some(node_ref)
            }
        } else {
            None
        }
    }

    pub fn node_strong_count(&self) -> Option<usize> {
        if self.is_node() {
            let raw_ptr = (self.raw - 2) as *const u8;
            unsafe {
                let arc = Arc::from_raw(raw_ptr);
                let strong_count = Arc::strong_count(&arc);
                core::mem::forget(arc);
                Some(strong_count)
            }
        } else {
            None
        }
    }
}
