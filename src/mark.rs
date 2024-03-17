#[derive(Debug, Clone, Copy)]
/// A mark can be used to indicate which slots in an XNode contain items that have been marked.
/// It internally stores a u64, functioning as a bitmap,
/// where each bit that is set to 1 represents a slot at the corresponding offset that has been marked.
pub(super) struct Mark {
    inner: u64,
}

impl Mark {
    pub const EMPTY: Self = Self::new(0);

    pub const fn new(inner: u64) -> Self {
        Self { inner }
    }

    pub fn set(&mut self, offset: u8) {
        self.inner |= 1 << offset as u64;
    }

    pub fn unset(&mut self, offset: u8) {
        self.inner &= !(1 << offset as u64);
    }

    pub fn update(&mut self, offset: u8, set: bool) -> bool {
        let mut new_inner = self.inner;
        if set {
            new_inner |= 1 << offset as u64;
        } else {
            new_inner &= !(1 << offset as u64);
        }

        let changed = new_inner != self.inner;
        self.inner = new_inner;

        changed
    }

    pub fn clear(&mut self) {
        self.inner = 0
    }

    pub fn is_marked(&self, offset: u8) -> bool {
        (self.inner & 1 << offset as u64) != 0
    }

    pub fn is_clear(&self) -> bool {
        self.inner == 0
    }
}

/// The mark type used in the XArray. The XArray itself and an item in it can have up to three different marks.
///
/// Users can use a self-defined type to distinguish which kind of mark they want to set.
/// Such a type must implement the `Into<XMark>` trait,
pub enum XMark {
    Mark0,
    Mark1,
    Mark2,
}

pub const NUM_MARKS: usize = 3;

impl XMark {
    /// Map the XMark to an index in the range 0 to 2.
    pub(super) fn index(&self) -> usize {
        match self {
            XMark::Mark0 => 0,
            XMark::Mark1 => 1,
            XMark::Mark2 => 2,
        }
    }
}

/// A meaningless mark used as a default generic parameter for XArray
/// when marking functionality is not needed.
#[derive(Clone, Copy)]
pub struct NoneMark {}

impl Into<XMark> for NoneMark {
    fn into(self) -> XMark {
        panic!("NoneMark can not be used!");
    }
}
