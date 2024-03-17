use core::cmp::Ordering;
use core::ops::{Deref, DerefMut};

use crate::entry::{ItemEntry, XEntry};
use crate::mark::{Mark, NUM_MARKS};
use crate::xarray::{BITS_PER_LAYER, SLOT_MASK, SLOT_SIZE};

/// The height of an `XNode` within an `XArray`.
///
/// In an `XArray`, the head has the highest height, while the `XNode`s that directly store items
/// are at the lowest height, with a height value of 1. Each level up from the bottom height
/// increases the height number by 1. The height of an `XArray` is the height of its head.
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
    /// Creates a `Height` directly from a height value.
    pub fn new(height: u8) -> Self {
        Self { height }
    }

    /// Creates a `Height` which has the mininal height value but allows the `index`-th item to be
    /// stored.
    pub fn from_index(index: u64) -> Self {
        let mut height = Height::new(1);
        while index > height.max_index() {
            *height += 1;
        }
        height
    }

    /// Goes up, which increases the height velue by one.
    pub fn go_root(&self) -> Self {
        Self::new(self.height + 1)
    }

    /// Goes down, which decreases the height value by one.
    pub fn go_leaf(&self) -> Self {
        Self::new(self.height - 1)
    }

    fn height_shift(&self) -> u8 {
        (self.height - 1) * BITS_PER_LAYER as u8
    }

    /// Calculates the corresponding offset for the target index at the current height.
    pub fn height_offset(&self, index: u64) -> u8 {
        ((index >> self.height_shift()) & SLOT_MASK as u64) as u8
    }

    /// Calculates the maximum index that can be represented in an `XArray` with the current
    /// height.
    pub fn max_index(&self) -> u64 {
        ((SLOT_SIZE as u64) << self.height_shift()) - 1
    }
}

/// The `XNode` is the intermediate node in the tree-like structure of the `XArray`.
///
/// It contains `SLOT_SIZE` number of `XEntry`s, meaning it can accommodate up to `SLOT_SIZE` child
/// nodes. The `height` and `offset_in_parent` attributes of an `XNode` are determined at
/// initialization and remain unchanged thereafter.
#[derive(Clone, Debug)]
pub(super) struct XNode<I>
where
    I: ItemEntry,
{
    /// The height of the subtree rooted at the current node. The height of a leaf node,
    /// which stores the user-given items, is 1.
    height: Height,
    /// This node is its parent's `offset_in_parent`-th child.
    ///
    /// This field will be zero if this node is the root, as the node will be the 0-th child of its
    /// parent once the height of `XArray` is increased.
    offset_in_parent: u8,
    /// The slots storing `XEntry`s, which point to user-given items for leaf nodes and other
    /// `XNode`s for interior nodes.
    slots: [XEntry<I>; SLOT_SIZE],
    /// The marks representing whether each slot is marked or not.
    ///
    /// Users can set mark or unset mark on user-given items, and a leaf node or an interior node
    /// is marked if and only if there is at least one marked item within the node.
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

    /// Get the slot offset at the current `XNode` for the target index `target_index`.
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

    /// Sets the slot at the given `offset` to the given `entry`.
    ///
    /// If `entry` represents an item, the old marks at the same offset will be cleared. Otherwise,
    /// if `entry` represents a node, the marks at the same offset will be updated according to
    /// whether the new node contains marked items.
    ///
    /// This method changes the mark _only_ on this `XNode'. It's the caller's responsibility to
    /// ensure that the marks on the ancestors of this `XNode' are up to date. See also
    /// [`XNode::update_mark`].
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

    /// Sets the input `mark` at the given `offset`.
    ///
    /// This method changes the mark _only_ on this `XNode'. It's the caller's responsibility to
    /// ensure that the marks on the ancestors of this `XNode' are up to date. See also
    /// [`XNode::update_mark`].
    pub fn set_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].set(offset);
    }

    /// Unsets the input `mark` at the given `offset`.
    ///
    /// This method changes the mark _only_ on this `XNode'. It's the caller's responsibility to
    /// ensure that the marks on the ancestors of this `XNode' are up to date. See also
    /// [`XNode::update_mark`].
    pub fn unset_mark(&mut self, offset: u8, mark: usize) {
        self.marks[mark].unset(offset);
    }

    pub fn clear_mark(&mut self, mark: usize) {
        self.marks[mark].clear();
    }

    /// Updates the mark at the given `offset` and returns `true` if the mark is changed.
    ///
    /// This method does nothing if the slot at the given `offset` does not represent a node. It
    /// assumes the marks of the child node are up to date, and ensures the mark at the given
    /// `offset` is also up to date.
    ///
    /// Whenever a mark at the leaf node changes, this method should be invoked from the leaf node
    /// up to the root node, until the mark does not change on some node or the root node has been
    /// reached.
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
