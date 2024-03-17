use core::marker::PhantomData;

use smallvec::SmallVec;

use crate::borrow::DormantMutRef;
use crate::entry::{ItemEntry, ItemRef, NodeMaybeMut, XEntry};
use crate::mark::XMark;
use crate::node::XNode;
use crate::xarray::{XArray, MAX_HEIGHT, SLOT_SIZE};

trait Operation {}

struct ReadOnly {}
struct ReadWrite {}

impl Operation for ReadOnly {}
impl Operation for ReadWrite {}

/// CursorState represents the current state of the cursor. Currently, there are two possible states:
/// 1. inactive: the state where the cursor is not positioned on any node.
/// 2. positioned on a node: this state includes information about the node the cursor is on,
/// as well as the offset of the entry that needs to be operated on within the slots of the current node.
enum CursorState<'a, I, O>
where
    I: ItemEntry,
    O: Operation,
{
    Inactive(PhantomData<O>),
    AtNode {
        node: &'a XNode<I>,
        operation_offset: u8,
    },
    AtNodeMut {
        node: &'a mut XNode<I>,
        operation_offset: u8,
    },
}

impl<'a, I: ItemEntry, O: Operation> Default for CursorState<'a, I, O> {
    fn default() -> Self {
        Self::Inactive(PhantomData)
    }
}

