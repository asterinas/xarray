use core::marker::PhantomData;

use smallvec::SmallVec;

use crate::borrow::{DestroyableMutRef, DestroyableRef, DormantMutRef};
use crate::entry::{ItemEntry, NodeMaybeMut, XEntry};
use crate::mark::XMark;
use crate::node::XNode;
use crate::xarray::{XArray, MAX_HEIGHT, SLOT_SIZE};

trait Operation {}

struct ReadOnly {}
struct ReadWrite {}

impl Operation for ReadOnly {}
impl Operation for ReadWrite {}

/// A type representing the state of a [`Cursor`] or a [`CursorMut`]. Currently, there are three
/// possible states:
///  - `Inactive`: The cursor is not positioned on any node.
///  - `AtNode`: The cursor is positioned on some node and holds a shared reference to it.
///  - `AtNodeMut`: The cursor is positioned on some node and holds an exclusive reference to it.
///
/// Currently, a cursor never ends up on an interior node. In other words, when methods of `Cursor`
/// or `CursorMut` finish, the cursor will either not positioned on any node or positioned on some
/// leaf node.
///
/// A `Cursor` manages its state with `CursorState<'a, I, ReadOnly>`, which will never be in the
/// `AtNodeMut` state. A `Cursor` never attempts to perform modification, so it never holds an
/// exclusive reference.
///
/// On contrast, a `CursorMut` uses `CursorState<'a, I, ReadWrite>` to manage its state, where all
/// the three states are useful. Due to the COW mechansim, a node can be shared in multiple
/// `XArray`s. In that case, the `CursorMut` will first enter the `AtNode` state as it cannot hold
/// an exclusive reference to shared data. Just before performing the modification, it copies the
/// shared data and creates the exclusive reference, which makes the cursor enter `AtNodeMut`
/// state.
enum CursorState<'a, I, O>
where
    I: ItemEntry,
    O: Operation,
{
    Inactive(PhantomData<O>),
    AtNode {
        node: DestroyableRef<'a, XNode<I>>,
        operation_offset: u8,
    },
    AtNodeMut {
        node: DestroyableMutRef<'a, XNode<I>>,
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
            node: DestroyableRef::new(node),
            operation_offset,
        };
    }
}

impl<'a, I: ItemEntry> CursorState<'a, I, ReadWrite> {
    fn move_to_mut(&mut self, node: &'a mut XNode<I>, index: u64) {
        let operation_offset = node.entry_offset(index);
        *self = Self::AtNodeMut {
            node: DestroyableMutRef::new(node),
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
            } => Some((node.borrow(), operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((node.into(), operation_offset)),
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
            } => Some((node.into(), operation_offset)),
            Self::Inactive(..) | Self::AtNode { .. } => None,
        }
    }

