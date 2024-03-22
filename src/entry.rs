use alloc::boxed::Box;
use alloc::sync::Arc;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, Not};

use crate::node::{TryClone, XNode};

/// A trait for the types users wish to store in an `XArray`.
///
/// Items stored in an `XArray` must be representable by a `*const ()` aligned to 4. We prefer
/// `*const ()` than `usize` to make the implementation conform to [Strict Provenance][1].
///
///  [1]: https://doc.rust-lang.org/std/ptr/index.html#strict-provenance
///
/// # Safety
///
/// Users must ensure that [`ItemEntry::into_raw`] always produce `*const ()`s that meet the above
/// alignment requirements.
///
/// Users must also ensure that as long as the value does not get dropped (e.g., by dropping the
/// value obtaining from [`ItemEntry::from_raw`]), it is safe to invoke [`ItemEntry::raw_as_ref`]
/// multiple times to obtain values of [`ItemEntry::Ref`] that behave just like shared references
/// to the underleying data.
pub unsafe trait ItemEntry {
    /// A type that behaves just like a shared references to the underleying data.
    type Ref<'a>: Deref<Target = Self>
    where
        Self: 'a;

    /// Converts the original value into a `*const ()`, consuming the ownership of the original
    /// value.
    fn into_raw(self) -> *const ();

    /// Recovers the original value from a `*const ()`, reclaiming the ownership of the original
    /// value.
    ///
    /// # Safety
    ///
    /// The original value must have not been dropped, and all references obtained from
    /// [`ItemEntry::raw_as_ref`] must be dead.
    unsafe fn from_raw(raw: *const ()) -> Self;

    /// Obtains a shared reference to the original value.
    ///
    /// # Safety
    ///
    /// The original value must outlive the lifetime parameter `'a`, and during `'a` no mutable
    /// references to the value will exist.
    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a>;
}

/// A type that represents `&'a Arc<T>`.
#[derive(PartialEq, Debug)]
pub struct ArcRef<'a, T> {
    inner: ManuallyDrop<Arc<T>>,
    _marker: PhantomData<&'a Arc<T>>,
}

impl<'a, T> Deref for ArcRef<'a, T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

// SAFETY: `Arc<T>` meets the safety requirements of `ItemEntry`.
unsafe impl<T> ItemEntry for Arc<T> {
    type Ref<'a> = ArcRef<'a, T> where Self: 'a;

    fn into_raw(self) -> *const () {
        // A contant expression, so compilers should be smart enough to optimize it away.
        assert!((core::mem::align_of::<T>() & XEntry::<Self>::TYPE_MASK) == 0);

        Arc::into_raw(self).cast()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        // SAFETY: By the safety requirements of `ItemEntry::from_raw`, the original value has not
        // been dropped and there are no outstanding references to it. Thus, the ownership of the
        // original value can be taken.
        unsafe { Arc::from_raw(raw.cast()) }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        // SAFETY: By the safety requirements of `ItemEntry::raw_as_ref`, the original value
        // outlives the lifetime parameter `'a` and during `'a` no mutable references to it can
        // exist. Thus, a shared reference to the original value can be created.
        unsafe {
            ArcRef {
                inner: ManuallyDrop::new(Arc::from_raw(raw.cast())),
                _marker: PhantomData,
            }
        }
    }
}

/// A type that represents `&'a Box<T>`.
#[derive(PartialEq, Debug)]
pub struct BoxRef<'a, T> {
    inner: *mut T,
    _marker: PhantomData<&'a Box<T>>,
}

impl<'a, T> Deref for BoxRef<'a, T> {
    type Target = Box<T>;

    fn deref(&self) -> &Self::Target {
        // SAFETY: A `Box<T>` is guaranteed to be represented by a single pointer [1] and a shared
        // reference to the `Box<T>` during the lifetime `'a` can be created according to the
        // safety requirements of `ItemEntry::raw_as_ref`.
        //
        // [1]: https://doc.rust-lang.org/std/boxed/#memory-layout
        unsafe { core::mem::transmute(&self.inner) }
    }
}

// SAFETY: `Box<T>` meets the safety requirements of `ItemEntry`.
unsafe impl<T> ItemEntry for Box<T> {
    type Ref<'a> = BoxRef<'a, T> where Self: 'a;

    fn into_raw(self) -> *const () {
        // A contant expression, so compilers should be smart enough to optimize it away.
        assert!((core::mem::align_of::<T>() & XEntry::<Self>::TYPE_MASK) == 0);

        Box::into_raw(self).cast_const().cast()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        // SAFETY: By the safety requirements of `ItemEntry::from_raw`, the original value has not
        // been dropped and there are no outstanding references to it. Thus, the ownership of the
        // original value can be taken.
        unsafe { Box::from_raw(raw.cast_mut().cast()) }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        BoxRef {
            inner: raw.cast_mut().cast(),
            _marker: PhantomData,
        }
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
    raw: *const (),
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
        raw: core::ptr::null(),
        _marker: PhantomData,
    };

