use super::*;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// CursorState represents the current state of the cursor. Currently, there are two possible states:
/// 1. inactive: the state where the cursor is not positioned on any node.
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

/// A `Cursor` can traverse in the `XArray` and have a target `XEntry` to operate, which is stored in the `index` of `XArray`.
/// `Cursor` can be only created by an `XArray`, and will hold its immutable reference, and can only perform read-only operations
/// for the corresponding `XArray`.
///
/// When creating a cursor, it will immediately traverses to touch the target XEntry in the XArray. If the cursor cannot reach to the node
/// that can operate the target XEntry, its state will set to `Inactive`. A Cursor can reset its target index.
/// Once it do this, it will also immediately traverses to touch the target XEntry.  A Cursor can also perform `next()` to quickly operate the
/// XEntry at the next index. If the Cursor perform reset or next and then have a target index that is not able to touch,
/// the Cursor's state will also set to `Inactive`.
///
/// Hence, at any given moment a cursor will be positioned on the XNode and be ready to operate its target XEntry.
/// If not, it means that the cursor is not able to touch the target `XEntry`.
///
/// The cursor will also record all nodes passed from the head node to the target position in `passed_node`,
/// thereby assisting it in performing some operations that involve searching upwards.
///
/// Multiple Cursors are allowed to operate on a single XArray at the same time.
pub struct Cursor<'a, I, M>
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
    /// Record add nodes passed from the head node to the target position.
    /// The index is the layer of the recorded node.
    passed_node: [Option<&'a XNode<I, ReadOnly>>; MAX_LAYER],

    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry, M: ValidMark> Cursor<'a, I, M> {
    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub(crate) fn new(xa: &'a XArray<I, M>, index: u64) -> Self {
        let mut cursor = Self {
            xa,
            index,
            state: CursorState::default(),
            passed_node: [None; MAX_LAYER],
            _marker: PhantomData,
        };

        cursor.traverse_to_target();
        cursor
    }

    /// Move the `Cursor` to the `XNode`, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on.
    fn move_to(&mut self, node: &'a XNode<I, ReadOnly>) -> &'a XEntry<I> {
        let (current_entry, offset) = {
            let offset = node.entry_offset(self.index);
            let current_entry = node.ref_node_entry(offset);
            (current_entry, offset)
        };
        self.state.arrive_node(node, offset);
        current_entry
    }

    /// Reset the target index of the Cursor. Once set, it will immediately attempt to move the Cursor to touch the target XEntry.
    pub fn reset_to(&mut self, index: u64) {
        self.init();
        self.index = index;
        self.traverse_to_target();
    }

    /// Move the target index of the cursor to index + 1.
    /// If the target index's corresponding XEntry is not within the current XNode, the cursor will move to touch the target XEntry.
    /// If the move fails, the cursor's state will be set to `Inactive`.
    pub fn next(&mut self) {
        // TODO: overflow;
        self.index += 1;
        if !self.is_arrived() {
            return;
        }

        if self.xa.max_index() < self.index {
            self.init();
            return;
        }

        let (mut current_node, mut operation_offset) = self.state.node_info().unwrap();
        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            operation_offset = current_node.offset_in_parent() + 1;
            let parent_layer = (*current_node.layer() + 1) as usize;
            self.passed_node[parent_layer - 1] = None;
            current_node = self.passed_node[parent_layer].unwrap();
        }
        self.state.arrive_node(current_node, operation_offset);

        while current_node.layer() != 0 {
            let next_entry = current_node.ref_node_entry(operation_offset);
            if !next_entry.is_node() {
                self.init();
                return;
            }
            let next_node = next_entry.as_node().unwrap();
            self.passed_node[*next_node.layer() as usize] = Some(next_node);
            self.move_to(next_node);
            (current_node, operation_offset) = self.state.node_info().unwrap();
        }
    }

    /// Judge if the target item is marked with the input `mark`.
    /// If target item does not exist, the function will return `None`.
    pub fn is_marked(&mut self, mark: M) -> Option<bool> {
        if let Some((current_node, operation_offset)) = self.state.node_info() {
            Some(current_node.is_marked(operation_offset, mark.index()))
        } else {
            None
        }
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub fn load(&mut self) -> Option<&'a I> {
        if let Some((current_node, operation_offset)) = self.state.node_info() {
            let entry = current_node.ref_node_entry(operation_offset);
            if entry.is_item() {
                return Some(unsafe { &*(entry as *const XEntry<I> as *const I) });
            }
        }
        None
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) -> Option<&'a XEntry<I>> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return Some(current_node.ref_node_entry(operation_offset));
        }

        let max_index = self.xa.max_index();
        if max_index < self.index || max_index == 0 {
            return None;
        }
        self.move_to(self.xa.head().as_node().unwrap());

        let (mut current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = current_node.ref_node_entry(operation_offset);
        while current_layer > 0 {
            if !operated_entry.is_node() {
                self.init();
                return None;
            }

            self.passed_node[*current_layer as usize] = Some(current_node);

            current_node = operated_entry.as_node().unwrap();
            *current_layer -= 1;
            operated_entry = self.move_to(current_node);
        }
        Some(operated_entry)
    }

    /// Initialize the Cursor to its initial state.
    pub fn init(&mut self) {
        self.state = CursorState::default();
        self.passed_node = [None; MAX_LAYER];
    }

    /// Return the target index of the cursor.
    pub fn index(&mut self) -> u64 {
        self.index
    }

    /// Determine whether the cursor arrive at the node that can operate target entry.
    /// It can only be used before or after traversing. Since the cursor will only either not yet started or has already reached the target node
    /// when not in a traversal, it is reasonable to determine whether the cursor has reached its destination node by checking if the cursor is positioned on a node.
    pub fn is_arrived(&mut self) -> bool {
        self.state.is_at_node()
    }
}

