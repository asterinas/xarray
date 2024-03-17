use alloc::boxed::Box;
use alloc::sync::Arc;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, Not};

use crate::node::{TryClone, XNode};

/// A trait for the types users wish to store in an `XArray`.
///
/// Items stored in an `XArray` must be representable by a `usize` aligned to 4.
///
/// # Safety
///
/// Users must ensure that `into_raw()` always produce `usize`s that meet the above alignment
/// requirements.
///
/// Users must also ensure that as long as the value does not get dropped (e.g., by making use of
/// [`core::mem::ManuallyDrop`]), it is legal to invoke [`ItemEntry::from_raw`] multiple times on
/// the raw `usize` produced by invoking [`ItemEntry::into_raw`] only one time.
pub unsafe trait ItemEntry {
    /// Converts the original value into a `usize`, consuming the ownership of the original value.
    fn into_raw(self) -> usize;

    /// Recovers the original value from a `usize`, reclaiming the ownership of the original value.
    ///
    /// # Safety
    ///
    /// The original value must have not been dropped, and the raw value must be previously
    /// returned by [`ItemEntry::into_raw`].
    unsafe fn from_raw(raw: usize) -> Self;
}

// SAFETY: `Arc<T>` meets the safety requirements of `ItemEntry`.
unsafe impl<T> ItemEntry for Arc<T> {
    fn into_raw(self) -> usize {
        let raw_ptr = Arc::into_raw(self);
        raw_ptr as usize
    }

    // SAFETY: `Arc::<T>::from_raw` and `Arc::<T>::into_raw` meet the safety requirements of
    // `ItemEntry::from_raw`.
    unsafe fn from_raw(raw: usize) -> Self {
        unsafe { Arc::from_raw(raw as *mut _) }
    }
}

// SAFETY: `Box<T>` meets the safety requirements of `ItemEntry`.
unsafe impl<T> ItemEntry for Box<T> {
    fn into_raw(self) -> usize {
        let raw_ptr = Box::into_raw(self);
        raw_ptr as usize
    }

    // SAFETY: `Box::<T>::from_raw` and `Box::<T>::into_raw` meet the safety requirements of
    // `ItemEntry::from_raw`.
    unsafe fn from_raw(raw: usize) -> Self {
        unsafe { Box::from_raw(raw as *mut _) }
    }
}

/// A type that behaves exactly the same as `&I`.
///
/// This works around some implementation limitations where `&I` must be returned, but it is not
/// technically possible because the memory bits of the value are complexly encoded. Therefore a
/// wrapper type that represents `&I` comes to the rescue.
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

/// A type serving as the basic unit of storage for `XArray`s, used in the head of the `XArray` and
/// the slots of `XNode`s.
///
/// There are the following types of `XEntry`:
/// - Internal entries: These are invisible to users and have the last two bits set to 10.
/// - Item entries: These represent user-given items and have the last two bits set to 00.
///
/// An `XEntry` owns the item or node that it represents. Once an `XEntry` generated from an item
/// or an `XNode`, the ownership of the item or the `XNode` will be transferred to the `XEntry`.
///
/// An `XEntry` behaves just like the item or node it represents. Therefore, if the item type `I`
/// implements the [`Clone`] trait, the `XEntry` will also also implement the [`Clone`] trait.
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

    // SAFETY: `ptr` must be returned from `Arc::<XNode<I>>::into_raw` or `I::into_raw` and be
    // consistent with `ty`. In addition, the ownership of the value of `Arc<XNode<I>>` or `I` must
    // be transferred to the constructed instance of `XEntry`.
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
        // SAFETY: `node_ptr` is returned from `Arc::<Node<I>>::into_raw` and the ownership of the
        // value of `Arc<XNode<I>>` is transferred.
        unsafe { Self::new(node_ptr, EntryType::Node) }
    }

    pub fn is_node(&self) -> bool {
        self.ty() == Some(EntryType::Node)
    }

    pub fn as_node_ref(&self) -> Option<&XNode<I>> {
        if !self.is_node() {
            return None;
        }

        // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
        Some(unsafe { &*(self.ptr() as *const XNode<I>) })
    }

    pub fn as_node_maybe_mut(&mut self) -> Option<NodeMaybeMut<'_, I>> {
        match self.node_strong_count() {
            0 => None,
            // SAFETY: `&mut self` ensures the exclusive access to the value of `Arc<XNode<I>>`,
            // and `node_strong_count() == 1` ensures the exclusive access to the value of
            // `XNode<I>`.
            1 => Some(NodeMaybeMut::Exclusive(unsafe {
                &mut *(self.ptr() as *mut _)
            })),
            // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
            _ => Some(NodeMaybeMut::Shared(unsafe { &*(self.ptr() as *const _) })),
        }
    }

    pub fn as_node_mut_or_cow(&mut self) -> Option<&mut XNode<I>> {
        match self.node_strong_count() {
            0 => return None,
            // SAFETY: `&mut self` ensures the exclusive access to the value of `Arc<XNode<I>>`,
            // and `node_strong_count() == 1` ensures the exclusive access to the value of
            // `XNode<I>`.
            1 => return Some(unsafe { &mut *(self.ptr() as *mut _) }),
            _ => (),
        }

        // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
        let node = unsafe { &*(self.ptr() as *const XNode<I>) };
        let new_node = node.try_clone().unwrap();

        *self = Self::from_node(new_node);
        // SAFETY: `node_strong_count() == 1` now holds.
        Some(unsafe { &mut *(self.ptr() as *mut XNode<I>) })
    }

    fn node_strong_count(&self) -> usize {
        if !self.is_node() {
            return 0;
        }

        // SAFETY: `self` owns the value of `Arc<XNode<I>>` and the constructed instance of
        // `Arc<XNode<I>>` will not be dropped.
        let node = unsafe { ManuallyDrop::new(Arc::from_raw(self.ptr() as *const XNode<I>)) };
        Arc::strong_count(&*node)
    }
}

impl<I: ItemEntry> XEntry<I> {
    pub fn from_item(item: I) -> Self {
        let item_ptr = I::into_raw(item) as usize;
        // SAFETY: `item_ptr` is returned from `I::from_raw` and the ownership of the value of `I`
        // is transferred.
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

        // SAFETY: `self` owns the value of `I`.
        Some(unsafe { I::from_raw(ptr) })
    }

    pub fn as_item_ref(&self) -> Option<ItemRef<'_, I>> {
        if !self.is_item() {
            return None;
        }

        let ptr = self.ptr();

        // SAFETY: `self` owns the value of `I`, the constructed instance of `I` will not be
        // dropped, and `ItemRef` only allows shared access to the instance.
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
            // SAFETY: `self` owns the value of `I`.
            Some(EntryType::Item) => unsafe {
                I::from_raw(self.ptr());
            },
            // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
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
            // SAFETY: `self` owns the value of `I`, the constructed instance of `I` will not be
            // dropped, and `clone()` only takes a shared reference to the instance.
            Some(EntryType::Item) => unsafe {
                let item_entry = ManuallyDrop::new(I::from_raw(self.ptr()));
                Self::from_item((*item_entry).clone())
            },
            // SAFETY: `self` owns the value of `Arc<XNode<T>>`, and `Arc` can be cloned by
            // increasing its strong count.
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