    // SAFETY: `ptr` must be returned from `Arc::<XNode<I>>::into_raw` or `I::into_raw` and be
    // consistent with `ty`. In addition, the ownership of the value of `Arc<XNode<I>>` or `I` must
    // be transferred to the constructed instance of `XEntry`.
    unsafe fn new(ptr: *const (), ty: EntryType) -> Self {
        let raw = ptr.map_addr(|addr| {
            debug_assert!(addr & Self::TYPE_MASK == 0);
            addr | (ty as usize)
        });
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    fn ptr(&self) -> *const () {
        self.raw.map_addr(|addr| addr & !Self::TYPE_MASK)
    }

    fn ty(&self) -> Option<EntryType> {
        self.is_null()
            .not()
            .then(|| (self.raw.addr() & Self::TYPE_MASK).try_into().unwrap())
    }

    pub fn is_null(&self) -> bool {
        self.raw.is_null()
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
            Arc::into_raw(arc_node).cast()
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
        Some(unsafe { &*self.ptr().cast() })
    }

    pub fn as_node_maybe_mut(&mut self) -> Option<NodeMaybeMut<'_, I>> {
        match self.node_strong_count() {
            0 => None,
            // SAFETY: `&mut self` ensures the exclusive access to the value of `Arc<XNode<I>>`,
            // and `node_strong_count() == 1` ensures the exclusive access to the value of
            // `XNode<I>`.
            1 => Some(NodeMaybeMut::Exclusive(unsafe {
                &mut *self.ptr().cast_mut().cast()
            })),
            // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
            _ => Some(NodeMaybeMut::Shared(unsafe { &*self.ptr().cast() })),
        }
    }

    pub fn as_node_mut_or_cow(&mut self) -> Option<&mut XNode<I>> {
        match self.node_strong_count() {
            0 => return None,
            // SAFETY: `&mut self` ensures the exclusive access to the value of `Arc<XNode<I>>`,
            // and `node_strong_count() == 1` ensures the exclusive access to the value of
            // `XNode<I>`.
            1 => return Some(unsafe { &mut *self.ptr().cast_mut().cast() }),
            _ => (),
        }

        // SAFETY: `self` owns the value of `Arc<XNode<I>>`.
        let node: &XNode<I> = unsafe { &*self.ptr().cast() };
        let new_node = node.try_clone().unwrap();

        *self = Self::from_node(new_node);
        // SAFETY: `node_strong_count() == 1` now holds.
        Some(unsafe { &mut *self.ptr().cast_mut().cast() })
    }

    fn node_strong_count(&self) -> usize {
        if !self.is_node() {
            return 0;
        }

        // SAFETY: `self` owns the value of `Arc<XNode<I>>` and the constructed instance of
        // `Arc<XNode<I>>` will not be dropped.
        let node = unsafe { ManuallyDrop::new(Arc::<XNode<I>>::from_raw(self.ptr().cast())) };
        Arc::strong_count(&*node)
    }
}

impl<I: ItemEntry> XEntry<I> {
    pub fn from_item(item: I) -> Self {
        let item_ptr = I::into_raw(item);
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

    pub fn as_item_ref(&self) -> Option<I::Ref<'_>> {
        if !self.is_item() {
            return None;
        }

        let ptr = self.ptr();

        // SAFETY: `self` owns the value of `I` and does not create any mutable references to the
        // value. Thus, the value of `I` outlives the lifetime of `&self`.
        Some(unsafe { I::raw_as_ref(ptr) })
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
                Arc::<XNode<I>>::from_raw(self.ptr().cast());
            },
        }
    }
}

impl<I: ItemEntry + Clone> Clone for XEntry<I> {
    fn clone(&self) -> Self {
        match self.ty() {
            None => Self::EMPTY,
            Some(EntryType::Item) => {
                // SAFETY: `self` owns the value of `I` and does not create any mutable references to the
                // value. Thus, the value of `I` outlives the lifetime of `&self`.
                let item_entry = unsafe { I::raw_as_ref(self.ptr()) };
                Self::from_item((*item_entry).clone())
            }
            // SAFETY: `self` owns the value of `Arc<XNode<T>>`, and `Arc` can be cloned by
            // increasing its strong count.
            Some(EntryType::Node) => unsafe {
                Arc::<XNode<I>>::increment_strong_count(self.ptr().cast());
                Self {
                    raw: self.raw,
                    _marker: PhantomData,
                }
            },
        }
    }
}
