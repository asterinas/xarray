use core::cmp::Ordering;
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, Weak},
};

use crate::*;

pub(crate) struct ReadOnly {}
pub(crate) struct ReadWrite {}

/// The layer of an XNode within an XArray.
///
/// In an XArray, the head has the highest layer, while the XNodes that directly store items are at the lowest layer,
/// with a layer value of 0. Each level up from the bottom layer increases the layer number by 1.
/// The layer of an XArray is the layer of its head.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub(crate) struct Layer {
    layer: u8,
}

impl Deref for Layer {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.layer
    }
}

impl DerefMut for Layer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.layer
    }
}

impl PartialEq<u8> for Layer {
    fn eq(&self, other: &u8) -> bool {
        self.layer == *other
    }
}

impl PartialOrd<u8> for Layer {
    fn partial_cmp(&self, other: &u8) -> Option<Ordering> {
        self.layer.partial_cmp(other)
    }
}

impl Layer {
    pub(crate) fn new(layer: u8) -> Self {
        Self { layer }
    }

    fn layer_shift(&self) -> u8 {
        self.layer * BITS_PER_LAYER as u8
    }

    /// Calculate the corresponding offset for the target index at the current layer.
    pub(crate) fn layer_offset(&self, index: u64) -> u8 {
        ((index >> self.layer_shift()) & SLOT_MASK as u64) as u8
    }

    /// Calculate the maximum index that can be represented in XArray at the current layer.
    pub(crate) fn max_index(&self) -> u64 {
        ((SLOT_SIZE as u64) << self.layer_shift()) - 1
    }
}

/// `XNode` is the intermediate node in the tree-like structure of XArray.
///
/// It contains `SLOT_SIZE` number of XEntries, meaning it can accommodate up to `SLOT_SIZE` child nodes.
/// The 'layer' and 'offset_in_parent' attributes of an XNode are determined at initialization and remain unchanged thereafter.
///
/// XNode has a generic parameter called 'Operation', which has two possible instances: `ReadOnly` and `ReadWrite`.
/// These instances indicate whether the XNode will only perform read operations or both read and write operations
/// (where write operations imply potential modifications to the contents of slots).
pub(crate) struct XNode<I: ItemEntry, Operation = ReadOnly> {
    /// The node's layer from the bottom of the tree. The layer of a lead node,
    /// which stores the user-given items, is 0.
    layer: Layer,
    /// This node is its parent's `offset_in_parent`-th child.
    /// This field is meaningless if this node is the root (will be 0).
    offset_in_parent: u8,
    inner: Mutex<XNodeInner<I>>,
    _marker: PhantomData<Operation>,
}

pub(crate) struct XNodeInner<I: ItemEntry> {
    parent: Option<Weak<XNode<I, ReadWrite>>>,
    slots: [XEntry<I>; SLOT_SIZE],
    marks: [Mark; 3],
}

impl<I: ItemEntry, Operation> XNode<I, Operation> {
    pub(crate) fn new(layer: Layer, offset: u8, parent: Option<Weak<XNode<I, ReadWrite>>>) -> Self {
        Self {
            layer,
            offset_in_parent: offset,
            inner: Mutex::new(XNodeInner::new(parent)),
            _marker: PhantomData,
        }
    }

    /// Get the offset in the slots of the current XNode corresponding to the XEntry for the target index.
    pub(crate) fn entry_offset(&self, target_index: u64) -> u8 {
        self.layer.layer_offset(target_index)
    }

    pub(crate) fn layer(&self) -> Layer {
        self.layer
    }

    pub(crate) fn offset_in_parent(&self) -> u8 {
        self.offset_in_parent
    }

    pub(crate) fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.inner.lock().unwrap().is_marked(offset, mark)
    }

    pub(crate) fn is_mark_clear(&self, mark: usize) -> bool {
        self.inner.lock().unwrap().is_mark_clear(mark)
    }

    pub(crate) fn mark(&self, mark: usize) -> Mark {
        self.inner.lock().unwrap().marks[mark]
    }
}

impl<I: ItemEntry> XNode<I, ReadOnly> {
    pub(crate) fn parent(&self) -> Option<&XNode<I, ReadOnly>> {
        self.inner
            .lock()
            .unwrap()
            .parent
            .as_ref()
            .map(|parent| unsafe { &*(parent.as_ptr() as *const XNode<I, ReadOnly>) })
    }

