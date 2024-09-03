use core::{
    alloc::{GlobalAlloc, Layout},
    mem, ptr,
};

use crate::allocator::align_up;

use super::Locked;

struct ListNode {
    size: usize,
    next: Option<&'static mut ListNode>,
}
impl ListNode {
    const fn new(size: usize) -> Self {
        ListNode { size, next: None }
    }

    fn start_addr(&self) -> usize {
        self as *const Self as usize
    }

    fn end_addr(&self) -> usize {
        self.start_addr() + self.size
    }
}
pub struct LinkedListAllocator {
    head: ListNode,
}

impl LinkedListAllocator {
    /// 创建一个空的 LinkedListAllocator。
    pub const fn new() -> Self {
        Self {
            head: ListNode::new(0),
        }
    }

    /// 使用给定的堆边界初始化分配器。
    ///
    /// 这个函数是不安全的，因为调用者必须保证给定的堆边界是有效的，并且堆是未使用的。这个方法只能被调用一次。
    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.add_free_region(heap_start, heap_size);
    }

    /// 将给定的内存区域添加到列表的前面。
    unsafe fn add_free_region(&mut self, addr: usize, size: usize) {
        // 确保释放的区域能够容纳 ListNode
        assert_eq!(align_up(addr, mem::align_of::<ListNode>()), addr);
        assert!(size >= mem::size_of::<ListNode>());

        // 创建一个新的列表节点并将其附加到列表的开头
        let mut node = ListNode::new(size);
        node.next = self.head.next.take();
        let node_ptr = addr as *mut ListNode;
        node_ptr.write(node);
        self.head.next = Some(&mut *node_ptr)
    }
    /// 查找具有给定大小和对齐方式的空闲区域，并将其从列表中移除。
    ///
    /// 返回一个包含列表节点和分配起始地址的元组。
    fn find_region(&mut self, size: usize, align: usize) -> Option<(&'static mut ListNode, usize)> {
        // 对当前列表节点的引用，每次迭代都会更新
        let mut current = &mut self.head;
        // 在链表中查找足够大的内存区域
        while let Some(ref mut region) = current.next {
            if let Ok(alloc_start) = Self::alloc_from_region(&region, size, align) {
                // 适合分配的区域 -> 从列表中移除节点
                let next = region.next.take();
                let ret = Some((current.next.take().unwrap(), alloc_start));
                current.next = next;
                return ret;
            } else {
                // 区域不适合 -> 继续下一个区域
                current = current.next.as_mut().unwrap();
            }
        }

        // 未找到合适的区域
        None
    }
    /// Try to use the given region for an allocation with given size and
    /// alignment.
    ///
    /// Returns the allocation start address on success.
    fn alloc_from_region(region: &ListNode, size: usize, align: usize) -> Result<usize, ()> {
        let alloc_start = align_up(region.start_addr(), align);
        let alloc_end = alloc_start.checked_add(size).ok_or(())?;

        if alloc_end > region.end_addr() {
            // region too small
            return Err(());
        }

        let excess_size = region.end_addr() - alloc_end;
        if excess_size > 0 && excess_size < mem::size_of::<ListNode>() {
            // rest of region too small to hold a ListNode (required because the
            // allocation splits the region in a used and a free part)
            return Err(());
        }

        // region suitable for allocation
        Ok(alloc_start)
    }
    /// 调整给定的布局，以便生成的分配内存区域也能够存储 `ListNode`。
    ///
    /// 返回调整后的大小和对齐方式作为 (size, align) 元组。
    fn size_align(layout: Layout) -> (usize, usize) {
        let layout = layout
            .align_to(mem::align_of::<ListNode>())
            .expect("adjusting alignment failed")
            .pad_to_align();
        let size = layout.size().max(mem::size_of::<ListNode>());
        (size, layout.align())
    }
}
unsafe impl GlobalAlloc for Locked<LinkedListAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // 执行布局调整
        let (size, align) = LinkedListAllocator::size_align(layout);
        let mut allocator = self.lock();

        if let Some((region, alloc_start)) = allocator.find_region(size, align) {
            let alloc_end = alloc_start.checked_add(size).expect("overflow");
            let excess_size = region.end_addr() - alloc_end;
            if excess_size > 0 {
                allocator.add_free_region(alloc_end, excess_size);
            }
            alloc_start as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // 执行布局调整
        let (size, _) = LinkedListAllocator::size_align(layout);

        self.lock().add_free_region(ptr as usize, size)
    }
}
