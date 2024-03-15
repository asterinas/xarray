use crate::cursor::Cursor;
use crate::entry::{ItemEntry, ItemRef};
use crate::lock::XLock;
use crate::mark::XMark;

/// An iterator over a sub-range of entries in a XArray.
/// This struct is created by the `range()` method on `XArray`.
pub struct Range<'a, I, L, M>
where
    I: ItemEntry,
    L: XLock,
    M: Into<XMark>,
{
    cursor: Cursor<'a, I, L, M>,
    start: u64,
    end: u64,
}

impl<'a, I: ItemEntry, L: XLock, M: Into<XMark>> Range<'a, I, L, M> {
    pub(super) fn new(cursor: Cursor<'a, I, L, M>, start: u64, end: u64) -> Self {
        Range { cursor, start, end }
    }
}

impl<'a, I: ItemEntry, L: XLock, M: Into<XMark>> core::iter::Iterator for Range<'a, I, L, M> {
    type Item = (u64, ItemRef<'a, I>);

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
