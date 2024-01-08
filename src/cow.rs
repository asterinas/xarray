use crate::*;

/// Provide a method for XArray and XNode to check whether copy-on-write is necessary and perform it.
pub trait CowCheck<I: ItemEntry> {
    /// By examining the target entry that is about to be operated on,
    /// perform copy-on-write when the target entry is subject to a mutable operation and is shared with other XArrays.
    fn copy_on_write<'a>(&'a mut self, entry: &'a XEntry<I>, offset: u8) -> &'a XEntry<I>;
}

impl<I: ItemEntry> CowCheck<I> for XNodeInner<I> {
    default fn copy_on_write<'a>(&'a mut self, entry: &'a XEntry<I>, _offset: u8) -> &'a XEntry<I> {
        entry
    }
}

impl<I: ItemEntry + Clone> CowCheck<I> for XNodeInner<I> {
    fn copy_on_write<'a>(&'a mut self, entry: &'a XEntry<I>, offset: u8) -> &'a XEntry<I> {
        if entry.is_node() && entry.node_strong_count().unwrap() > 1 {
            let new_entry = deep_clone_node_entry(entry);
            let _ = self.set_entry(offset, new_entry);
            self.entry(offset)
        } else {
            entry
        }
    }
}

impl<I: ItemEntry> CowCheck<I> for XArray<I> {
    default fn copy_on_write<'a>(&'a mut self, entry: &'a XEntry<I>, _offset: u8) -> &'a XEntry<I> {
        entry
    }
}

impl<I: ItemEntry + Clone> CowCheck<I> for XArray<I> {
    fn copy_on_write<'a>(&'a mut self, entry: &'a XEntry<I>, _offset: u8) -> &'a XEntry<I> {
        if entry.is_node() && entry.node_strong_count().unwrap() > 1 {
            let new_entry = deep_clone_node_entry(entry);
            let _ = self.set_head(new_entry);
            self.head()
        } else {
            entry
        }
    }
}