    fn into_node_maybe_mut(self) -> Option<(NodeMaybeMut<'a, I>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((NodeMaybeMut::Shared(node.borrow()), operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((NodeMaybeMut::Exclusive(node.into()), operation_offset)),
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
            } => Some((node.borrow(), *operation_offset)),
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
            } => Some((node.borrow(), *operation_offset)),
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((node.borrow(), *operation_offset)),
            Self::Inactive(..) => None,
        }
    }

    fn as_node_mut(&mut self) -> Option<(&mut XNode<I>, u8)> {
        match self {
            Self::AtNodeMut {
                node,
                operation_offset,
            } => Some((node.borrow_mut(), *operation_offset)),
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
            Self::AtNodeMut { node, .. } => node.borrow().is_leaf(),
            Self::AtNode { node, .. } => node.borrow().is_leaf(),
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

/// A `Cursor` can traverse in the [`XArray`] by setting or increasing the target index and can
/// perform read-only operations to the target item represented by the target index.
///
/// `Cursor`s act like shared references, so multiple cursors are allowed to operate on a single
/// `XArray` at the same time.
///
/// The typical way to obtain a `Cursor` instance is to call [`XArray::cursor`].
pub struct Cursor<'a, I, M>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    /// The `XArray` where the cursor locates.
    xa: &'a XArray<I, M>,
    /// The target index of the cursor.
    index: u64,
    /// The state of the cursor.
    state: CursorState<'a, I, ReadOnly>,
    /// Ancestors of the leaf node (exclusive), starting from the root node and going down.
    ancestors: SmallVec<[&'a XNode<I>; MAX_HEIGHT]>,
}

impl<'a, I: ItemEntry, M: Into<XMark>> Cursor<'a, I, M> {
    /// Creates a `Cursor` to perform read-related operations in the `XArray`.
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

    /// Resets the target index to `index`.
    ///
    /// Once set, the cursor will be positioned on the corresponding leaf node, if the leaf node
    /// exists.
    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;

        self.traverse_to_target();
    }

    /// Returns the target index of the cursor.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Increases the target index of the cursor by one.
    ///
    /// Once increased, the cursor will be positioned on the corresponding leaf node, if the leaf
    /// node exists.
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

    /// Loads the item at the target index.
    ///
    /// If the target item exists, this method will return a [`ItemEntry::Ref`] that acts exactly
    /// like a `&'a I` wrapped in `Some(_)`. Otherwises, it will return `None`.
    pub fn load(&mut self) -> Option<I::Ref<'a>> {
        self.traverse_to_target();
        self.state
            .as_node()
            .and_then(|(node, off)| node.entry(off).as_item_ref())
    }

    /// Checks whether the target item is marked with the input `mark`.
    ///
    /// If the target item does not exist, this method will also return false.
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }

    /// Traverses from the root node to the leaf node according to the target index, if necessary
    /// and possible.
    ///
    /// This methold should be called before any read-only operations.
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

    /// Traverses from an interior node to the leaf node according to the target index, if
    /// possible.
    ///
    /// This is a helper function for internal use. Users should call
    /// [`Cursor::traverse_to_target`] instead.
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

    /// Resets the cursor to the inactive state.
    fn reset(&mut self) {
        self.state = CursorState::default();
        self.ancestors.clear();
    }
}

/// A dormant mutable reference to a value of `XNode<I>`.
///
/// While the mutable reference is dormant, a subtree of the node can continue to be operated. When
/// the operation is finished (i.e., all references to the subtree are dead), `awaken` or
/// `awaken_modified` (depending on whether the marks on the subtree have been updated) can be
/// called to restore the original reference to the node.
struct NodeMutRef<'a, I>
where
    I: ItemEntry,
{
    inner: DormantMutRef<'a, XNode<I>>,
}

impl<'a, I: ItemEntry> NodeMutRef<'a, I> {
    /// Creates a dormant reference for the given `node` and gets a mutable reference for the
    /// operation on a subtree of the node (specified in `operation_offset`).
    fn new(node: &'a mut XNode<I>, operation_offset: u8) -> (&'a mut XEntry<I>, NodeMutRef<'a, I>) {
        let (node, inner) = DormantMutRef::new(node);
        (node.entry_mut(operation_offset), NodeMutRef { inner })
    }

    /// Restores the original node reference after the operation on the subtree is finished.
    ///
    /// This method does not update the mark corresponding to the subtree, so it should only be
    /// used when the marks on the subtree are not changed.
    ///
    /// # Safety
    ///
    /// Users must ensure all references to the subtree are now dead.
    unsafe fn awaken(self) -> &'a mut XNode<I> {
        // SAFETY: The safety requirements of the method ensure that the original reference and all
        // its derived references are dead.
        unsafe { self.inner.awaken() }
    }

    /// Restores the original node reference after the operation on the subtree is finished and
    /// updates the mark corresponding to the subtree.
    ///
    /// The `operation_offset` in [`NodeMutRef::new`] is not stored, so users must call this method
    /// with the `last_index` to identify the subtree on which the marks are changed.
    ///
    /// # Safety
    ///
    /// Users must ensure all references to the subtree are now dead.
    unsafe fn awaken_modified(self, last_index: u64) -> (&'a mut XNode<I>, bool) {
        // SAFETY: The safety requirements of the method ensure that the original reference and all
        // its derived references are dead.
        let node = unsafe { self.inner.awaken() };
        let changed = node.update_mark(node.height().height_offset(last_index));
        (node, changed)
    }
}

