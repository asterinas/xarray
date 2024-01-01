use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop, MaybeUninit},
    sync::{Arc, RwLock},
};

use crate::*;

/// 用来抽象用户将要存入XArray的指针类型
/// 可以为诸如Arc、Box等实现，用户也可以自己为自定义的指针类型实现该trait
pub trait PointerItem {
    /// 将原有的指针类型转化为一个裸指针
    ///
    /// # Safety
    /// 用户需要确保该裸指针与原指针在内存布局上一致
    /// 也就是原指针大小跟裸指针大小一致，且指向的位置一致
    /// 同时需要确保原指针是aligned to 4的，因为XArray中的entry后两位要用于entry类型判断。
    unsafe fn into_raw(self) -> *const u8;

    /// 将一个裸指针转化为原指针
    ///
    /// # Safety
    /// 裸指针必须是通过into_raw生成得到的，同时需要注意所有权的恢复。
    unsafe fn from_raw(ptr: *const u8) -> Self;
}

impl<T> PointerItem for Box<T> {
    unsafe fn into_raw(self) -> *const u8 {
        let raw_ptr = Box::into_raw(self) as *const u8;
        debug_assert!(raw_ptr.is_aligned_to(4));
        raw_ptr
    }

    unsafe fn from_raw(ptr: *const u8) -> Self {
        Box::from_raw(ptr as *mut _)
    }
}

impl<T> PointerItem for Arc<T> {
    unsafe fn into_raw(self) -> *const u8 {
        let raw_ptr = unsafe { core::intrinsics::transmute::<Arc<T>, *const u8>(self) };
        debug_assert!(raw_ptr.is_aligned_to(4));
        raw_ptr
    }

    unsafe fn from_raw(ptr: *const u8) -> Self {
        let arc = core::intrinsics::transmute::<*const u8, Arc<T>>(ptr);
        arc
    }
}

/// 用来抽象用户将要存入XArray的类型，这些存入XArray的obj称之为item。
/// 对于存入的item，要求其大小为4字节，可以是各种指针类型或者是usize、u64等整数类型。
pub(crate) trait ItemEntry {
    /// 用户读取存储Item时的返回类型
    type Target<'a>
    where
        Self: 'a;

    /// 由原类型生成usize，消耗原类型所有权，该usize将直接存入XArray的XEntry。
    fn into_raw(self) -> usize;

    /// 由usize恢复原类型，恢复所有权
    ///
    /// # Safety
    /// 传入的raw必须是由into_raw生成的
    unsafe fn from_raw(raw: usize) -> Self;

    /// 读取该类型对应的XEntry，返回用户需要的读取类型
    ///
    /// # Safety
    /// 需要确保entry是一个item_entry，同时需要确保原类型仍有效
    unsafe fn load_item<'a>(entry: &'a XEntry) -> Self::Target<'a>;
}

impl<I: PointerItem> ItemEntry for I {
    type Target<'a> = &'a Self where Self: 'a;

    fn into_raw(self) -> usize {
        let raw_ptr = unsafe { I::into_raw(self) };
        raw_ptr as usize
    }

    unsafe fn from_raw(raw: usize) -> Self {
        I::from_raw(raw as *const u8)
    }

    unsafe fn load_item<'a>(entry: &'a XEntry) -> Self::Target<'a> {
        debug_assert!(entry.is_item());
        &*(entry as *const XEntry as *const I)
    }
}

impl ItemEntry for usize {
    type Target<'a> = usize;

    fn into_raw(self) -> usize {
        debug_assert!(self <= usize::MAX >> 1);
        (self << 1) | 1
    }

    unsafe fn from_raw(raw: usize) -> Self {
        raw >> 1
    }

    unsafe fn load_item<'a>(entry: &'a XEntry) -> Self::Target<'a> {
        Self::from_raw(entry.raw)
    }
}

pub(crate) struct Item {}

pub(crate) struct Node {}

/// XArray中有所有权的Entry，只有两种Type，Item以及Node，分别对应ItemEntry以及指向Node的Entry
/// 指向Node的Entry目前有统一的结构类型，也就是Arc<RwLock<XNode<I>>>。
#[derive(Eq)]
#[repr(transparent)]
pub struct OwnedEntry<I: ItemEntry, Type> {
    raw: usize,
    _marker: core::marker::PhantomData<(I, Type)>,
}

impl<I: ItemEntry, Type> PartialEq for OwnedEntry<I, Type> {
    fn eq(&self, o: &Self) -> bool {
        self.raw == o.raw
    }
}

impl<I: ItemEntry + Clone> Clone for OwnedEntry<I, Item> {
    fn clone(&self) -> Self {
        let cloned_entry = unsafe {
            let item_entry = ManuallyDrop::new(I::from_raw(self.raw));
            OwnedEntry::from_item((*item_entry).clone())
        };
        return cloned_entry;
    }
}

