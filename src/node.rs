use core::cmp::Ordering;
use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::cow::Cow;
use crate::entry::{ItemEntry, XEntry};
use crate::lock::{MutexLock, XLock};
use crate::mark::Mark;
use crate::xarray::{BITS_PER_LAYER, SLOT_MASK, SLOT_SIZE};

pub(super) struct ReadOnly {}
pub(super) struct ReadWrite {}

/// The height of an XNode within an XArray.
///
/// In an XArray, the head has the highest height, while the XNodes that directly store items are at the lowest height,
/// with a height value of 1. Each level up from the bottom height increases the height number by 1.
/// The height of an XArray is the height of its head.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub(super) struct Height {
    height: u8,
}

impl Deref for Height {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.height
    }
}

impl DerefMut for Height {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.height
    }
}

impl PartialEq<u8> for Height {
    fn eq(&self, other: &u8) -> bool {
        self.height == *other
    }
}

impl PartialOrd<u8> for Height {
    fn partial_cmp(&self, other: &u8) -> Option<Ordering> {
        self.height.partial_cmp(other)
    }
}

impl Height {
    pub fn new(height: u8) -> Self {
        Self { height }
    }

    fn height_shift(&self) -> u8 {
        (self.height - 1) * BITS_PER_LAYER as u8
    }

    /// Calculate the corresponding offset for the target index at the current height.
    pub fn height_offset(&self, index: u64) -> u8 {
        ((index >> self.height_shift()) & SLOT_MASK as u64) as u8
    }

    /// Calculate the maximum index that can be represented in XArray at the current height.
    pub fn max_index(&self) -> u64 {
        ((SLOT_SIZE as u64) << self.height_shift()) - 1
    }
}

/// `XNode` is the intermediate node in the tree-like structure of XArray.
///
/// It contains `SLOT_SIZE` number of XEntries, meaning it can accommodate up to `SLOT_SIZE` child nodes.
/// The 'height' and 'offset_in_parent' attributes of an XNode are determined at initialization and remain unchanged thereafter.
///
/// XNode has a generic parameter called 'Operation', which has two possible instances: `ReadOnly` and `ReadWrite`.
/// These instances indicate whether the XNode will only perform read operations or both read and write operations
/// (where write operations imply potential modifications to the contents of slots).
pub(super) struct XNode<I, L, Operation = ReadOnly>
where
    I: ItemEntry,
    L: XLock,
{
    /// The height of the subtree rooted at the current node. The height of a leaf node,
    /// which stores the user-given items, is 1.
    height: Height,
    /// This node is its parent's `offset_in_parent`-th child.
    /// This field is meaningless if this node is the root (will be 0).
    offset_in_parent: u8,
    inner: L::Lock<XNodeInner<I, L>>,
    _marker: PhantomData<Operation>,
}

struct XNodeInner<I, L>
where
    I: ItemEntry,
    L: XLock,
{
    slots: [XEntry<I, L>; SLOT_SIZE],
    marks: [Mark; 3],
}

impl<I: ItemEntry, L: XLock, Operation> XNode<I, L, Operation> {
    pub fn new(height: Height, offset: u8) -> Self {
        Self {
            height,
            offset_in_parent: offset,
            inner: L::new(XNodeInner::new()),
            _marker: PhantomData,
        }
    }

    /// Get the offset in the slots of the current XNode corresponding to the XEntry for the target index.
    pub fn entry_offset(&self, target_index: u64) -> u8 {
        self.height.height_offset(target_index)
    }

    pub fn height(&self) -> Height {
        self.height
    }

    pub fn offset_in_parent(&self) -> u8 {
        self.offset_in_parent
    }

    pub fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.inner.lock().is_marked(offset, mark)
    }

    pub fn is_mark_clear(&self, mark: usize) -> bool {
        self.inner.lock().is_mark_clear(mark)
    }

    pub fn mark(&self, mark: usize) -> Mark {
        self.inner.lock().marks[mark]
    }

    pub fn is_leaf(&self) -> bool {
        self.height == 1
    }
}

