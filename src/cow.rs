use crate::entry::{ItemEntry, XEntry};
use crate::lock::XLock;
use crate::node::deep_clone_node_entry;

/// The COW trait provides the capability for Copy-On-Write (COW) behavior to XEntries with Clone ability.
pub(super) trait Cow<I: ItemEntry, L: XLock> {
    /// Check if the target entry that is about to be operated on need to perform COW.
    /// If the target entry is subject to a mutable operation and is shared with other XArrays,
    /// perform the COW and return the copied XEntry with `Some()`, else return `None`.
    fn copy_if_shared(&self) -> Option<XEntry<I, L>>;
}

impl<I: ItemEntry, L: XLock> Cow<I, L> for XEntry<I, L> {
    default fn copy_if_shared(&self) -> Option<XEntry<I, L>> {
        None
    }
}

impl<I: ItemEntry + Clone, L: XLock> Cow<I, L> for XEntry<I, L> {
    fn copy_if_shared(&self) -> Option<XEntry<I, L>> {
        if self.is_node() && self.node_strong_count().unwrap() > 1 {
            let new_entry = deep_clone_node_entry(self);
            Some(new_entry)
        } else {
            None
        }
    }
}
