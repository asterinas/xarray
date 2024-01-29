use super::*;

/// The COW trait provides the capability for Copy-On-Write (COW) behavior to structures related to XArray,
/// allowing them to perform COW operations on their internal XEntries.
pub(super) trait Cow<I: ItemEntry, L: XLock> {
    /// Check if the target entry that is about to be operated on need to perform COW.
    /// If the target entry is subject to a mutable operation and is shared with other XArrays,
    /// perform the COW and return the copied XEntry with `Some()`, else return `None`.
    fn copy_if_shared(&self, entry: &XEntry<I, L>) -> Option<XEntry<I, L>>;
}

impl<I: ItemEntry, L: XLock> Cow<I, L> for XNode<I, L, ReadWrite> {
    default fn copy_if_shared(&self, _entry: &XEntry<I, L>) -> Option<XEntry<I, L>> {
        None
    }
}

impl<I: ItemEntry + Clone, L: XLock> Cow<I, L> for XNode<I, L, ReadWrite> {
    fn copy_if_shared(&self, entry: &XEntry<I, L>) -> Option<XEntry<I, L>> {
        copy_if_shared(entry)
    }
}

impl<I: ItemEntry, L: XLock, M: ValidMark> Cow<I, L> for XArray<I, L, M> {
    default fn copy_if_shared(&self, _entry: &XEntry<I, L>) -> Option<XEntry<I, L>> {
        None
    }
}

impl<I: ItemEntry + Clone, L: XLock, M: ValidMark> Cow<I, L> for XArray<I, L, M> {
    fn copy_if_shared(&self, entry: &XEntry<I, L>) -> Option<XEntry<I, L>> {
        copy_if_shared(entry)
    }
}

fn copy_if_shared<I: ItemEntry + Clone, L: XLock>(entry: &XEntry<I, L>) -> Option<XEntry<I, L>> {
    if entry.is_node() && entry.node_strong_count().unwrap() > 1 {
        let new_entry = deep_clone_node_entry(entry);
        Some(new_entry)
    } else {
        None
    }
}
