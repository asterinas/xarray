use std::marker::PhantomData;

use crate::*;

/// A `Cursor` can traverse in the `XArray` and have a target operated `XEntry`, which is stored in the `index` of `XArray`.
/// `Cursor` can be only created by an `XArray`, and will hold its immutable reference, and can only perform read-only operations
/// for the corresponding `XArray`.
/// When a cursor traverses an XArray, at any given moment, it is positioned on an XNode. If not, it means that
/// the cursor has not begin to traverse. Its member `offset` indicates the next XEntry it will move to, which is the `slots[offset]` inside the current XNode.
///
/// At the same time, multiple Cursors are allowed to operate on a single XArray.
pub(crate) struct Cursor<'a, I>
where
    I: ItemEntry,
{
    /// The `XArray` the cursor located in.
    xa: &'a XArray<I>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// The next XEntry to be operated on is at 'offset' in the slots of the current XNode.
    offset: u8,
    /// Current positioned XNode.
    current_node: Option<&'a XNode<I, ReadOnly>>,
    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry> Cursor<'a, I> {
    /// Create an `Cursor` to perform read related operations on the `XArray`.
    pub(crate) fn new(xa: &'a XArray<I>, index: u64) -> Self {
        Self {
            xa,
            index,
            offset: 0,
            current_node: None,
            _marker: PhantomData,
        }
    }

    /// Move the `Cursor` to the `XNode` that `node_entry` points to, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on.
    fn move_to(&mut self, node_entry: &'a XEntry<I>) -> Option<RefEntry<'a, I>> {
        if let Some(node) = node_entry.as_node() {
            let (current_entry, offset) = {
                let offset = node.entry_offset(self.index);
                let current_entry = node.entry(offset);
                (current_entry, offset)
            };
            self.current_node = Some(node);
            self.offset = offset;
            Some(current_entry)
        } else {
            None
        }
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub(crate) fn load(&mut self) -> Option<&'a XEntry<I>> {
        if let Some(node) = self.xa.head().as_node() {
            if (self.index >> node.height()) as u64 > SLOT_MASK as u64 {
                self.current_node = None;
                return None;
            }
        } else {
            return None;
        }

        // # Safety
        // Because there won't be another concurrent modification operation, the `current_entry` is valid.
        let mut current_entry = RefEntry::<'a>::new(self.xa.head());
        while let Some(node) = unsafe { current_entry.as_entry().as_node() } {
            if node.height() == 0 {
                break;
            }
            current_entry = unsafe { self.move_to(current_entry.as_entry()).unwrap() };
        }
        unsafe {
            self.move_to(current_entry.as_entry())
                .map(|ref_entry| ref_entry.as_entry())
        }
    }

    /// Initialize the Cursor to its initial state.
    fn init(&mut self) {
        self.current_node = None;
        self.offset = 0;
    }

    /// Return the target index of the cursor.
    fn index(&mut self) -> u64 {
        self.index
    }
}

/// A `CursorMut` can traverse in the `XArray` and have a target operated `XEntry`, which is stored in the `index` of `XArray`.
/// `Cursor` can be only created by an `XArray`, and will hold its mutable reference, and can perform read and write operations
/// for the corresponding `XArray`.
/// When a cursor traverses an XArray, at any given moment, it is positioned on an XNode. If not, it means that
/// the cursor has not begin to traverse. Its member `offset` indicates the next XEntry it will move to, which is the `slots[offset]` inside the current XNode.
///
/// When a CursorMut doing operation on XArray, it should not be affected by other CursorMuts or affect other Cursors.
pub(crate) struct CursorMut<'a, I>
where
    I: ItemEntry,
{
    /// The `XArray` the cursor located in.
    xa: &'a mut XArray<I>,
    /// The target index of the cursor in the belonged `XArray`.
    index: u64,
    /// The next XEntry to be operated on is at 'offset' in the slots of the current XNode.
    offset: u8,
    /// Current positioned XNode.
    current_node: Option<&'a XNode<I, ReadWrite>>,
    _marker: PhantomData<I>,
}

impl<'a, I: ItemEntry> CursorMut<'a, I> {
    /// Create an `CursorMut` to perform read and write operations on the `XArray`.
    pub(crate) fn new(xa: &'a mut XArray<I>, index: u64) -> Self {
        Self {
            xa,
            index,
            offset: 0,
            current_node: None,
            _marker: PhantomData,
        }
    }

    /// Move the `CursorMut` to the `XNode` that `node_entry` points to, and update the cursor's state based on its target index.
    /// Return a reference to the `XEntry` within the slots of the current XNode that needs to be operated on next.
    fn move_to(&mut self, node_entry: &'a XEntry<I>) -> Option<RefEntry<'a, I>> {
        if let Some(node) = node_entry.as_node_mut() {
            let (current_entry, offset) = {
                let offset = node.entry_offset(self.index);
                let current_entry = node.entry(offset);
                (current_entry, offset)
            };
            self.current_node = Some(node);
            self.offset = offset;
            Some(current_entry)
        } else {
            None
        }
    }

    /// Load the `XEntry` at the current cursor index within the `XArray`.
    ///
    /// Returns a reference to the `XEntry` at the target index if succeed.
    /// If the cursor cannot reach to the target index, the method will return `None`.
    pub(crate) fn load(&mut self) -> Option<&'a XEntry<I>> {
        if let Some(node) = self.xa.head().as_node() {
            if (self.index >> node.height()) as u64 > SLOT_MASK as u64 {
                self.current_node = None;
                return None;
            }
        } else {
            return None;
        }
        // # Safety
        // Because there won't be another concurrent modification operation, the `current_entry` is valid.
        let mut current_entry = RefEntry::<'a>::new(self.xa.head());
        while let Some(node) = unsafe { current_entry.as_entry().as_node() } {
            if node.height() == 0 {
                break;
            }
            current_entry = unsafe { self.move_to(current_entry.as_entry()).unwrap() };
        }
        unsafe {
            self.move_to(current_entry.as_entry())
                .map(|ref_entry| ref_entry.as_entry())
        }
    }

    /// Stores the provided `XEntry` in the `XArray` at the position indicated by the current cursor index.
    ///
    /// If the provided entry is the same as the current entry at the cursor position,
    /// the method returns the provided entry without making changes.
    /// Otherwise, it replaces the current entry with the provided one and returns the old entry.
    pub(crate) fn store(&mut self, entry: XEntry<I>) -> XEntry<I> {
        let current_entry = self.traverse();
        if entry.raw() == current_entry.raw() {
            return entry;
        }
        let node = self.current_node.unwrap();
        let old_entry = node.set_entry(self.offset, entry);
        return old_entry;
    }

    /// Removes the `XEntry` at the target index of the 'CursorMut' within the `XArray`.
    ///
    /// This is achieved by storing an empty `XEntry` at the target index using the `store` method.
    /// The method returns the replaced `XEntry` that was previously stored at the target index.
    pub(crate) fn remove(&mut self) -> XEntry<I> {
        self.store(XEntry::EMPTY)
    }

    /// Traverse the subtree based on the target index starting from the head node.
    /// Move continuously until reaching the `XNode` capable of storing the target index.
    /// It then returns the reference to the `XEntry` stored in the slot corresponding to the target index.
    /// A target operated XEntry must be an item entry.
    ///
    /// Before traverse, the cursor will first expand the height of `XArray` to make sure it have enough capacity.
    /// During the traverse, the cursor will allocate new `XNode` and put it in the appropriate slot if needed.
    fn traverse(&mut self) -> &'a XEntry<I> {
        let mut current_height = self.expand_height();
        let mut ref_entry = RefEntry::<'a>::new(self.xa.head_mut());
        // When the target entry has not been reached, the cursor will continue to move downward,
        // and if it encounters a situation where there is no XNode, it will allocate an XNode.
        //
        // # Safety
        // Because there won't be another concurrent modification operation, the `ref_entry` is valid.
        while current_height > 0 {
            let current_entry = unsafe { self.move_to(ref_entry.as_entry()).unwrap().as_entry() };
            current_height -= NODE_HEIGHT as u8;
            if let None = current_entry.as_node() {
                let new_entry = {
                    let new_owned_entry = self.alloc_node(current_height, self.offset);
                    let node = self.current_node.unwrap();
                    let _ = node.set_entry(self.offset, new_owned_entry);
                    node.entry(self.offset)
                };
                ref_entry = new_entry;
            } else {
                ref_entry = RefEntry::<'a>::new(current_entry);
            }
        }
        let k = unsafe { self.move_to(ref_entry.as_entry()).unwrap().as_entry() };
        k
    }

    /// Increase the height of XArray to expand its capacity, allowing it to accommodate the target index,
    /// and returns the height of the final head node.
    ///
    /// If the head node of the XArray does not exist, allocate a new head node of appropriate height directly.
    /// Otherwise, if needed, repeatedly insert new nodes on top of the current head node to serve as the new head.
    fn expand_height(&mut self) -> u8 {
        if self.xa.head().is_null() {
            let mut head_height = 0;
            while (self.index >> head_height) as usize >= SLOT_SIZE {
                head_height += NODE_HEIGHT as u8;
            }
            let head = self.alloc_node(head_height, 0);
            self.xa.set_head(head);
            return head_height;
        } else {
            loop {
                let (capacity, head_height) = {
                    let head = self.xa.head().as_node().unwrap();
                    (head.max_index(), head.height())
                };

                if capacity > self.index {
                    return head_height;
                }

                let new_node = self.alloc_node(head_height + NODE_HEIGHT as u8, 0);
                let old_head_entry = self.xa.set_head(new_node);

                let new_head = self.xa.head_mut().as_node_mut().unwrap();
                let _empty = new_head.set_entry(0, old_head_entry);
            }
        }
    }

    /// Allocate a new XNode with the specified height and offset,
    /// then generate a node entry from it and return it to the caller.
    fn alloc_node(&mut self, height: u8, offset: u8) -> XEntry<I> {
        XEntry::from_node(XNode::<I, ReadWrite>::new(height, offset))
    }

    /// Initialize the Cursor to its initial state.
    fn init(&mut self) {
        self.current_node = None;
        self.offset = 0;
    }

    /// Return the target index of the cursor.
    pub(crate) fn index(&mut self) -> u64 {
        self.index
    }
}
