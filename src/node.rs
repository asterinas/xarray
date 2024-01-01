use std::{
    marker::PhantomData,
    sync::{Arc, Mutex, RwLock, Weak},
};

use crate::*;

/// XArray中的节点结构，每一个节点都由一个RwLock管理
/// 其中shift、offset用于基本操作计算，count和nr_value服务于Multi-index entries
pub struct XNode<I: ItemEntry> {
    shift: u8,
    offset: u8,
    count: u8,
    nr_value: u8,
    parent: XEntry,
    slots: [XEntry; CHUNK_SIZE],
    marks: [Mark; 3],
    _marker: PhantomData<I>,
}

impl<I: ItemEntry> XNode<I> {
    pub(crate) fn new(shift: u8, offset: u8, parent: XEntry) -> Self {
        Self {
            shift,
            offset,
            count: 0,
            nr_value: 0,
            parent: parent,
            slots: [XEntry::EMPTY; CHUNK_SIZE],
            marks: [Mark { inner: 0 }; 3],
            _marker: PhantomData,
        }
    }

    pub(crate) fn mark_mut(&mut self, index: usize) -> &mut Mark {
        &mut self.marks[index]
    }

    pub(crate) fn mark(&self, index: usize) -> &Mark {
        &self.marks[index]
    }

    pub(crate) const fn get_offset(&self, index: u64) -> u8 {
        ((index >> self.shift as u64) & CHUNK_MASK as u64) as u8
    }

    pub(crate) fn entry(&self, index: u8) -> &XEntry {
        &self.slots[index as usize]
    }

    pub(crate) fn set_node_entry(
        &mut self,
        offset: u8,
        entry: OwnedEntry<I, Node>,
    ) -> Option<OwnedEntry<I, Node>> {
        let old_entry = OwnedEntry::<I, Node>::from_raw(self.slots[offset as usize]);
        self.slots[offset as usize] = OwnedEntry::<I, Node>::into_raw(entry);
        old_entry
    }

    pub(crate) fn set_item_entry(
        &mut self,
        offset: u8,
        entry: OwnedEntry<I, Item>,
    ) -> Option<OwnedEntry<I, Item>> {
        let old_entry = OwnedEntry::<I, Item>::from_raw(self.slots[offset as usize]);
        self.slots[offset as usize] = OwnedEntry::<I, Item>::into_raw(entry);
        old_entry
    }

    pub(crate) fn max_index(&self) -> u64 {
        ((CHUNK_SIZE as u64) << (self.shift as u64)) - 1
    }

    pub(crate) fn shift(&self) -> u8 {
        self.shift
    }

    pub(crate) fn offset(&self) -> u8 {
        self.offset
    }

    pub(crate) fn parent(&self) -> XEntry {
        self.parent
    }

    pub(crate) fn set_offset(&mut self, offset: u8) {
        self.offset = offset;
    }

    pub(crate) fn set_parent(&mut self, parent: XEntry) {
        self.parent = parent;
    }

    pub(crate) fn set_shift(&mut self, shift: u8) {
        self.shift = shift;
    }
}

impl<I: ItemEntry> Drop for XNode<I> {
    fn drop(&mut self) {
        for entry in self.slots {
            if entry.is_node() {
                OwnedEntry::<I, Node>::from_raw(entry);
            }
            if entry.is_item() {
                OwnedEntry::<I, Item>::from_raw(entry);
            }
        }
    }
}
