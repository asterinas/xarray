use alloc::boxed::Box;
use alloc::sync::Arc;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, Not};

use crate::node::{TryClone, XNode};

/// A trait that should be implemented for the types users wish to store in an `XArray`.
/// Items stored in an XArray are required to be 8 bytes in size, Currently it can be various pointer types.
///
/// # Safety
/// Users must ensure that the produced `usize` of `into_raw()` meets the requirements for an item entry
/// in the XArray. Specifically, if the original type is a pointer, the last two bits should be 00;
/// if the original type is a value like usize, the last bit should be 1 (TODO).
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
        let raw_ptr = Arc::into_raw(self);
        debug_assert!(raw_ptr.is_aligned_to(4));
        raw_ptr as usize
    }

    unsafe fn from_raw(raw: usize) -> Self {
        unsafe { Arc::from_raw(raw as *mut T) }
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

#[derive(PartialEq, Debug)]
pub struct ItemRef<'a, I>
where
    I: ItemEntry,
{
    item: ManuallyDrop<I>,
    _marker: PhantomData<&'a I>,
}

impl<'a, I: ItemEntry> Deref for ItemRef<'a, I> {
    type Target = I;

    fn deref(&self) -> &I {
        &*self.item
    }
}

/// The type stored in the head of `XArray` and the slots of `XNode`s, which is the basic unit of storage
/// within an XArray.
///
/// There are the following types of `XEntry`:
/// - Internal entries: These are invisible to users and have the last two bits set to 10. Currently `XArray`
/// only have node entries as internal entries, which are entries that point to XNodes.
/// - Item entries: Items stored by the user. Currently stored items can only be pointers and the last two bits
/// of these item entries are 00.
///
/// `XEntry` have the ownership. Once it generated from an item or a XNode, the ownership of the item or the XNode
/// will be transferred to the `XEntry`. If the stored item in the XArray implemented Clone trait, then the XEntry
/// in the XArray can also implement Clone trait.
#[derive(PartialEq, Eq, Debug)]
#[repr(transparent)]
pub(super) struct XEntry<I>
where
    I: ItemEntry,
{
    raw: usize,
    _marker: core::marker::PhantomData<(Arc<XNode<I>>, I)>,
}

#[derive(PartialEq, Eq, Debug)]
#[repr(usize)]
enum EntryType {
    Node = 0,
    Item = 2,
}

impl TryFrom<usize> for EntryType {
    type Error = ();

    fn try_from(val: usize) -> Result<Self, Self::Error> {
        match val {
            x if x == EntryType::Node as usize => Ok(EntryType::Node),
            x if x == EntryType::Item as usize => Ok(EntryType::Item),
            _ => Err(()),
        }
    }
}

impl<I: ItemEntry> XEntry<I> {
    const TYPE_MASK: usize = 3;

    pub const EMPTY: Self = Self {
        raw: 0,
        _marker: PhantomData,
    };

    unsafe fn new(ptr: usize, ty: EntryType) -> Self {
        debug_assert!(ptr & Self::TYPE_MASK == 0);
        Self {
            raw: ptr | (ty as usize),
            _marker: PhantomData,
        }
    }

    fn ptr(&self) -> usize {
        self.raw & !Self::TYPE_MASK
    }

    fn ty(&self) -> Option<EntryType> {
        self.is_null()
            .not()
            .then(|| (self.raw & Self::TYPE_MASK).try_into().unwrap())
    }

    pub fn is_null(&self) -> bool {
        self.raw == 0
    }
}

pub(super) enum NodeMaybeMut<'a, I>
where
    I: ItemEntry,
{
    Shared(&'a XNode<I>),
    Exclusive(&'a mut XNode<I>),
}

impl<'a, I: ItemEntry> Deref for NodeMaybeMut<'a, I> {
    type Target = XNode<I>;

    fn deref(&self) -> &XNode<I> {
        match &self {
            Self::Shared(ref node) => node,
            Self::Exclusive(ref node) => node,
        }
    }
}

impl<I: ItemEntry> XEntry<I> {
    pub fn from_node(node: XNode<I>) -> Self {
        let node_ptr = {
            let arc_node = Arc::new(node);
            Arc::into_raw(arc_node) as usize
        };
        unsafe { Self::new(node_ptr, EntryType::Node) }
    }

    pub fn is_node(&self) -> bool {
        self.ty() == Some(EntryType::Node)
    }

    pub fn as_node_ref(&self) -> Option<&XNode<I>> {
        if !self.is_node() {
            return None;
        }

        Some(unsafe { &*(self.ptr() as *const XNode<I>) })
    }

    pub fn as_node_maybe_mut(&mut self) -> Option<NodeMaybeMut<'_, I>> {
        match self.node_strong_count() {
            0 => None,
            1 => Some(NodeMaybeMut::Exclusive(unsafe {
                &mut *(self.ptr() as *mut _)
            })),
            _ => Some(NodeMaybeMut::Shared(unsafe { &*(self.ptr() as *const _) })),
        }
    }

    pub fn as_node_mut_or_cow(&mut self) -> Option<&mut XNode<I>> {
        match self.node_strong_count() {
            0 => return None,
            1 => return Some(unsafe { &mut *(self.ptr() as *mut _) }),
            _ => (),
        }

        let node = unsafe { &*(self.ptr() as *const XNode<I>) };
        let new_node = node.try_clone().unwrap();

        *self = Self::from_node(new_node);
        Some(unsafe { &mut *(self.ptr() as *mut XNode<I>) })
    }

    fn node_strong_count(&self) -> usize {
        if !self.is_node() {
            return 0;
        }

        let node = unsafe { ManuallyDrop::new(Arc::from_raw(self.ptr() as *const XNode<I>)) };
        Arc::strong_count(&*node)
    }
}

impl<I: ItemEntry> XEntry<I> {
    pub fn from_item(item: I) -> Self {
        let item_ptr = I::into_raw(item) as usize;
        unsafe { Self::new(item_ptr, EntryType::Item) }
    }

    pub fn is_item(&self) -> bool {
        self.ty() == Some(EntryType::Item)
    }

    pub fn into_item(self) -> Option<I> {
        if !self.is_item() {
            return None;
        }

        let ptr = self.ptr();
        core::mem::forget(self);

        Some(unsafe { I::from_raw(ptr) })
    }

    pub fn as_item_ref(&self) -> Option<ItemRef<'_, I>> {
        if !self.is_item() {
            return None;
        }

        let ptr = self.ptr();

        Some(ItemRef {
            item: unsafe { ManuallyDrop::new(I::from_raw(ptr)) },
            _marker: PhantomData,
        })
    }
}

impl<I: ItemEntry> Drop for XEntry<I> {
    fn drop(&mut self) {
        match self.ty() {
            None => (),
            Some(EntryType::Item) => unsafe {
                I::from_raw(self.ptr());
            },
            Some(EntryType::Node) => unsafe {
                Arc::from_raw(self.ptr() as *const XNode<I>);
            },
        }
    }
}

impl<I: ItemEntry + Clone> Clone for XEntry<I> {
    fn clone(&self) -> Self {
        match self.ty() {
            None => Self::EMPTY,
            Some(EntryType::Item) => unsafe {
                let item_entry = ManuallyDrop::new(I::from_raw(self.ptr()));
                Self::from_item((*item_entry).clone())
            },
            Some(EntryType::Node) => unsafe {
                Arc::increment_strong_count(self.ptr() as *const XNode<I>);
                Self {
                    raw: self.raw,
                    _marker: PhantomData,
                }
            },
        }
    }
}