/// A `CursorMut` can traverse in the [`XArray`] by setting or increasing the target index and can
/// perform read-write operations to the target item represented by the target index.
///
/// `CursorMut`s act like exclusive references, so multiple cursors are not allowed to operate on a
/// single `XArray` at the same time.
///
/// The typical way to obtain a `CursorMut` instance is to call [`XArray::cursor_mut`].
///
/// **Features for COW (Copy-On-Write).** Due to COW, multiple `XArray`s can share the same piece
/// of data. As a result, `CursorMut` does not always have exclusive access to the items stored in
/// the `XArray`. However, just before performing the modification, `CursorMut` will create
/// exclusive copies by cloning shared items, which guarantees the isolation of data stored in
/// different `XArray`s.
pub struct CursorMut<'a, I, M>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    /// The `XArray` where the cursor locates.
    xa: DormantMutRef<'a, XArray<I, M>>,
    /// The target index of the cursor.
    index: u64,
    /// The state of the cursor.
    state: CursorState<'a, I, ReadWrite>,
    /// Ancestors of the leaf node (exclusive), starting from the root node and going down, until
    /// the first node which is shared in multiple `XArray`s.
    mut_ancestors: SmallVec<[NodeMutRef<'a, I>; MAX_HEIGHT]>,
    /// Ancestors of the leaf node (exclusive), but only containing the nodes which are shared in
    /// multiple `XArray`s, from the first one and going down.
    ancestors: SmallVec<[&'a XNode<I>; MAX_HEIGHT]>,
}

impl<'a, I: ItemEntry, M: Into<XMark>> CursorMut<'a, I, M> {
    /// Create a `CursorMut` to perform read- and write-related operations in the `XArray`.
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

    /// Resets the target index to `index`.
    ///
    /// Once set, the cursor will be positioned on the corresponding leaf node, if the leaf node
    /// exists.
    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;

        self.traverse_to_target();
    }

    /// Returns the target index of the cursor.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Increases the target index of the cursor by one.
    ///
    /// Once increased, the cursor will be positioned on the corresponding leaf node, if the leaf
    /// node exists.
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
                // SAFETY: All references derived from the tail node in `self.mut_ancestors` live
                // in `self.ancestors` and `self.state`, which has already been reset.
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

    /// Loads the item at the target index.
    ///
    /// If the target item exists, this method will return a [`ItemEntry::Ref`] that acts exactly
    /// like a `&'_ I` wrapped in `Some(_)`. Otherwises, it will return `None`.
    pub fn load(&mut self) -> Option<I::Ref<'_>> {
        self.traverse_to_target();
        self.state
            .as_node()
            .and_then(|(node, off)| node.entry(off).as_item_ref())
    }

    /// Checks whether the target item is marked with the input `mark`.
    ///
    /// If the target item does not exist, this method will also return false.
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }

    /// Stores a new `item` at the target index, and returns the old item if it previously exists.
    pub fn store(&mut self, item: I) -> Option<I> {
        self.expand_and_traverse_to_target();
        self.state
            .as_node_mut()
            .and_then(|(node, off)| node.set_entry(off, XEntry::from_item(item)).into_item())
    }

    /// Removes the item at the target index, and returns the removed item if it previously exists.
    pub fn remove(&mut self) -> Option<I> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .and_then(|(node, off)| node.set_entry(off, XEntry::EMPTY).into_item())
    }

    /// Sets the input `mark` for the item at the target index if the target item exists, otherwise
    /// returns an error.
    //
    // The marks on the ancestors of the leaf node also need to be updated, which will be done
    // later in `NodeMutRef::awaken_modified`.
    pub fn set_mark(&mut self, mark: M) -> Result<(), ()> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .filter(|(node, off)| node.entry(*off).is_item())
            .map(|(node, off)| node.set_mark(off, mark.into().index()))
            .ok_or(())
    }

    /// Unsets the input `mark` for the item at the target index if the target item exists,
    /// otherwise returns an error.
    //
    // The marks on the ancestors of the leaf node also need to be updated, which will be done
    // later in `NodeMutRef::awaken_modified`.
    pub fn unset_mark(&mut self, mark: M) -> Result<(), ()> {
        self.ensure_exclusive_before_modification();
        self.state
            .as_node_mut()
            .filter(|(node, off)| node.entry(*off).is_item())
            .map(|(node, off)| node.unset_mark(off, mark.into().index()))
            .ok_or(())
    }

    /// Traverses from the root node to the leaf node according to the target index, without
    /// creating new nodes, if necessary and possible.
    ///
    /// This method should be called before any read-only operations.
    fn traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        // SAFETY: The cursor is inactive. There are no alive references derived from the value of
        // `&mut XArray<I, M>`.
        let xa = unsafe { self.xa.reborrow() };

        let max_index = xa.max_index();
        if max_index < self.index || max_index == 0 {
            return;
        }

        let current_node = xa.head_mut().as_node_maybe_mut().unwrap();
        self.state.move_to_maybe_mut(current_node, self.index);
        self.continue_traverse_to_target();
    }

    /// Traverses from the root node to the leaf node according to the target index, potentially
    /// with creating new nodes, if necessary.
    ///
    /// This method should be called before any create-if-not-exist operations.
    fn expand_and_traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            self.ensure_exclusive_before_modification();
            return;
        }

        let head = {
            // SAFETY: The cursor is inactive. There are no alive references derived from the value
            // of `&mut XArray<I, M>`.
            let xa = unsafe { self.xa.reborrow() };
            xa.reserve(self.index);
            xa.head_mut().as_node_mut_or_cow().unwrap()
        };

        self.state.move_to_mut(head, self.index);
        self.exclusively_traverse_to_target();
    }

    /// Ensures the exclusive access to the leaf node by copying data when necessary.
    ///
    /// This method should be called before any modify-if-exist operations.
    fn ensure_exclusive_before_modification(&mut self) {
        if self.state.is_at_node_mut() {
            return;
        }

        self.state = CursorState::default();
        self.ancestors.clear();

        let node = match self.mut_ancestors.pop() {
            // SAFETY: All references derived from the tail node in `self.mut_ancestors` live in
            // `self.ancestors` and `self.state`, which has already been reset.
            Some(node) => unsafe { node.awaken() },
            None => {
                // SAFETY: All references derived from `self.xa` live in `self.mut_ancestors`,
                // `self.ancestors`, and `self.state`. All of them have already been cleared.
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

    /// Traverses from an interior node to the leaf node according to the target index, without
    /// creating new nodes, if possible.
    ///
    /// This is a helper function for internal use. Users should call
    /// [`CursorMut::traverse_to_target`] instead.
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

    /// Traverses from an interior node to the leaf node according to the target index, potentially
    /// with creating new nodes.
    ///
    /// This is a helper function for internal use. Users should call
    /// [`CursorMut::expand_and_traverse_to_target`] or
    /// [`CursorMut::ensure_exclusive_before_modification`] instead.
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

    /// Updates marks on ancestors if necessary and resets the cursor to the inactive state.
    fn reset(&mut self) {
        self.state = CursorState::default();
        self.ancestors.clear();

        while let Some(node) = self.mut_ancestors.pop() {
            // SAFETY: All references derived from the node in `self.mut_ancestors` live in the
            // following part of `self.mut_ancestors`, `self.ancestors`, and `self.state`, which
            // has already been cleared.
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
        // This updates marks on ancestors if necessary.
        self.reset();
    }
}
