use std::{
    marker::PhantomData,
    sync::{Arc, RwLock, Weak},
};

use crate::*;

pub enum CurrentState {
    Empty,
    Bound,
    Restart,
    Node(XEntry),
}

impl CurrentState {
    pub(crate) fn get<I: ItemEntry>(&self) -> Option<&RwLock<XNode<I>>> {
        if let Self::Node(node) = self {
            node.as_node()
        } else {
            None
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub(crate) fn is_restart(&self) -> bool {
        matches!(self, Self::Restart)
    }

    pub(crate) fn is_bound(&self) -> bool {
        matches!(self, Self::Bound)
    }
}

pub struct State<I: ItemEntry, M: XMark> {
    pub index: u64,
    pub shift: u8,
    pub sibs: u8,
    pub offset: u8,
    pub node: CurrentState,
    _marker: PhantomData<(I, M)>,
}

/// 一个时刻只能获得一个node的写锁
impl<I: ItemEntry, M: XMark> State<I, M> {
    pub fn new(index: u64) -> Self {
        State {
            index,
            shift: 0,
            sibs: 0,
            offset: 0,
            node: CurrentState::Restart,
            _marker: PhantomData,
        }
    }

    /// xa_state移动到目标node_entry, 返回在当前node中，对应index应该操作的下一个entry。
    fn move_to<'a>(&mut self, node_entry: XEntry) -> Option<&'a XEntry> {
        if let Some(node) = node_entry.as_node::<I>() {
            let node_read = node.read().unwrap();
            let offset = node_read.get_offset(self.index);
            let op_entry = node_read.entry(offset);
            // if let Some(sib) = entry.as_sibling() {
            //     offset = sib;
            //     op_entry = node_read.entry(offset);
            // }
            self.node = CurrentState::Node(node_entry);
            self.offset = offset;
            unsafe { Some(&*(op_entry as *const XEntry)) }
        } else {
            None
        }
    }

    pub fn load<'a>(&mut self, xa: &'a XArray<I, M>) -> &'a XEntry {
        let mut current_node = self.node.get::<I>();
        let mut op_entry = {
            if let Some(node) = current_node {
                let node_read = node.read().unwrap();
                let op_entry = node_read.entry(self.offset);
                *op_entry
            } else {
                if let Some(node) = xa.head().as_node::<I>() {
                    let node_read = node.read().unwrap();
                    if (self.index >> node_read.shift()) as u64 > CHUNK_MASK as u64 {
                        self.node = CurrentState::Bound;
                        XEntry::EMPTY
                    } else {
                        *xa.head()
                    }
                } else {
                    XEntry::EMPTY
                }
            }
        };
        while let Some(node) = op_entry.as_node::<I>() {
            let node_read = node.read().unwrap();
            if self.shift > node_read.shift() {
                break;
            }
            if node_read.shift() == 0 {
                break;
            }
            drop(node_read);
            op_entry = *self.move_to(op_entry).unwrap();
        }
        self.move_to(op_entry).unwrap()
    }

    pub fn store(
        &mut self,
        xa: &mut XArray<I, M>,
        entry: Option<OwnedEntry<I, Item>>,
    ) -> Option<OwnedEntry<I, Item>> {
        let op_entry = self.create(xa);
        // TODO: Multi-index entry
        if entry.as_ref().is_some_and(|entry| *op_entry == *entry) {
            return entry;
        }
        let node = self.node.get().unwrap();
        let mut node_write = node.write().unwrap();
        let old_entry = node_write.set_item_entry(self.offset, entry);
        return old_entry;
    }

    /// 不断创建新的Node, 直到得到目标index的entry
    fn create(&mut self, xa: &mut XArray<I, M>) -> &XEntry {
        let mut shift = 0;
        // Normal
        if let None = self.node.get::<I>() {
            self.clear_state();
            // 将当前的树先扩展为足够支持index储存的形式
            // 此时self操作的node为head节点
            shift = self.expand(xa);
            self.move_to(*xa.head());
        } else {
            shift = self.shift;
        }

        let mut entry = {
            let node = self.node.get::<I>().unwrap();
            let node_read = node.read().unwrap();
            *node_read.entry(self.offset)
        };
        while shift > 0 {
            shift -= CHUNK_SHIFT as u8;
            // if entry.is_item() {
            //     break;
            // }
            if let None = entry.as_node::<I>() {
                let node_entry = {
                    let new_entry = self.alloc(shift);
                    let node = self.node.get().unwrap();
                    let mut node_write = node.write().unwrap();
                    let _ = node_write.set_node_entry(self.offset, new_entry);
                    *node_write.entry(self.offset)
                };
                entry = node_entry;
            }
            if shift <= 0 {
                break;
            }
            entry = *self.move_to(entry).unwrap();
        }
        self.move_to(entry).unwrap()
        // obsidian
    }

    /// 在根节点处增加若干节点，以增加树高。
    /// array -> head -> ...
    /// -> array -> new_node -> ... -> head -> ...
    fn expand(&mut self, xa: &mut XArray<I, M>) -> u8 {
        let mut shift = 0;
        let mut capacity = 0;

        // 有head node，则可以直接知道当前XArray的最大容纳量，shift赋值为当前最大shift + 6。
        if let Some(node) = xa.head().as_node::<I>() {
            let node_read = node.read().unwrap();
            shift = node_read.shift() + CHUNK_SHIFT as u8;
            capacity = node_read.max_index();
        }
        // 没有head的情况，则计算所需要的shift，直接返回，在上层函数直接create根节点，赋值相应shift进行存储。
        else {
            while (self.index >> shift) as usize >= CHUNK_SIZE {
                shift += CHUNK_SHIFT as u8;
            }
            let head = self.alloc(shift);
            capacity = max_index(shift);
            xa.set_head(head);
            return shift + CHUNK_SHIFT as u8;
        }

        // 指向空节点
        self.clear_state();
        while self.index > capacity {
            // 创建一个新的head，原本的head作为新node的child
            let node_entry = self.alloc(shift);
            let old_head_entry = xa.set_head(node_entry).unwrap();
            let new_head_entry = xa.head();

            let mut head_write = old_head_entry.as_node().write().unwrap();
            head_write.set_offset(0);
            head_write.set_parent(*new_head_entry);
            capacity = head_write.max_index();
            drop(head_write);

            let new_head_node = new_head_entry.as_node::<I>().unwrap();
            let mut node_write = new_head_node.write().unwrap();
            node_write.set_node_entry(0, old_head_entry);

            shift += CHUNK_SHIFT as u8;
        }
        self.node = CurrentState::Node(*xa.head());
        shift
    }

    /// Alloc a node entry as a slot for the XState operated node.
    fn alloc(&mut self, shift: u8) -> OwnedEntry<I, Node> {
        let (parent, offset) = {
            if let CurrentState::Node(entry) = self.node {
                (entry, self.offset)
            } else {
                (XEntry::EMPTY, 0)
            }
        };
        debug_assert!(parent.is_null() || parent.is_node());
        OwnedEntry::<I, Node>::from_node(XNode::new(shift, offset, parent))
    }

    fn clear_state(&mut self) {
        self.node = CurrentState::Empty;
        self.offset = 0;
    }
}

fn max_index(shift: u8) -> u64 {
    ((CHUNK_SIZE as u64) << (shift as u64)) - 1
}