impl<'a, I: ItemEntry, O: Operation> CursorState<'a, I, O> {
    fn move_to(&mut self, node: &'a XNode<I>, index: u64) {
        let operation_offset = node.entry_offset(index);
        *self = Self::AtNode {
            node,
            operation_offset,
        };
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadWrite> {
    fn move_to_mut(&mut self, node: &'a mut XNode<I>, index: u64) {
        let operation_offset = node.entry_offset(index);
        *self = Self::AtNodeMut {
            node,
            operation_offset,
        };
    }

    fn move_to_maybe_mut(&mut self, node: NodeMaybeMut<'a, I>, index: u64) {
        match node {
            NodeMaybeMut::Shared(node) => self.move_to(node, index),
            NodeMaybeMut::Exclusive(node) => self.move_to_mut(node, index),
        }
    }
}

impl<'a, I: ItemEntry, O: Operation> CursorState<'a, I, O> {
    fn into_node(self) -> Option<(&'a XNode<I>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((node, operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((node, operation_offset)),
            Self::Inactive(..) => None,
        }
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadWrite> {
    fn into_node_mut(self) -> Option<(&'a mut XNode<I>, u8)> {
        match self {
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((node, operation_offset)),
            Self::Inactive(..) | Self::AtNode { .. } => None,
        }
    }

    fn into_node_maybe_mut(self) -> Option<(NodeMaybeMut<'a, I>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((NodeMaybeMut::Shared(node), operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((NodeMaybeMut::Exclusive(node), operation_offset)),
            Self::Inactive(..) => None,
        }
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadOnly> {
    fn as_node(&self) -> Option<(&'a XNode<I>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((*node, *operation_offset)),
            Self::Inactive(..) | Self::AtNodeMut { .. } => None,
        }
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadWrite> {
    fn as_node(&self) -> Option<(&XNode<I>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((*node, *operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((*node, *operation_offset)),
            Self::Inactive(..) => None,
        }
    }

    fn as_node_mut(&mut self) -> Option<(&mut XNode<I>, u8)> {
        match self {
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((*node, *operation_offset)),
            Self::Inactive(..) | Self::AtNode { .. } => None,
        }
    }
}

impl<'a, I: ItemEntry, O: Operation> CursorState<'a, I, O> {
    fn is_at_node(&self) -> bool {
        match self {
            Self::AtNode { .. } | Self::AtNodeMut { .. } => true,
            Self::Inactive(..) => false,
        }
    }

    fn is_leaf(&self) -> bool {
        match self {
            Self::AtNodeMut { node, .. } => node.is_leaf(),
            Self::AtNode { node, .. } => node.is_leaf(),
            Self::Inactive(..) => false,
        }
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadWrite> {
    fn is_at_node_mut(&self) -> bool {
        match self {
            Self::AtNodeMut { .. } => true,
            Self::Inactive(..) | Self::AtNode { .. } => false,
        }
    }
}

/// A `Cursor` can traverse in the `XArray` and have a target `XEntry` to operate,
/// which is stored in the `index` of `XArray`. `Cursor` can be only created by an `XArray`,
/// and will hold its immutable reference, and can only perform read-only operations
/// for the corresponding `XArray`.
///
/// When creating a cursor, it will immediately traverses to touch the target XEntry in the XArray.
/// If the cursor cannot reach to the node that can operate the target XEntry, its state will set to `Inactive`.
/// A Cursor can reset its target index. Once it do this, it will also immediately traverses to touch the target XEntry.
/// A Cursor can also perform `next()` to quickly operate the XEntry at the next index.
/// If the Cursor perform reset or next and then have a target index that is not able to touch,
/// the Cursor's state will also set to `Inactive`.
///
/// Hence, at any given moment when no operation is being performed, a cursor will be positioned on
/// the XNode and be ready to operate its target XEntry. If not, it means that the cursor is not able
/// to touch the target `XEntry`.
///
/// The cursor will also record all nodes passed from the head node to the target position in `passed_node`,
/// thereby assisting it in performing some operations that involve searching upwards.
///
/// Multiple Cursors are allowed to operate on a single XArray at the same time.
pub struct Cursor<'a, I, M>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    /// The `XArray` the cursor located in.
    xa: &'a XArray<I, M>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// Represents the current state of the cursor.
    state: CursorState<'a, I, ReadOnly>,
    /// Record add nodes passed from the head node to the target position.
    /// The index is the height of the recorded node.
    ancestors: SmallVec<[&'a XNode<I>; MAX_HEIGHT]>,
}

impl<'a, I: ItemEntry, M: Into<XMark>> Cursor<'a, I, M> {
    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub(super) fn new(xa: &'a XArray<I, M>, index: u64) -> Self {
        let mut cursor = Self {
            xa,
            index,
            state: CursorState::default(),
            ancestors: SmallVec::new(),
        };
        cursor.traverse_to_target();

        cursor
    }

    /// Reset the target index of the Cursor. Once set, it will immediately attempt to move the Cursor
    ///  to touch the target XEntry.
    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;

        self.traverse_to_target();
    }

    /// Return the target index of the cursor.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Move the target index of the cursor to index + 1.
    /// If the target index's corresponding XEntry is not within the current XNode, the cursor will
    /// move to touch the target XEntry. If the move fails, the cursor's state will be set to `Inactive`.
    pub fn next(&mut self) {
        self.index = self.index.checked_add(1).unwrap();

        if !self.state.is_at_node() {
            return;
        }

        let (mut current_node, mut operation_offset) =
            core::mem::take(&mut self.state).into_node().unwrap();

        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            let Some(parent_node) = self.ancestors.pop() else {
                self.reset();
                return;
            };

            operation_offset = current_node.offset_in_parent() + 1;
            current_node = parent_node;
        }

        self.state.move_to(current_node, self.index);
        self.continue_traverse_to_target();
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub fn load(&mut self) -> Option<ItemRef<'a, I>> {
        self.traverse_to_target();
        self.state
            .as_node()
            .and_then(|(node, off)| node.entry(off).as_item_ref())
    }

    /// Judge if the target item is marked with the input `mark`.
    /// If target item does not exist, the function will return `None`.
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let max_index = self.xa.max_index();
        if max_index < self.index || max_index == 0 {
            return;
        }

        let current_node = self.xa.head().as_node_ref().unwrap();
        self.state.move_to(current_node, self.index);
        self.continue_traverse_to_target();
    }

    fn continue_traverse_to_target(&mut self) {
        while !self.state.is_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node().unwrap();

            let operated_entry = current_node.entry(operation_offset);
            if !operated_entry.is_node() {
                self.reset();
                return;
            }

            self.ancestors.push(current_node);

            let new_node = operated_entry.as_node_ref().unwrap();
            self.state.move_to(new_node, self.index);
        }
    }

    /// Initialize the Cursor to its initial state.
    fn reset(&mut self) {
        self.state = CursorState::default();
        self.ancestors.clear();
    }
}

struct NodeMutRef<'a, I>
where
    I: ItemEntry,
{
    inner: DormantMutRef<'a, XNode<I>>,
}

impl<'a, I: ItemEntry> NodeMutRef<'a, I> {
    fn new(node: &'a mut XNode<I>, operation_offset: u8) -> (&'a mut XEntry<I>, NodeMutRef<'a, I>) {
        let (node, inner) = DormantMutRef::new(node);
        (node.entry_mut(operation_offset), NodeMutRef { inner })
    }

    unsafe fn awaken(self) -> &'a mut XNode<I> {
        self.inner.awaken()
    }

    unsafe fn awaken_modified(self, last_index: u64) -> (&'a mut XNode<I>, bool) {
        let node = unsafe { self.inner.awaken() };
        let changed = node.update_mark(node.height().height_offset(last_index));
        (node, changed)
    }
}

/// A `CursorMut` can traverse in the `XArray` and have a target `XEntry` to operate,
/// which is stored in the `index` of `XArray`. `CursorMut` can be only created by an `XArray`,
/// and will hold its mutable reference, and can perform read and write operations
/// for the corresponding `XArray`.
///
/// When creating a `CursorMut`, it will immediately traverses to touch the target XEntry in the XArray.
/// If the `CursorMut` cannot reach to the node that can operate the target XEntry,
/// its state will set to `Inactive`. A `CursorMut` can reset its target index.
/// Once it do this, it will also immediately traverses to touch the target XEntry.  
/// A `CursorMut` can also perform `next()` to quickly operate the XEntry at the next index.
/// If the `CursorMut` perform reset or next and then have a target index that is not able to touch,
/// the `CursorMut`'s state will also set to `Inactive`.  
///
/// When CursorMut performs `reset_to()` and `next()` methods and moves its index,
/// the CursorMut will no longer be exclusive.
///
/// Hence, at any given moment when no operation is being performed, a `CursorMut` will be
/// positioned on the XNode and be ready to operate its target XEntry. If not, it means that the `CursorMut`
/// is not able to touch the target `XEntry`. For this situation, the `CursorMut`
/// can invoke `store` method which will expand the XArray to guarantee to reach the target position.
///
/// The `CursorMut` will also record all nodes passed from the head node to the target position
/// in passed_node, thereby assisting it in performing some operations that involve searching upwards.
///
/// **Features for COW (Copy-On-Write).** The CursorMut guarantees that if it is exclusive,
/// all nodes it traverses during the process are exclusively owned by the current XArray.
/// If it finds that the node it is about to access is shared with another XArray due to a COW clone,
/// it will trigger a COW to copy and create an exclusive node for access. Additionally,
/// since it holds a mutable reference to the current XArray, it will not conflict with
/// any other cursors on the XArray. CursorMut is set to exclusive when a modification
/// is about to be performed
///
/// When a CursorMut doing write operation on XArray, it should not be affected by other CursorMuts
/// or affect other Cursors.
pub struct CursorMut<'a, I, M>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    /// The `XArray` the cursor located in.
    xa: DormantMutRef<'a, XArray<I, M>>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// Represents the current state of the cursor.
    state: CursorState<'a, I, ReadWrite>,
    /// Record add nodes passed from the head node to the target position.
    /// The index is the height of the recorded node.
    mut_ancestors: SmallVec<[NodeMutRef<'a, I>; MAX_HEIGHT]>,
    ancestors: SmallVec<[&'a XNode<I>; MAX_HEIGHT]>,
}

impl<'a, I: ItemEntry, M: Into<XMark>> CursorMut<'a, I, M> {
    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub(super) fn new(xa: &'a mut XArray<I, M>, index: u64) -> Self {
        let mut cursor = Self {
            xa: DormantMutRef::new(xa).1,
            index,
            state: CursorState::default(),
            mut_ancestors: SmallVec::new(),
            ancestors: SmallVec::new(),
        };
        cursor.traverse_to_target();

        cursor
    }

    /// Reset the target index of the Cursor. Once set, it will immediately attempt to move the
    /// Cursor to touch the target XEntry.
    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;

        self.traverse_to_target();
    }

    /// Return the target index of the cursor.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Move the target index of the cursor to index + 1.
    /// If the target index's corresponding XEntry is not within the current XNode, the cursor
    /// will move to touch the target XEntry. If the move fails, the cursor's state will be
    /// set to `Inactive`.
    pub fn next(&mut self) {
        self.index = self.index.checked_add(1).unwrap();

        if !self.state.is_at_node() {
            return;
        }

        let (mut current_node, mut operation_offset) = core::mem::take(&mut self.state)
            .into_node_maybe_mut()
            .unwrap();

        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            let offset_in_parent = current_node.offset_in_parent();
            drop(current_node);

            let parent_node = if let Some(node) = self.ancestors.pop() {
                NodeMaybeMut::Shared(node)
            } else if let Some(node) = self.mut_ancestors.pop() {
                NodeMaybeMut::Exclusive(unsafe { node.awaken_modified(self.index - 1).0 })
            } else {
                self.reset();
                return;
            };

            operation_offset = offset_in_parent + 1;
            current_node = parent_node;
        }

        self.state.move_to_maybe_mut(current_node, self.index);
        self.continue_traverse_to_target();
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub fn load(&mut self) -> Option<ItemRef<'_, I>> {
        self.traverse_to_target();
        self.state
            .as_node()
            .and_then(|(node, off)| node.entry(off).as_item_ref())
    }

    /// Judge if the target item is marked with the input `mark`.
    /// If target item does not exist, the function will return `None`.
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }

    /// Stores the provided `XEntry` in the `XArray` at the position indicated by the current cursor index.
    ///
    /// If the provided entry is the same as the current entry at the cursor position,
    /// the method returns the provided entry without making changes.
    /// Otherwise, it replaces the current entry with the provided one and returns the old entry.
    pub fn store(&mut self, item: I) -> Option<I> {
        self.expand_and_traverse_to_target();
        self.state
            .as_node_mut()
            .and_then(|(node, off)| node.set_entry(off, XEntry::from_item(item)).into_item())
    }

    /// Removes the `XEntry` at the target index of the 'CursorMut' within the `XArray`.
    ///
    /// This is achieved by storing an empty `XEntry` at the target index using the `store` method.
    /// The method returns the replaced `XEntry` that was previously stored at the target index.
    pub fn remove(&mut self) -> Option<I> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .and_then(|(node, off)| node.set_entry(off, XEntry::EMPTY).into_item())
    }

