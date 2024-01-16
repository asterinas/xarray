#[derive(Debug, Clone, Copy)]
/// A mark can be used to indicate which slots in an XNode contain items that have been marked.
/// It internally stores a u64, functioning as a bitmap,
/// where each bit that is set to 1 represents a slot at the corresponding offset that has been marked.
pub(crate) struct Mark {
    inner: u64,
}

impl Mark {
    pub(crate) const EMPTY: Self = Self::new(0);

    pub(crate) const fn new(inner: u64) -> Self {
        Self { inner }
    }

    pub(crate) fn set(&mut self, offset: u8) {
        self.inner |= 1 << offset as u64;
    }

    pub(crate) fn unset(&mut self, offset: u8) {
        self.inner &= !(1 << offset as u64);
    }

    pub(crate) fn clear(&mut self) {
        self.inner = 0
    }

    pub(crate) fn is_marked(&self, offset: u8) -> bool {
        (self.inner & 1 << offset as u64) != 0
    }

    pub(crate) fn is_clear(&self) -> bool {
        self.inner == 0
    }
}

// In XArray, an item can have up to three different marks. Users can use a type to distinguish
// which kind of mark they want to set. Such a type must implement the `ValidMark` trait,
// meaning it should be convertible to an index in the range of 0 to 2.
pub trait ValidMark: Copy + Clone {
    /// Map the self type to an index in the range 0 to 2.
    fn index_raw(&self) -> usize;

    /// Users are not required to implement this; it ensures that the mapped index does not exceed 2.
    fn index(&self) -> usize {
        let index = self.index_raw();
        debug_assert!(index < 3);
        index
    }
}

/// A meaningless mark used as a default generic parameter for XArray
/// when marking functionality is not needed.
#[derive(Clone, Copy)]
pub struct NoneMark {}

impl ValidMark for NoneMark {
    fn index_raw(&self) -> usize {
        0
    }
}
