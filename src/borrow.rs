use core::{marker::PhantomData, ptr::NonNull};

/// This represents a dormant mutable reference that can be awakened when the original reference
/// and all its derived references are dead.
///
/// See also
/// <https://github.com/rust-lang/rust/blob/35dfc67d94c47a6c6ae28c46e7dc1c547f772485/library/alloc/src/collections/btree/borrow.rs#L14>.
pub(super) struct DormantMutRef<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T> DormantMutRef<'a, T> {
    /// Creates a dormant mutable reference and returns both the original reference and the dormant
    /// reference, so that the original reference can continue to be used.
    pub fn new(val: &'a mut T) -> (&'a mut T, DormantMutRef<'a, T>) {
        let mut ptr = NonNull::from(val);
        // SAFETY: The original reference is still exclusive and can continue to be used.
        let new_val = unsafe { ptr.as_mut() };
        (
            new_val,
            DormantMutRef {
                ptr,
                _marker: PhantomData,
            },
        )
    }

    /// Awakens the dormant references after the original reference and all its derived references
    /// are dead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the original reference and all its derived references
    /// (including derived dormant references) are now dead.
    pub unsafe fn awaken(mut self) -> &'a mut T {
        // SAFETY: The safety requirements of the method ensure that the reference is valid and
        // exclusive.
        unsafe { self.ptr.as_mut() }
    }

    /// Reborrows the dormant references after the original reference and all derived references
    /// are dead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the original reference and all its derived references
    /// (including derived dormant references) are now dead.
    pub unsafe fn reborrow(&mut self) -> &'a mut T {
        // SAFETY: The safety requirements of the method ensure that the reference is valid and
        // exclusive.
        unsafe { self.ptr.as_mut() }
    }
}
