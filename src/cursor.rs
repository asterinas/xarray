use crate::*;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// CursorState represents the current state of the cursor. Currently, there are two possible states:
/// 1. inactive: the initial state where the cursor is not positioned on any node.
/// 2. positioned on a node: this state includes information about the node the cursor is on,
/// as well as the offset of the entry that needs to be operated on within the slots of the current node.
enum CursorState<'a, I, Operation>
where
    I: ItemEntry,
{
    Inactive,
    AtNode {
        node: &'a XNode<I, Operation>,
        operation_offset: u8,
    },
}

impl<'a, I: ItemEntry, Operation> CursorState<'a, I, Operation> {
    fn default() -> Self {
        Self::Inactive
    }

    fn arrive_node(&mut self, node: &'a XNode<I, Operation>, operation_offset: u8) {
        *self = Self::AtNode {
            node,
            operation_offset,
        };
    }

    fn is_inactive(&self) -> bool {
        matches!(self, Self::Inactive)
    }

    fn is_at_node(&self) -> bool {
        matches!(
            self,
            Self::AtNode {
                node: _,
                operation_offset: _
            }
        )
    }

    fn node_info(&self) -> Option<(&'a XNode<I, Operation>, u8)> {
        if let Self::AtNode {
            node,
            operation_offset,
        } = self
        {
            Some((node, *operation_offset))
        } else {
            None
        }
    }
}

/// A `Cursor` can traverse in the `XArray` and have a target operated `XEntry`, which is stored in the `index` of `XArray`.
/// `Cursor` can be only created by an `XArray`, and will hold its immutable reference, and can only perform read-only operations
/// for the corresponding `XArray`.
/// When a cursor traverses an XArray, at any given moment, it is positioned on an XNode. If not, it means that
/// the cursor has not begin to traverse.
///
/// After traversing, the `Cursor` will arrive at a node where the target entry can be operated on. If the arrival fails,
/// it will be reset to its initial state, meaning it is not positioned at any node. Therefore, if the `Cursor` is not
/// in the midst of a traversal operation, it is either not yet started or it has already reached a node where the
/// target entry can be acted upon.
///
/// Multiple Cursors are allowed to operate on a single XArray at the same time.
///
/// TODO: Implement `next()` to allow to change the target index in cursors.
pub(crate) struct Cursor<'a, I, M>
where
    I: ItemEntry,
    M: ValidMark,
{
    /// The `XArray` the cursor located in.
    xa: &'a XArray<I, M>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// Represents the current state of the cursor.
    state: CursorState<'a, I, ReadOnly>,

    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry, M: ValidMark> Cursor<'a, I, M> {
    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub(crate) fn new(xa: &'a XArray<I, M>, index: u64) -> Self {
        Self {
            xa,
            index,
            state: CursorState::default(),
            _marker: PhantomData,
        }
    }

    /// Obtain a reference to the XEntry from a pointer pointing to it.
    ///
    /// # Safety
    /// The user must ensure that the pointer remains valid for the duration of use of the target XEntry reference.
    unsafe fn ref_entry(&self, entry_ptr: *const XEntry<I>) -> &'a XEntry<I> {
        self.xa.ref_entry(entry_ptr)
    }

    /// Obtain a reference to the XEntry in the slots of target node. The input `offset` indicate
    /// the offset of the XEntry in the slots.
    fn ref_node_entry(&self, node: &'a XNode<I, ReadOnly>, offset: u8) -> &'a XEntry<I> {
        let target_entry_ptr = node.entry(offset);
        // Safety: The returned entry has the same lifetime with the XNode that owns it.
        // Hence the position that `target_entry_ptr` points to will be valid during the usage of returned reference.
        unsafe { self.ref_entry(target_entry_ptr) }
    }

    /// Move the `Cursor` to the `XNode` that `node_entry` points to, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on.
    fn move_to(&mut self, node: &'a XNode<I, ReadOnly>) -> Option<&'a XEntry<I>> {
        let (current_entry, offset) = {
            let offset = node.entry_offset(self.index);
            let current_entry = self.ref_node_entry(node, offset);
            (current_entry, offset)
        };
        self.state.arrive_node(node, offset);
        Some(current_entry)
    }

    /// Judge if the target item is marked with the input `mark`.
    /// If target item does not exist, the function will return `None`.
    pub(crate) fn is_marked(&mut self, mark: M) -> Option<bool> {
        self.traverse_to_target();
        if let CursorState::AtNode {
            operation_offset,
            node,
        } = self.state
        {
            Some(node.is_marked(operation_offset, mark.index()))
        } else {
            None
        }
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub(crate) fn load(&mut self) -> Option<&'a XEntry<I>> {
        self.traverse_to_target()
    }

    /// Traverse the subtree and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) -> Option<&'a XEntry<I>> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return Some(self.ref_node_entry(current_node, operation_offset));
        }

        let max_index = self.xa.max_index();
        if max_index < self.index || max_index == 0 {
            return None;
        }
        self.move_to(self.xa.head().as_node().unwrap());

        let (current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = self.ref_node_entry(current_node, operation_offset);
        while current_layer > 0 {
            if let None = operated_entry.as_node() {
                self.init();
                return None;
            }

            *current_layer -= 1;
            operated_entry = self.move_to(operated_entry.as_node().unwrap()).unwrap();
        }
        Some(operated_entry)
    }

    /// Initialize the Cursor to its initial state.
    fn init(&mut self) {
        self.state = CursorState::default();
    }

    /// Return the target index of the cursor.
    fn index(&mut self) -> u64 {
        self.index
    }

    /// Determine whether the cursor arrive at the node that can operate target entry.
    /// It can only be used before or after traversing. Since the cursor will only either not yet started or has already reached the target node
    /// when not in a traversal, it is reasonable to determine whether the cursor has reached its destination node by checking if the cursor is positioned on a node.
    fn is_arrived(&mut self) -> bool {
        self.state.is_at_node()
    }
}

