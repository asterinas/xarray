use core::{marker::PhantomData, ptr::NonNull};

pub(super) struct DormantMutRef<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T> DormantMutRef<'a, T> {
    pub fn new(val: &'a mut T) -> (&'a mut T, DormantMutRef<'a, T>) {
        let mut ptr = NonNull::from(val);
        let new_val = unsafe { ptr.as_mut() };
        (
            new_val,
            DormantMutRef {
                ptr,
                _marker: PhantomData,
            },
        )
    }

    pub unsafe fn awaken(mut self) -> &'a mut T {
        unsafe { self.ptr.as_mut() }
    }

    pub unsafe fn reborrow(&mut self) -> &'a mut T {
        unsafe { self.ptr.as_mut() }
    }
}
