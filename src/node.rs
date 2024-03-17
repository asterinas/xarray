use core::cmp::Ordering;
use core::ops::{Deref, DerefMut};

use crate::entry::{ItemEntry, XEntry};
use crate::mark::{Mark, NUM_MARKS};
use crate::xarray::{BITS_PER_LAYER, SLOT_MASK, SLOT_SIZE};

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

    pub fn from_index(index: u64) -> Self {
        let mut height = Height::new(1);
        while index > height.max_index() {
            *height += 1;
        }
        height
    }

    pub fn go_root(&self) -> Self {
        Self::new(self.height + 1)
    }

    pub fn go_leaf(&self) -> Self {
        Self::new(self.height - 1)
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
#[derive(Clone, Debug)]
pub(super) struct XNode<I>
where
    I: ItemEntry,
{
    /// The height of the subtree rooted at the current node. The height of a leaf node,
    /// which stores the user-given items, is 1.
    height: Height,
    /// This node is its parent's `offset_in_parent`-th child.
    /// This field is meaningless if this node is the root (will be 0).
    offset_in_parent: u8,
    slots: [XEntry<I>; SLOT_SIZE],
    marks: [Mark; NUM_MARKS],
}

impl<I: ItemEntry> XNode<I> {
    pub fn new_root(height: Height) -> Self {
        Self::new(height, 0)
    }

    pub fn new(height: Height, offset: u8) -> Self {
        Self {
            height,
            offset_in_parent: offset,
            slots: [XEntry::EMPTY; SLOT_SIZE],
            marks: [Mark::EMPTY; NUM_MARKS],
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

    pub fn entry(&self, offset: u8) -> &XEntry<I> {
        &self.slots[offset as usize]
    }

    pub fn entry_mut(&mut self, offset: u8) -> &mut XEntry<I> {
        &mut self.slots[offset as usize]
    }

    pub fn entries_mut(&mut self) -> &mut [XEntry<I>] {
        &mut self.slots
    }

    pub fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.marks[mark].is_marked(offset)
    }

    pub fn is_mark_clear(&self, mark: usize) -> bool {
        self.marks[mark].is_clear()
    }

    pub fn mark(&self, mark: usize) -> Mark {
        self.marks[mark]
    }

    pub fn is_leaf(&self) -> bool {
        self.height == 1
    }

    pub fn set_entry(&mut self, offset: u8, entry: XEntry<I>) -> XEntry<I> {
        let is_new_node = entry.is_node();

        let old_entry = core::mem::replace(&mut self.slots[offset as usize], entry);

        if is_new_node {
            self.update_mark(offset);
            return old_entry;
        }

        for i in 0..NUM_MARKS {
            self.marks[i].unset(offset);
        }
        old_entry
    }

    pub fn set_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].set(offset);
    }

    pub fn unset_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].unset(offset);
    }

    pub fn clear_mark(&mut self, mark: usize) {
        self.marks[mark].clear();
    }

    pub fn update_mark(&mut self, offset: u8) -> bool {
        let Some(node) = self.slots[offset as usize].as_node_ref() else {
            return false;
        };

        let mut changed = false;
        for i in 0..NUM_MARKS {
            changed |= self.marks[i].update(offset, !node.is_mark_clear(i));
        }
        changed
    }
}

pub(super) trait TryClone
where
    Self: Sized,
{
    fn try_clone(&self) -> Option<Self>;
}

impl<I: ItemEntry> TryClone for XNode<I> {
    default fn try_clone(&self) -> Option<Self> {
        None
    }
}

impl<I: ItemEntry + Clone> TryClone for XNode<I> {
    fn try_clone(&self) -> Option<Self> {
        Some(self.clone())
    }
}