impl<I: ItemEntry, L: XLock> XNode<I, L, ReadOnly> {
    /// Obtain a reference to the XEntry in the slots of the node. The input `offset` indicate
    /// the offset of the target XEntry in the slots.
    ///
    /// # Safety
    /// Users should ensure that no modifications for slots are made to the current XNode
    /// while the reference to the returned XEntry exists.
    pub unsafe fn ref_node_entry(&self, offset: u8) -> &XEntry<I, L> {
        let lock = self.inner.lock();

        let entry_ptr = &lock.slots[offset as usize] as *const XEntry<I, L>;
        unsafe { &*entry_ptr }
    }
}

impl<I: ItemEntry, L: XLock> XNode<I, L, ReadWrite> {
    /// Obtain a reference to the XEntry in the slots of the node. The input `offset` indicate
    /// the offset of target XEntry in the slots.
    ///
    /// # Safety
    /// Users should ensure that no modifications for slots are made to the current XNode
    /// while the reference to the returned XEntry exists.
    pub unsafe fn ref_node_entry(&self, is_exclusive: bool, offset: u8) -> &XEntry<I, L> {
        let mut lock = self.inner.lock();

        // When a modification to the target entry is needed, it first checks whether the entry is shared with other XArrays.
        // If it is, then it performs COW by allocating a new entry and using it,
        // to prevent the modification from affecting the read or write operations on other XArrays.
        if is_exclusive {
            if let Some(new_entry) = lock.slots[offset as usize].copy_if_shared() {
                lock.set_entry(offset, new_entry);
            }
        }
        let entry_ptr = &lock.slots[offset as usize] as *const XEntry<I, L>;
        unsafe { &*entry_ptr }
    }

    pub fn set_entry(&self, offset: u8, entry: XEntry<I, L>) -> XEntry<I, L> {
        self.inner.lock().set_entry(offset, entry)
    }

    pub fn set_mark(&self, offset: u8, mark: usize) {
        self.inner.lock().set_mark(offset, mark)
    }

    pub fn unset_mark(&self, offset: u8, mark: usize) {
        self.inner.lock().unset_mark(offset, mark)
    }

    pub fn clear_mark(&self, mark: usize) {
        self.inner.lock().clear_mark(mark)
    }
}

impl<I: ItemEntry, L: XLock> XNodeInner<I, L> {
    fn new() -> Self {
        Self {
            slots: [XEntry::EMPTY; SLOT_SIZE],
            marks: [Mark::EMPTY; 3],
        }
    }

    fn set_entry(&mut self, offset: u8, entry: XEntry<I, L>) -> XEntry<I, L> {
        for i in 0..3 {
            self.marks[i].unset(offset);
        }
        let old_entry = core::mem::replace(&mut self.slots[offset as usize], entry);
        old_entry
    }

    fn set_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].set(offset);
    }

    fn unset_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].unset(offset);
    }

    fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.marks[mark].is_marked(offset)
    }

    fn is_mark_clear(&self, mark: usize) -> bool {
        self.marks[mark].is_clear()
    }

    fn clear_mark(&mut self, mark: usize) {
        self.marks[mark].clear();
    }
}

pub(super) fn deep_copy_node_entry<I: ItemEntry + Clone, L: XLock>(
    entry: &XEntry<I, L>,
) -> XEntry<I, L> {
    debug_assert!(entry.is_node());
    let new_node = {
        let cloned_node: &XNode<I, L> = entry.as_node().unwrap();
        let new_node =
            XNode::<I, L, ReadWrite>::new(cloned_node.height(), cloned_node.offset_in_parent());
        let mut new_node_lock = new_node.inner.lock();
        let cloned_node_lock = cloned_node.inner.lock();
        new_node_lock.marks = cloned_node_lock.marks;
        for i in 0..SLOT_SIZE {
            let entry = &cloned_node_lock.slots[i];
            let new_entry = entry.clone();
            new_node_lock.slots[i as usize] = new_entry;
        }
        drop(new_node_lock);
        new_node
    };
    XEntry::from_node(new_node)
}
