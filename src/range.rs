use crate::cursor::Cursor;
use crate::entry::ItemEntry;
use crate::mark::XMark;

/// An iterator over a range of entries in an [`XArray`].
///
/// The typical way to obtain a `Range` instance is to call [`XArray::range`].
///
/// [`XArray`]: crate::XArray
/// [`XArray::range`]: crate::XArray::range
pub struct Range<'a, I, M>
where
    I: ItemEntry,
    M: Into<XMark>,
{
    cursor: Cursor<'a, I, M>,
    end: u64,
}

impl<'a, I: ItemEntry, M: Into<XMark>> Range<'a, I, M> {
    pub(super) fn new(cursor: Cursor<'a, I, M>, end: u64) -> Self {
        Range { cursor, end }
    }
}

impl<'a, I: ItemEntry, M: Into<XMark>> core::iter::Iterator for Range<'a, I, M> {
    type Item = (u64, I::Ref<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.cursor.index() >= self.end {
                return None;
            }

            let item = self.cursor.load();
            if item.is_none() {
                self.cursor.next();
                continue;
            }

            let res = item.map(|item| (self.cursor.index(), item));
            self.cursor.next();
            return res;
        }
    }
}
