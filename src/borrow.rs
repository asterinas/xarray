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

/// A "destroyable" immutable reference.
///
/// The Rust memory model [assumes][1] that all references passed as function arguments (including
/// indirect ones, e.g., structure fields of function arguments) must _not_ be invalidated while
/// the function is running.
///
/// [1]: https://github.com/rust-lang/miri/issues/3186#issuecomment-1826253846
///
/// However, this assumption is not necessarily true in certain cases. For example, when using
/// [`DormantMutRef`], we may want to make sure that all derived references are indeed dead so that
/// the dormant reference can be awakened. To do this, it is necessary to wrap the references in
/// this destroyable type before passing it to a function, otherwise the references will be alive
/// until the function returns.
///
/// By using this type, a reference is converted to a raw pointer, so no assumptions like the ones
/// above are made. Meanwhile, the raw pointer can be safely converted back to the reference for
/// use, since the lifetime is still properly tracked in the type.
pub(super) struct DestroyableRef<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> DestroyableRef<'a, T> {
    /// Creates a destroyable reference from an immutable reference.
    pub fn new(val: &'a T) -> Self {
        DestroyableRef {
            ptr: NonNull::from(val),
            _marker: PhantomData,
        }
    }

    /// Borrows the immutable reference.
    pub fn borrow(&self) -> &'a T {
        // SAFETY: `self` was converted from a value of `&'a T`.
        unsafe { self.ptr.as_ref() }
    }
}

/// A "destroyable" mutable reference.
///
/// For the rationale, see [`DestroyableRef`].
pub(super) struct DestroyableMutRef<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T> DestroyableMutRef<'a, T> {
    /// Creates a destroyable reference from an immutable reference.
    pub fn new(val: &'a mut T) -> Self {
        DestroyableMutRef {
            ptr: NonNull::from(val),
            _marker: PhantomData,
        }
    }

    /// Borrows the mutable reference as an immutable reference.
    pub fn borrow(&self) -> &T {
        // SAFETY: `self` was converted from a value of `&'a mut T`.
        unsafe { self.ptr.as_ref() }
    }

    /// Borrows the mutable reference.
    pub fn borrow_mut(&mut self) -> &mut T {
        // SAFETY: `self` was converted from a value of `&'a mut T`.
        unsafe { self.ptr.as_mut() }
    }

    /// Moves the mutable reference out.
    pub fn into(mut self) -> &'a mut T {
        // SAFETY: `self` was converted from a value of `&'a mut T`.
        unsafe { self.ptr.as_mut() }
    }
}