impl<I: ItemEntry> Clone for OwnedEntry<I, Node> {
    fn clone(&self) -> Self {
        unsafe {
            Arc::increment_strong_count((self.raw - 2) as *const RwLock<XNode<I>>);
        }
        Self {
            raw: self.raw,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<I: ItemEntry, Type> Drop for OwnedEntry<I, Type> {
    fn drop(&mut self) {
        if self.is_item() {
            unsafe {
                I::from_raw(self.raw);
            }
        }
        if self.is_node() {
            unsafe {
                Arc::from_raw((self.raw - 2) as *const RwLock<XNode<I>>);
            }
        }
    }
}

impl<I: ItemEntry> OwnedEntry<I, Item> {
    pub(crate) fn into_raw(self) -> XEntry {
        let raw_entry = XEntry { raw: self.raw };
        let _ = ManuallyDrop::new(self);
        raw_entry
    }

    pub(crate) fn from_raw(raw_entry: XEntry) -> Option<Self> {
        if raw_entry.is_item() {
            Some(Self {
                raw: raw_entry.raw,
                _marker: PhantomData,
            })
        } else {
            None
        }
    }

    pub(crate) fn from_item(item: I) -> Self {
        let raw = I::into_raw(item);
        Self::new(raw as usize)
    }
}

impl<I: ItemEntry> OwnedEntry<I, Node> {
    pub(crate) fn into_raw(self) -> XEntry {
        let raw_entry = XEntry { raw: self.raw };
        let _ = ManuallyDrop::new(self);
        raw_entry
    }

    pub(crate) fn from_raw(raw_entry: XEntry) -> Option<Self> {
        if raw_entry.is_node() {
            Some(Self {
                raw: raw_entry.raw,
                _marker: PhantomData,
            })
        } else {
            None
        }
    }

    pub(crate) fn from_node(node: XNode<I>) -> Self {
        let node_ptr = {
            let arc_node = Arc::new(RwLock::new(node));
            Arc::into_raw(arc_node)
        };
        Self::new(node_ptr as usize | 2)
    }

    pub(crate) fn as_node(&self) -> &RwLock<XNode<I>> {
        unsafe {
            let node_ref = &*((self.raw - 2) as *const RwLock<XNode<I>>);
            node_ref
        }
    }
}

impl<I: ItemEntry, Type> OwnedEntry<I, Type> {
    pub(crate) const fn new(raw: usize) -> Self {
        Self {
            raw,
            _marker: core::marker::PhantomData,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        self.raw == 0
    }

    pub(crate) fn is_internal(&self) -> bool {
        self.raw & 3 == 2
    }

    pub(crate) fn is_item(&self) -> bool {
        !self.is_null() && !self.is_internal()
    }

    pub(crate) fn is_node(&self) -> bool {
        self.is_internal() && self.raw > (CHUNK_SIZE << 2)
    }
}

/// 储存在XArray的head以及XNode的slots中的类型，XArray中存储的基本单位
/// 有以下这样的类型分类
/// - internal entries 用户不可见的内部entries 后两位bit为10
///     - node entry 指向XNode的entry
///     - sibling entry (用于multi-index entries), empty entry, retry entry (用于异常处理)
/// - item entry 用户存储的item
///     - pointer entry 后两位为00, (tagged pointer entry)
///     - value entry 末位为1
///
/// XEntry没有所有权，可以copy，
/// 指向XNode和表示Item的涉及到所有权的XEntry必须要由OwnedEntry调用into_raw获取，
/// 这个操作只会发生在XArray的set_head以及XNode的set_slot过程中，
/// 一获得XEntry就会将其存储在head或slot里，并将旧的XEntry恢复成OwnedEntry。
/// 此操作相当于将OwnedEntry的所有权转移到了XArray和XNode上，
/// 二者的drop期间需要负责将这些XEntry恢复成OwnedEntry
///
/// XArray和XNode的clone也要负责实际的OwnedEntry的clone
///
#[derive(Eq, Copy, Clone)]
#[repr(transparent)]
pub struct XEntry {
    raw: usize,
}

impl PartialEq for XEntry {
    fn eq(&self, o: &Self) -> bool {
        self.raw == o.raw
    }
}

impl XEntry {
    pub(crate) const EMPTY: Self = Self::new(0);

    pub(crate) const fn new(raw: usize) -> Self {
        Self { raw }
    }

    pub(crate) fn is_null(&self) -> bool {
        self.raw == 0
    }

    pub(crate) fn is_internal(&self) -> bool {
        self.raw & 3 == 2
    }

    pub(crate) fn is_item(&self) -> bool {
        !self.is_null() && !self.is_internal()
    }

    pub(crate) fn is_node(&self) -> bool {
        self.is_internal() && self.raw > (CHUNK_SIZE << 2)
    }

    pub(crate) fn is_sibling(&self) -> bool {
        self.is_internal() && self.raw < (((CHUNK_SIZE - 1) << 2) | 2)
    }

    pub(crate) fn as_node<I: ItemEntry>(&self) -> Option<&RwLock<XNode<I>>> {
        if self.is_node() {
            unsafe {
                let node_ref = &*((self.raw - 2) as *const RwLock<XNode<I>>);
                Some(node_ref)
            }
        } else {
            None
        }
    }

    pub(crate) fn as_sibling(&self) -> Option<u8> {
        if self.is_sibling() {
            Some((self.raw >> 2).try_into().unwrap())
        } else {
            None
        }
    }
}

impl<I: ItemEntry, Type> PartialEq<OwnedEntry<I, Type>> for XEntry {
    fn eq(&self, o: &OwnedEntry<I, Type>) -> bool {
        self.raw == o.raw
    }
}