    pub(crate) fn entry<'a>(&'a self, offset: u8) -> *const XEntry<I> {
        let lock = self.inner.lock().unwrap();
        let entry = lock.entry(offset);
        entry
    }
}

impl<I: ItemEntry> XNode<I, ReadWrite> {
    pub(crate) fn parent(&self) -> Option<&XNode<I, ReadWrite>> {
        self.inner
            .lock()
            .unwrap()
            .parent
            .as_ref()
            .map(|parent| unsafe { &*(parent.as_ptr()) })
    }

    pub(crate) fn entry<'a>(&'a self, offset: u8) -> *const XEntry<I> {
        let mut lock = self.inner.lock().unwrap();
        let entry = lock.entry_mut(offset);
        entry
    }

    pub(crate) fn set_parent(&self, parent: &XNode<I, ReadWrite>) {
        let parent = {
            let arc = unsafe { Arc::from_raw(parent as *const XNode<I, ReadWrite>) };
            let weak = Arc::downgrade(&arc);
            core::mem::forget(arc);
            weak
        };
        self.inner.lock().unwrap().parent = Some(parent);
    }

    pub(crate) fn set_entry(&self, offset: u8, entry: XEntry<I>) -> XEntry<I> {
        self.inner.lock().unwrap().set_entry(offset, entry)
    }

    pub(crate) fn set_mark(&self, offset: u8, mark: usize) {
        self.inner.lock().unwrap().set_mark(offset, mark)
    }

    pub(crate) fn unset_mark(&self, offset: u8, mark: usize) {
        self.inner.lock().unwrap().unset_mark(offset, mark)
    }

    pub(crate) fn clear_mark(&self, mark: usize) {
        self.inner.lock().unwrap().clear_mark(mark)
    }
}

impl<I: ItemEntry> XNodeInner<I> {
    pub(crate) fn new(parent: Option<Weak<XNode<I, ReadWrite>>>) -> Self {
        Self {
            parent,
            slots: [XEntry::EMPTY; SLOT_SIZE],
            marks: [Mark::EMPTY; 3],
        }
    }

    pub(crate) fn entry(&self, offset: u8) -> *const XEntry<I> {
        &self.slots[offset as usize] as *const XEntry<I>
    }

    pub(crate) fn entry_mut(&mut self, offset: u8) -> *const XEntry<I> {
        // When a modification to the target entry is needed, it first checks whether the entry is shared with other XArrays.
        // If it is, then it performs COW by allocating a new entry and using it,
        // to prevent the modification from affecting the read or write operations on other XArrays.
        if let Some(new_entry) = self.copy_if_shared(&self.slots[offset as usize]) {
            self.set_entry(offset, new_entry);
        }
        &self.slots[offset as usize] as *const XEntry<I>
    }

    pub(crate) fn set_entry(&mut self, offset: u8, entry: XEntry<I>) -> XEntry<I> {
        let old_entry = core::mem::replace(&mut self.slots[offset as usize], entry);
        old_entry
    }

    pub(crate) fn set_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].set(offset);
    }

    pub(crate) fn unset_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].unset(offset);
    }

    pub(crate) fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.marks[mark].is_marked(offset)
    }

    pub(crate) fn is_mark_clear(&self, mark: usize) -> bool {
        self.marks[mark].is_clear()
    }

    pub(crate) fn clear_mark(&mut self, mark: usize) {
        self.marks[mark].clear();
    }
}

pub(crate) fn deep_clone_node_entry<I: ItemEntry + Clone>(entry: &XEntry<I>) -> XEntry<I> {
    debug_assert!(entry.is_node());
    let new_node = {
        let cloned_node: &XNode<I> = entry.as_node().unwrap();
        let new_node = XNode::<I, ReadWrite>::new(
            cloned_node.layer(),
            cloned_node.offset_in_parent(),
            cloned_node.inner.lock().unwrap().parent.clone(),
        );
        let mut new_node_lock = new_node.inner.lock().unwrap();
        let cloned_node_lock = cloned_node.inner.lock().unwrap();
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