/// A `CursorMut` can traverse in the `XArray` and have a target operated `XEntry`, which is stored in the `index` of `XArray`.
/// `Cursor` can be only created by an `XArray`, and will hold its mutable reference, and can perform read and write operations
/// for the corresponding `XArray`.
/// When a cursor traverses an XArray, at any given moment, it is positioned on an XNode. If not, it means that
/// the cursor has not begin to traverse.
///
/// After traversing, the `CursorMut` will arrive at a node where the target entry can be operated on. If the arrival fails,
/// it will be reset to its initial state, meaning it is not positioned at any node. Therefore, if the `CursorMut` is not
/// in the midst of a traversal operation, it is either not yet started or it has already reached a node where the
/// target entry can be acted upon.
///
/// When a CursorMut doing operation on XArray, it should not be affected by other CursorMuts or affect other Cursors.
///
/// TODO: Implement `next()` to allow to change the target index in cursors.
pub(crate) struct CursorMut<'a, I, M>
where
    I: ItemEntry,
    M: ValidMark,
{
    /// The `XArray` the cursor located in.
    xa: &'a mut XArray<I, M>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// Represents the current state of the cursor.
    state: CursorState<'a, I, ReadWrite>,

    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry, M: ValidMark> CursorMut<'a, I, M> {
    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub(crate) fn new(xa: &'a mut XArray<I, M>, index: u64) -> Self {
        Self {
            xa,
            index,
            state: CursorState::default(),
            _marker: PhantomData,
        }
    }

    /// Obtain a reference to the XEntry in the slots of target node. The input `offset` indicate
    /// the offset of the XEntry in the slots.
    fn ref_node_entry(&self, node: &'a XNode<I, ReadWrite>, offset: u8) -> &'a XEntry<I> {
        let target_entry_ptr = node.entry(offset);
        // Safety: The returned entry has the same lifetime with the XNode that owns it.
        // Hence the position that `target_entry_ptr` points to will be valid during the usage of returned reference.
        unsafe { self.ref_entry(target_entry_ptr) }
    }

    /// Move the `CursorMut` to the `XNode` that `node_entry` points to, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on next.
    fn move_to(&mut self, node: &'a XNode<I, ReadWrite>) -> Option<&'a XEntry<I>> {
        let (current_entry, offset) = {
            let offset = node.entry_offset(self.index);
            let current_entry = self.ref_node_entry(node, offset);
            (current_entry, offset)
        };
        self.state.arrive_node(node, offset);
        Some(current_entry)
    }

    /// Stores the provided `XEntry` in the `XArray` at the position indicated by the current cursor index.
    ///
    /// If the provided entry is the same as the current entry at the cursor position,
    /// the method returns the provided entry without making changes.
    /// Otherwise, it replaces the current entry with the provided one and returns the old entry.
    pub(crate) fn store(&mut self, entry: XEntry<I>) -> XEntry<I> {
        let target_entry = self.traverse_to_target_mut();
        if entry.raw() == target_entry.raw() {
            return entry;
        }
        let (current_node, operation_offset) = self.state.node_info().unwrap();
        let old_entry = current_node.set_entry(operation_offset, entry);
        return old_entry;
    }

    /// Mark the item at the target index in the `XArray` with the input `mark`.
    /// If the item does not exist, return an Error.
    ///
    /// This operation will also mark all nodes along the path from the head node to the target node with the input `mark`,
    /// because a marked intermediate node should be equivalent to having a child node that is marked.
    pub(crate) fn set_mark(&mut self, mark: M) -> Result<(), ()> {
        self.traverse_to_target();
        if let Some((current_node, operation_offset)) = self.state.node_info() {
            current_node.set_mark(operation_offset, mark.index());
            let mut offset_in_parent = current_node.offset_in_parent();
            let mut parent = current_node.parent();
            while let Some(parent_node) = parent {
                if parent_node.is_marked(offset_in_parent, mark.index()) {
                    break;
                }
                parent_node.set_mark(offset_in_parent, mark.index());
                offset_in_parent = parent_node.offset_in_parent();
                parent = parent_node.parent();
            }
            Ok(())
        } else {
            Err(())
        }
    }

    /// Unset the input `mark` for the item at the target index in the `XArray`.
    /// If the item does not exist, return an Error.
    ///
    /// This operation will also unset the input `mark` for all nodes along the path from the head node to the target node
    /// if the input `mark` have not marked any of their children.
    pub(crate) fn unset_mark(&mut self, mark: M) -> Result<(), ()> {
        self.traverse_to_target();
        if let Some((mut current_node, operation_offset)) = self.state.node_info() {
            current_node.unset_mark(operation_offset, mark.index());
            while current_node.is_mark_clear(mark.index()) {
                let offset_in_parent = current_node.offset_in_parent();
                let parent = current_node.parent();
                if let Some(parent_node) = parent {
                    parent_node.unset_mark(offset_in_parent, mark.index());
                    current_node = parent_node;
                } else {
                    break;
                }
            }
            Ok(())
        } else {
            Err(())
        }
    }

    /// Removes the `XEntry` at the target index of the 'CursorMut' within the `XArray`.
    ///
    /// This is achieved by storing an empty `XEntry` at the target index using the `store` method.
    /// The method returns the replaced `XEntry` that was previously stored at the target index.
    pub(crate) fn remove(&mut self) -> XEntry<I> {
        self.store(XEntry::EMPTY)
    }

    /// Traverse the subtree and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) -> Option<&'a XEntry<I>> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return Some(self.ref_node_entry(current_node, operation_offset));
        }

        let max_index = self.xa.max_index();
        if max_index < self.index || max_index == 0 {
            return None;
        }
        let head = self.xa.head_mut().as_node_mut().unwrap();
        self.move_to(head);

        let (current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = self.ref_node_entry(current_node, operation_offset);
        while current_layer > 0 {
            if let None = operated_entry.as_node() {
                self.init();
                return None;
            }

            *current_layer -= 1;
            operated_entry = self.move_to(operated_entry.as_node_mut().unwrap()).unwrap();
        }
        Some(operated_entry)
    }

    /// Traverse the subtree and move to the node that can operate the target entry.
    /// During the traverse, the cursor may modify the XArray to let itself be able to reach the target node.
    ///
    /// Before traverse, the cursor will first expand the layer of `XArray` to make sure it have enough capacity.
    /// During the traverse, the cursor will allocate new `XNode` and put it in the appropriate slot if needed.
    ///
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    fn traverse_to_target_mut(&mut self) -> &'a XEntry<I> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return self.ref_node_entry(current_node, operation_offset);
        }

        self.expand_layer();
        let head_ref = self.xa.head_mut().as_node_mut().unwrap();
        self.move_to(head_ref);

        let (current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = self.ref_node_entry(current_node, operation_offset);
        while current_layer > 0 {
            if let None = operated_entry.as_node() {
                let new_entry = {
                    let (current_node, operation_offset) = self.state.node_info().unwrap();
                    let new_owned_entry = self.alloc_node(
                        Layer::new(*current_layer - 1),
                        operation_offset,
                        Some(current_node),
                    );
                    let _ = current_node.set_entry(operation_offset, new_owned_entry);
                    self.ref_node_entry(current_node, operation_offset)
                };
                operated_entry = new_entry;
            }
            *current_layer -= 1;
            operated_entry = self.move_to(operated_entry.as_node_mut().unwrap()).unwrap();
        }
        operated_entry
    }

    /// Increase the number of layers for XArray to expand its capacity, allowing it to accommodate the target index,
    /// and returns the layer of the final head node.
    ///
    /// If the head node of the XArray does not exist, allocate a new head node of appropriate layer directly.
    /// Otherwise, if needed, repeatedly insert new nodes on top of the current head node to serve as the new head.
    fn expand_layer(&mut self) -> Layer {
        if self.xa.head().is_null() {
            let mut head_layer = Layer::new(0);
            while self.index > head_layer.max_index() {
                *head_layer += 1;
            }
            let head = self.alloc_node(head_layer, 0, None);
            self.xa.set_head(head);
            return head_layer;
        } else {
            loop {
                let head_layer = {
                    let head = self.xa.head().as_node().unwrap();
                    head.layer()
                };

                if head_layer.max_index() > self.index {
                    return head_layer;
                }

                let new_node_entry = self.alloc_node(Layer::new(*head_layer + 1), 0, None);
                let old_head_entry = self.xa.set_head(new_node_entry);
                let old_head = old_head_entry.as_node_mut().unwrap();
                let new_head = self.xa.head_mut().as_node_mut().unwrap();
                old_head.set_parent(new_head);
                let _empty = new_head.set_entry(0, old_head_entry);
            }
        }
    }

    /// Allocate a new XNode with the specified layer and offset,
    /// then generate a node entry from it and return it to the caller.
    fn alloc_node(
        &mut self,
        layer: Layer,
        offset: u8,
        parent: Option<&XNode<I, ReadWrite>>,
    ) -> XEntry<I> {
        let parent = parent.map(|p| {
            let arc = unsafe { Arc::from_raw(p as *const XNode<I, ReadWrite>) };
            let weak = Arc::downgrade(&arc);
            core::mem::forget(arc);
            weak
        });
        XEntry::from_node(XNode::<I, ReadWrite>::new(layer, offset, parent))
    }
}

impl<'a, I: ItemEntry, M: ValidMark> Deref for CursorMut<'a, I, M> {
    type Target = Cursor<'a, I, M>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self as *const CursorMut<'a, I, M> as *const Cursor<'a, I, M>) }
    }
}

impl<'a, I: ItemEntry, M: ValidMark> DerefMut for CursorMut<'a, I, M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self as *const CursorMut<'a, I, M> as *mut Cursor<'a, I, M>) }
    }
}
