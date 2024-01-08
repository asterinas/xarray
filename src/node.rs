use std::{marker::PhantomData, sync::Mutex};

use crate::*;

pub(crate) struct ReadOnly {}
pub(crate) struct ReadWrite {}

/// `XNode` is the intermediate node in the tree-like structure of XArray.
///
/// It contains `SLOT_SIZE` number of XEntries, meaning it can accommodate up to `SLOT_SIZE` child nodes.
/// The 'height' and 'offset' attributes of an XNode are determined at initialization and remain unchanged thereafter.
/// XNode has a generic parameter called 'Operation', which has two possible instances: `ReadOnly` and `ReadWrite`.
/// These instances indicate whether the XNode will only perform read operations or both read and write operations
/// (where write operations imply potential modifications to the contents of slots).
pub(crate) struct XNode<I: ItemEntry, Operation = ReadOnly> {
    /// The node's height from the bottom of the tree. The height of a lead node,
    /// which stores the user-given items, is zero.
    height: u8,
    /// This node is its parent's `offset_in_parent`-th child.
    /// This field is meaningless if this node is the root (will be 0).
    offset_in_parent: u8,
    inner: Mutex<XNodeInner<I>>,
    _marker: PhantomData<Operation>,
}

pub(crate) struct XNodeInner<I: ItemEntry> {
    slots: [XEntry<I>; SLOT_SIZE],
}

impl<I: ItemEntry, Operation> XNode<I, Operation> {
    pub(crate) fn new(height: u8, offset: u8) -> Self {
        Self {
            height,
            offset_in_parent: offset,
            inner: Mutex::new(XNodeInner::new()),
            _marker: PhantomData,
        }
    }

    /// Get the offset in the slots of the current XNode corresponding to the XEntry for the target index.
    pub(crate) const fn entry_offset(&self, target_index: u64) -> u8 {
        ((target_index >> self.height as u64) & SLOT_MASK as u64) as u8
    }

    /// Get the max index the XNode and its child nodes can store.
    pub(crate) fn max_index(&self) -> u64 {
        ((SLOT_SIZE as u64) << (self.height as u64)) - 1
    }

    pub(crate) fn height(&self) -> u8 {
        self.height
    }

    pub(crate) fn offset_in_parent(&self) -> u8 {
        self.offset_in_parent
    }
}

impl<I: ItemEntry> XNode<I, ReadOnly> {
    pub(crate) fn entry<'a>(&'a self, offset: u8) -> RefEntry<'a, I> {
        let lock = self.inner.lock().unwrap();
        let entry = lock.entry(offset);
        RefEntry::new(entry)
    }
}

impl<I: ItemEntry> XNode<I, ReadWrite> {
    pub(crate) fn entry<'a>(&'a self, offset: u8) -> RefEntry<'a, I> {
        let mut lock = self.inner.lock().unwrap();
        let entry = lock.entry_mut(offset);
        RefEntry::new(entry)
    }

    pub(crate) fn set_entry(&self, offset: u8, entry: XEntry<I>) -> XEntry<I> {
        self.inner.lock().unwrap().set_entry(offset, entry)
    }
}

impl<I: ItemEntry> XNodeInner<I> {
    pub(crate) fn new() -> Self {
        Self {
            slots: [XEntry::EMPTY; SLOT_SIZE],
        }
    }

    pub(crate) fn entry(&self, offset: u8) -> &XEntry<I> {
        &self.slots[offset as usize]
    }

    pub(crate) fn entry_mut(&mut self, offset: u8) -> &XEntry<I> {
        // When a modification to the target entry is needed, it first checks whether the entry is shared with other XArrays.
        // If it is, then it performs a copy-on-write by allocating a new entry and using it,
        // to prevent the modification from affecting the read or write operations on other XArrays.
        self.copy_on_write(
            unsafe { &*(&self.slots[offset as usize] as *const XEntry<I>) },
            offset,
        )
    }

    pub(crate) fn set_entry(&mut self, offset: u8, entry: XEntry<I>) -> XEntry<I> {
        let old_entry = core::mem::replace(&mut self.slots[offset as usize], entry);
        old_entry
    }
}

pub(crate) fn deep_clone_node_entry<I: ItemEntry + Clone>(entry: &XEntry<I>) -> XEntry<I> {
    debug_assert!(entry.is_node());
    let new_node = {
        let cloned_node: &XNode<I> = entry.as_node().unwrap();
        let new_node =
            XNode::<I, ReadWrite>::new(cloned_node.height(), cloned_node.offset_in_parent());
        let mut new_node_lock = new_node.inner.lock().unwrap();
        let cloned_node_lock = cloned_node.inner.lock().unwrap();
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