    /// Mark the item at the target index in the `XArray` with the input `mark`.
    /// If the item does not exist, return an Error.
    ///
    /// This operation will also mark all nodes along the path from the head node to the target node
    /// with the input `mark`, because a marked intermediate node should be equivalent to
    /// having a child node that is marked.
    pub fn set_mark(&mut self, mark: M) -> Result<(), ()> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .filter(|(node, off)| node.entry(*off).is_item())
            .map(|(node, off)| node.set_mark(off, mark.into().index()))
            .ok_or(())
    }

    /// Unset the input `mark` for the item at the target index in the `XArray`.
    /// If the item does not exist, return an Error.
    ///
    /// This operation will also unset the input `mark` for all nodes along the path from the head node
    /// to the target node if the input `mark` have not marked any of their children.
    pub fn unset_mark(&mut self, mark: M) -> Result<(), ()> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .filter(|(node, off)| node.entry(*off).is_item())
            .map(|(node, off)| node.unset_mark(off, mark.into().index()))
            .ok_or(())
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let xa = unsafe { self.xa.reborrow() };

        let max_index = xa.max_index();
        if max_index < self.index || max_index == 0 {
            return;
        }

        let current_node = xa.head_mut().as_node_maybe_mut().unwrap();
        self.state.move_to_maybe_mut(current_node, self.index);
        self.continue_traverse_to_target();
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// During the traverse, the cursor may modify the XArray to let itself be able to reach the target node.
    ///
    /// Before traverse, the cursor will first expand the height of `XArray` to make sure it have enough capacity.
    /// During the traverse, the cursor will allocate new `XNode` and put it in the appropriate slot if needed.
    ///
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    fn expand_and_traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            self.ensure_exclusive_before_modification();
            return;
        }

        let head = {
            let xa = unsafe { self.xa.reborrow() };
            xa.reserve(self.index);
            xa.head_mut().as_node_mut_or_cow().unwrap()
        };

        self.state.move_to_mut(head, self.index);
        self.exclusively_traverse_to_target();
    }

    fn ensure_exclusive_before_modification(&mut self) {
        if self.state.is_at_node_mut() {
            return;
        }

        self.state = CursorState::default();
        self.ancestors.clear();

        let node = match self.mut_ancestors.pop() {
            Some(node) => unsafe { node.awaken() },
            None => {
                let xa = unsafe { self.xa.reborrow() };

                let head = xa.head_mut();
                if !head.is_node() {
                    return;
                }

                head.as_node_mut_or_cow().unwrap()
            }
        };

        self.state.move_to_mut(node, self.index);
        self.exclusively_traverse_to_target();
    }

    fn continue_traverse_to_target(&mut self) {
        while !self.state.is_leaf() {
            let (current_node, operation_offset) = core::mem::take(&mut self.state)
                .into_node_maybe_mut()
                .unwrap();

            let next_node = match current_node {
                NodeMaybeMut::Shared(node) => {
                    let operated_entry = node.entry(operation_offset);
                    if !operated_entry.is_node() {
                        self.reset();
                        return;
                    }

                    self.ancestors.push(node);

                    NodeMaybeMut::Shared(operated_entry.as_node_ref().unwrap())
                }
                NodeMaybeMut::Exclusive(node) => {
                    let (operated_entry, dormant_node) = NodeMutRef::new(node, operation_offset);
                    if !operated_entry.is_node() {
                        self.reset();
                        return;
                    }

                    self.mut_ancestors.push(dormant_node);

                    operated_entry.as_node_maybe_mut().unwrap()
                }
            };

            self.state.move_to_maybe_mut(next_node, self.index);
        }
    }

    fn exclusively_traverse_to_target(&mut self) {
        while !self.state.is_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node_mut().unwrap();

            if current_node.entry(operation_offset).is_null() {
                let new_node = XNode::new(current_node.height().go_leaf(), operation_offset);
                let new_entry = XEntry::from_node(new_node);
                current_node.set_entry(operation_offset, new_entry);
            }

            let (operated_entry, dormant_node) = NodeMutRef::new(current_node, operation_offset);
            self.mut_ancestors.push(dormant_node);

            let next_node = operated_entry.as_node_mut_or_cow().unwrap();
            self.state.move_to_mut(next_node, self.index)
        }
    }

    /// Initialize the Cursor to its initial state.
    fn reset(&mut self) {
        self.state = CursorState::default();
        self.ancestors.clear();

        while let Some(node) = self.mut_ancestors.pop() {
            let (_, changed) = unsafe { node.awaken_modified(self.index) };
            if !changed {
                self.mut_ancestors.clear();
                break;
            }
        }
    }
}

impl<'a, I: ItemEntry, M: Into<XMark>> Drop for CursorMut<'a, I, M> {
    fn drop(&mut self) {
        self.reset();
    }
}