/// A `CursorMut` can traverse in the `XArray` and have a target `XEntry` to operate, which is stored in the `index` of `XArray`.
/// `CursorMut` can be only created by an `XArray`, and will hold its mutable reference, and can perform read and write operations
/// for the corresponding `XArray`.
///
/// When creating a `CursorMut`, it will immediately traverses to touch the target XEntry in the XArray. If the `CursorMut` cannot reach to the node
/// that can operate the target XEntry, its state will set to `Inactive`. A `CursorMut` can reset its target index.
/// Once it do this, it will also immediately traverses to touch the target XEntry.  A `CursorMut` can also perform `next()` to quickly operate the
/// XEntry at the next index. If the `CursorMut` perform reset or next and then have a target index that is not able to touch,
/// the `CursorMut`'s state will also set to `Inactive`.
///
/// Hence, at any given moment a `CursorMut` will be positioned on the XNode and be ready to operate its target XEntry.
/// If not, it means that the `CursorMut` is not able to touch the target `XEntry`. For this situation, the `CursorMut`
/// can invoke `store` method which will expand the XArray to guarantee to reach the target position.
///
/// The `CursorMut` will also record all nodes passed from the head node to the target position in passed_node,
/// thereby assisting it in performing some operations that involve searching upwards.
///
/// **Features for COW (Copy-On-Write).** The CursorMut guarantees that all nodes it traverses during the process are exclusively owned by the current XArray.
/// If it finds that the node it is about to access is shared with another XArray due to a COW clone, it will trigger a COW to copy and create an exclusive node for access.
/// Additionally, since it holds a mutable reference to the current XArray, it will not conflict with any other cursors on the XArray.
///
/// When a CursorMut doing operation on XArray, it should not be affected by other CursorMuts or affect other Cursors.
pub struct CursorMut<'a, I, M>
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
    /// Record add nodes passed from the head node to the target position.
    /// The index is the layer of the recorded node.
    passed_node: [Option<&'a XNode<I, ReadWrite>>; MAX_LAYER],

    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry, M: ValidMark> CursorMut<'a, I, M> {
    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub(crate) fn new(xa: &'a mut XArray<I, M>, index: u64) -> Self {
        let mut cursor = Self {
            xa,
            index,
            state: CursorState::default(),
            passed_node: [None; MAX_LAYER],
            _marker: PhantomData,
        };

        cursor.traverse_to_target();
        cursor
    }

    /// Move the `CursorMut` to the `XNode`, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on next.
    fn move_to(&mut self, node: &'a XNode<I, ReadWrite>) -> &'a XEntry<I> {
        let (current_entry, offset) = {
            let offset = node.entry_offset(self.index);
            let current_entry = node.ref_node_entry(offset);
            (current_entry, offset)
        };
        self.state.arrive_node(node, offset);
        current_entry
    }

    /// Reset the target index of the Cursor. Once set, it will immediately attempt to move the Cursor to touch the target XEntry.
    pub fn reset_to(&mut self, index: u64) {
        self.init();
        self.index = index;
        self.traverse_to_target();
    }

    /// Stores the provided `XEntry` in the `XArray` at the position indicated by the current cursor index.
    ///
    /// If the provided entry is the same as the current entry at the cursor position,
    /// the method returns the provided entry without making changes.
    /// Otherwise, it replaces the current entry with the provided one and returns the old entry.
    pub fn store(&mut self, item: I) -> Option<I> {
        let stored_entry = XEntry::from_item(item);
        let target_entry = self.expand_and_traverse_to_target();
        if stored_entry.raw() == target_entry.raw() {
            return XEntry::into_item(stored_entry);
        }
        let (current_node, operation_offset) = self.state.node_info().unwrap();
        let old_entry = current_node.set_entry(operation_offset, stored_entry);
        XEntry::into_item(old_entry)
    }

    /// Move the target index of the cursor to index + 1.
    /// If the target index's corresponding XEntry is not within the current XNode, the cursor will move to touch the target XEntry.
    /// If the move fails, the cursor's state will be set to `Inactive`.
    pub fn next(&mut self) {
        // TODO: overflow;
        self.index += 1;
        if !self.is_arrived() {
            return;
        }

        if self.xa.max_index() < self.index {
            self.init();
            return;
        }

        let (mut current_node, mut operation_offset) = self.state.node_info().unwrap();
        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            operation_offset = current_node.offset_in_parent() + 1;
            let parent_layer = (*current_node.layer() + 1) as usize;
            self.passed_node[parent_layer - 1] = None;
            current_node = self.passed_node[parent_layer].unwrap();
        }
        self.state.arrive_node(current_node, operation_offset);

        while current_node.layer() != 0 {
            let next_entry = current_node.ref_node_entry(operation_offset);
            if !next_entry.is_node() {
                self.init();
                return;
            }
            let next_node = next_entry.as_node_mut().unwrap();
            self.passed_node[*next_node.layer() as usize] = Some(next_node);
            self.move_to(next_node);
            (current_node, operation_offset) = self.state.node_info().unwrap();
        }
    }

    /// Mark the item at the target index in the `XArray` with the input `mark`.
    /// If the item does not exist, return an Error.
    ///
    /// This operation will also mark all nodes along the path from the head node to the target node with the input `mark`,
    /// because a marked intermediate node should be equivalent to having a child node that is marked.
    pub fn set_mark(&mut self, mark: M) -> Result<(), ()> {
        if let Some((current_node, operation_offset)) = self.state.node_info() {
            let item_entry = current_node.ref_node_entry(operation_offset);
            if item_entry.is_null() {
                return Err(());
            }

            current_node.set_mark(operation_offset, mark.index());

            let head_layer = *(self.xa.head().as_node().unwrap().layer()) as usize;
            let mut offset_in_parent = current_node.offset_in_parent();
            let mut parent_layer = (*current_node.layer() + 1) as usize;
            while parent_layer <= head_layer {
                let parent_node = self.passed_node[parent_layer].unwrap();
                if parent_node.is_marked(offset_in_parent, mark.index()) {
                    break;
                }
                parent_node.set_mark(offset_in_parent, mark.index());
                offset_in_parent = parent_node.offset_in_parent();
                parent_layer += 1;
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
    pub fn unset_mark(&mut self, mark: M) -> Result<(), ()> {
        if let Some((mut current_node, operation_offset)) = self.state.node_info() {
            let item_entry = current_node.ref_node_entry(operation_offset);
            if item_entry.is_null() {
                return Err(());
            }

            current_node.unset_mark(operation_offset, mark.index());

            let head_layer = *(self.xa.head().as_node().unwrap().layer()) as usize;
            let mut parent_layer = (*current_node.layer() + 1) as usize;
            while current_node.is_mark_clear(mark.index()) {
                let offset_in_parent = current_node.offset_in_parent();
                let parent_node = self.passed_node[parent_layer].unwrap();
                parent_node.unset_mark(offset_in_parent, mark.index());

                current_node = parent_node;
                parent_layer += 1;
                if parent_layer > head_layer {
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
    pub fn remove(&mut self) -> Option<I> {
        if let Some((current_node, operation_offset)) = self.state.node_info() {
            let old_entry = current_node.set_entry(operation_offset, XEntry::EMPTY);
            return XEntry::into_item(old_entry);
        }
        None
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    /// If can not touch the target entry, the function will return `None`.
    fn traverse_to_target(&mut self) -> Option<&'a XEntry<I>> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return Some(current_node.ref_node_entry(operation_offset));
        }

        let max_index = self.xa.max_index();
        if max_index < self.index || max_index == 0 {
            return None;
        }
        let head = self.xa.head_mut().as_node_mut().unwrap();
        self.move_to(head);

        let (mut current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = current_node.ref_node_entry(operation_offset);
        while current_layer > 0 {
            if !operated_entry.is_node() {
                self.init();
                return None;
            }

            self.passed_node[*current_layer as usize] = Some(current_node);

            current_node = operated_entry.as_node_mut().unwrap();
            *current_layer -= 1;
            operated_entry = self.move_to(current_node);
        }
        Some(operated_entry)
    }

    /// Traverse the XArray and move to the node that can operate the target entry.
    /// During the traverse, the cursor may modify the XArray to let itself be able to reach the target node.
    ///
    /// Before traverse, the cursor will first expand the layer of `XArray` to make sure it have enough capacity.
    /// During the traverse, the cursor will allocate new `XNode` and put it in the appropriate slot if needed.
    ///
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    fn expand_and_traverse_to_target(&mut self) -> &'a XEntry<I> {
        if self.is_arrived() {
            let (current_node, operation_offset) = self.state.node_info().unwrap();
            return current_node.ref_node_entry(operation_offset);
        }

        self.expand_layer();
        let head_ref = self.xa.head_mut().as_node_mut().unwrap();
        self.move_to(head_ref);

        let (mut current_node, operation_offset) = self.state.node_info().unwrap();
        let mut current_layer = current_node.layer();
        let mut operated_entry = current_node.ref_node_entry(operation_offset);
        while current_layer > 0 {
            if !operated_entry.is_node() {
                let new_entry = {
                    let (current_node, operation_offset) = self.state.node_info().unwrap();
                    let new_owned_entry =
                        self.alloc_node(Layer::new(*current_layer - 1), operation_offset);
                    let _ = current_node.set_entry(operation_offset, new_owned_entry);
                    current_node.ref_node_entry(operation_offset)
                };
                operated_entry = new_entry;
            }

            self.passed_node[*current_layer as usize] = Some(current_node);

            current_node = operated_entry.as_node_mut().unwrap();
            *current_layer -= 1;
            operated_entry = self.move_to(current_node);
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
            let head = self.alloc_node(head_layer, 0);
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

                let new_node_entry = self.alloc_node(Layer::new(*head_layer + 1), 0);
                let old_head_entry = self.xa.set_head(new_node_entry);
                let old_head = old_head_entry.as_node_mut().unwrap();
                let new_head = self.xa.head_mut().as_node_mut().unwrap();
                for i in 0..3 {
                    if !old_head.mark(i).is_clear() {
                        new_head.set_mark(0, i);
                    }
                }
                let _empty = new_head.set_entry(0, old_head_entry);
            }
        }
    }

    /// Allocate a new XNode with the specified layer and offset,
    /// then generate a node entry from it and return it to the caller.
    fn alloc_node(&mut self, layer: Layer, offset: u8) -> XEntry<I> {
        XEntry::from_node(XNode::<I, ReadWrite>::new(layer, offset))
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
