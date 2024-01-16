use crate::*;

/// The COW trait provides the capability for Copy-On-Write (COW) behavior to structures related to XArray,
/// allowing them to perform COW operations on their internal XEntries.
pub(crate) trait Cow<I: ItemEntry> {
    /// Check if the target entry that is about to be operated on need to perform COW.
    /// If the target entry is subject to a mutable operation and is shared with other XArrays,
    /// perform the COW and return the copied XEntry with `Some()`, else return `None`.
    fn copy_if_shared(&self, entry: &XEntry<I>) -> Option<XEntry<I>>;
}

impl<I: ItemEntry> Cow<I> for XNodeInner<I> {
    default fn copy_if_shared(&self, _entry: &XEntry<I>) -> Option<XEntry<I>> {
        None
    }
}

impl<I: ItemEntry + Clone> Cow<I> for XNodeInner<I> {
    fn copy_if_shared(&self, entry: &XEntry<I>) -> Option<XEntry<I>> {
        copy_if_shared(entry)
    }
}

impl<I: ItemEntry, M: ValidMark> Cow<I> for XArray<I, M> {
    default fn copy_if_shared(&self, _entry: &XEntry<I>) -> Option<XEntry<I>> {
        None
    }
}

impl<I: ItemEntry + Clone, M: ValidMark> Cow<I> for XArray<I, M> {
    fn copy_if_shared(&self, entry: &XEntry<I>) -> Option<XEntry<I>> {
        copy_if_shared(entry)
    }
}

fn copy_if_shared<I: ItemEntry + Clone>(entry: &XEntry<I>) -> Option<XEntry<I>> {
    if entry.is_node() && entry.node_strong_count().unwrap() > 1 {
        let new_entry = deep_clone_node_entry(entry);
        Some(new_entry)
    } else {
        None
    }
}
