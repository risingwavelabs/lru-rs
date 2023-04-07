extern crate hashbrown;
use alloc::alloc::Global;
use alloc::borrow::Borrow;
use alloc::boxed::Box;
use hashbrown::HashMap;
use std::alloc::Allocator;
use std::fmt;
use std::hash::{BuildHasher, Hash};
// use std::iter::FusedIterator;
// use std::marker::PhantomData;
use std::mem;
use std::ptr;

use crate::{DefaultHasher, KeyRef};

extern crate alloc;

type Epoch = u64;

struct IndexedLruEntry<K, V> {
    key: mem::MaybeUninit<K>,
    val: mem::MaybeUninit<V>,
    prev: *mut IndexedLruEntry<K, V>,
    next: *mut IndexedLruEntry<K, V>,
    epoch: Epoch,
    index: u32,
    dropped: bool,
}

impl<K, V> IndexedLruEntry<K, V> {
    fn new(key: K, val: V, epoch: Epoch, index: u32) -> Self {
        IndexedLruEntry {
            key: mem::MaybeUninit::new(key),
            val: mem::MaybeUninit::new(val),
            prev: ptr::null_mut(),
            next: ptr::null_mut(),
            epoch,
            index,
            dropped: false,
        }
    }

    fn new_sigil() -> Self {
        IndexedLruEntry {
            key: mem::MaybeUninit::uninit(),
            val: mem::MaybeUninit::uninit(),
            prev: ptr::null_mut(),
            next: ptr::null_mut(),
            epoch: 0,
            index: 0,
            dropped: false,
        }
    }
}

pub struct IndexedLruCache<K, V, S = DefaultHasher, A: Clone + Allocator = Global> {
    map: HashMap<KeyRef<K>, Box<IndexedLruEntry<K, V>, A>, S, A>,
    cap: usize,
    ghost_cap: usize,
    ghost_len: usize,

    // head and tail are sigil nodes to faciliate inserting entries
    head: *mut IndexedLruEntry<K, V>,
    ghost_head: *mut IndexedLruEntry<K, V>,
    tail: *mut IndexedLruEntry<K, V>,

    /// used for epoch based eviction
    cur_epoch: Epoch,

    alloc: A,
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    pub fn with_hasher_in(cap: usize, hash_builder: S, alloc: A) -> Self {
        IndexedLruCache::construct_in(
            cap,
            HashMap::with_capacity_and_hasher_in(cap, hash_builder, alloc.clone()),
            alloc,
        )
    }

    pub fn unbounded_with_hasher_in(hash_builder: S, alloc: A) -> Self {
        IndexedLruCache::construct_in(
            usize::MAX,
            HashMap::with_hasher_in(hash_builder, alloc.clone()),
            alloc,
        )
    }

    /// Creates a new LRU Cache with the given capacity and allocator.
    fn construct_in(
        cap: usize,
        map: HashMap<KeyRef<K>, Box<IndexedLruEntry<K, V>, A>, S, A>,
        alloc: A,
    ) -> IndexedLruCache<K, V, S, A> {
        // NB: The compiler warns that cache does not need to be marked as mutable if we
        // declare it as such since we only mutate it inside the unsafe block.
        let head = Box::into_raw(Box::new_in(IndexedLruEntry::new_sigil(), alloc.clone()));
        let tail = Box::into_raw(Box::new_in(IndexedLruEntry::new_sigil(), alloc.clone()));
        let ghost_head = tail;
        let cache = IndexedLruCache {
            map,
            cap,
            ghost_cap: 2,
            ghost_len: 0,
            head,
            ghost_head,
            tail,
            cur_epoch: 0,
            alloc,
        };

        unsafe {
            (*cache.head).next = cache.tail;
            (*cache.tail).prev = cache.head;
        }

        cache
    }
}

impl<K: Hash + Eq, V> IndexedLruCache<K, V> {
    /// Creates a new LRU Cache that holds at most `cap` items.
    ///
    /// # Example
    ///
    /// ```
    /// use lru::IndexedLruCache;
    /// let mut cache: IndexedLruCache<isize, &str> = IndexedLruCache::new(10);
    /// ```
    pub fn new(cap: usize) -> IndexedLruCache<K, V> {
        IndexedLruCache::construct_in(cap, HashMap::with_capacity(cap), Global)
    }

    /// Creates a new LRU Cache that never automatically evicts items.
    ///
    /// # Example
    ///
    /// ```
    /// use lru::IndexedLruCache;
    /// let mut cache: IndexedLruCache<isize, &str> = IndexedLruCache::unbounded();
    /// ```
    pub fn unbounded() -> IndexedLruCache<K, V> {
        IndexedLruCache::construct_in(usize::MAX, HashMap::default(), Global)
    }
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    pub fn put(&mut self, k: K, v: V, index: u32) {
        self.capturing_put(k, v, false, index)
    }

    fn capturing_put(&mut self, k: K, mut v: V, _capture: bool, index: u32) {
        let node_ref = self.map.get_mut(&KeyRef { k: &k });

        match node_ref {
            Some(node_ref) => {
                let node_ptr: *mut IndexedLruEntry<K, V> = &mut **node_ref;

                // if the key is already in the cache just update its value and move it to the
                // front of the list
                unsafe {
                    (*node_ptr).index = index;
                    if (*node_ptr).dropped {
                        (*node_ptr).dropped = false;
                        (*node_ptr).val = mem::MaybeUninit::new(v);
                    } else {
                        mem::swap(&mut v, &mut (*(*node_ptr).val.as_mut_ptr()) as &mut V)
                    }
                }
                self.detach(node_ptr);
                self.attach(node_ptr);
            }
            None => {
                // if the capacity is zero, do nothing
                if self.cap() == 0 {
                    return;
                }
                let (_, mut node) = self.replace_or_create_node(k, v, index);

                let node_ptr: *mut IndexedLruEntry<K, V> = &mut *node;
                self.attach(node_ptr);

                let keyref = unsafe { (*node_ptr).key.as_ptr() };
                self.map.insert(KeyRef { k: keyref }, node);
            }
        }
    }

    fn replace_or_create_node(
        &mut self,
        k: K,
        v: V,
        index: u32,
    ) -> (Option<(K, V)>, Box<IndexedLruEntry<K, V>, A>) {
        if self.len() == self.cap() {
            panic!("self.len() == self.cap()");
            // if the cache is full, remove the last entry so we can use it for the new key
            // let old_key = KeyRef {
            //     k: unsafe { &(*(*(*self.tail).prev).key.as_ptr()) },
            // };
            // let mut old_node = self.map.remove(&old_key).unwrap();

            // // read out the node's old key and value and then replace it
            // let replaced = unsafe { (old_node.key.assume_init(), old_node.val.assume_init()) };

            // old_node.key = mem::MaybeUninit::new(k);
            // old_node.val = mem::MaybeUninit::new(v);

            // let node_ptr: *mut IndexedLruEntry<K, V> = &mut *old_node;
            // self.detach(node_ptr);

            // (Some(replaced), old_node)
        } else {
            // if the cache is not full allocate a new IndexedLruEntry
            (
                None,
                Box::<_, A>::new_in(
                    IndexedLruEntry::new(k, v, self.cur_epoch, index),
                    self.alloc.clone(),
                ),
            )
        }
    }

    pub fn peek_mut<'a, Q>(&'a mut self, k: &Q) -> (Option<&'a mut V>, Option<u32>)
    where
        KeyRef<K>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        match self.map.get_mut(k) {
            None => (None, None),
            Some(node) => {
                let index = (*node).index;
                let dropped = (*node).dropped;
                if dropped {
                    (None, Some(index))
                } else {
                    (
                        Some(unsafe { &mut (*(*node).val.as_mut_ptr()) as &mut V }),
                        Some(index),
                    )
                }
            }
        }
    }

    pub fn get_mut<'a, Q>(&'a mut self, k: &Q, new_index: u32) -> (Option<&'a mut V>, Option<u32>)
    where
        KeyRef<K>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(node) = self.map.get_mut(k) {
            let index = (*node).index;
            let dropped = (*node).dropped;
            let node_ptr: *mut IndexedLruEntry<K, V> = &mut **node;

            if dropped {
                (None, Some(index))
            } else {
                (*node).index = new_index;
                self.detach(node_ptr);
                self.attach(node_ptr);
                (
                    Some(unsafe { &mut (*(*node_ptr).val.as_mut_ptr()) as &mut V }),
                    Some(index),
                )
            }
        } else {
            (None, None)
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn contains<Q>(&self, k: &Q) -> bool
    where
        KeyRef<K>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(k)
    }

    pub fn is_empty(&self) -> bool {
        self.map.len() == 0 || unsafe { (*self.ghost_head).prev == self.head }
    }

    pub fn update_epoch(&mut self, epoch: Epoch) {
        assert!(epoch > self.cur_epoch);
        self.cur_epoch = epoch;
    }

    pub fn evict_by_epoch(&mut self, epoch: Epoch) {
        loop {
            if self.is_empty() {
                break;
            }

            let node = unsafe { (*self.ghost_head).prev };
            let node_epoch = unsafe { (*node).epoch };
            if node_epoch < epoch {
                let old_key = KeyRef {
                    k: unsafe { &(*(*node).key.as_ptr()) },
                };
                let mut old_node = self.map.get_mut(&old_key).unwrap();
                unsafe {
                    ptr::drop_in_place(old_node.val.as_mut_ptr());
                }
                old_node.dropped = true;
                // println!("WKXLOG: len: {}", self.ghost_len);
                self.ghost_len += 1;
                self.ghost_head = unsafe { (*self.ghost_head).prev };
                if self.ghost_len > self.ghost_cap {
                    // println!("WKXLOG: len: {}, cap: {}", self.ghost_len, self.ghost_cap);

                    let tail = unsafe { (*self.tail).prev };
                    let tail_key = KeyRef {
                        k: unsafe { &(*(*tail).key.as_ptr()) },
                    };
                    // println!("WKXLOG: before map len: {}", self.map.len());

                    let mut tail_node = self.map.remove(&tail_key).unwrap();
                    // println!("WKXLOG: after map len: {}", self.map.len());

                    unsafe {
                        ptr::drop_in_place(tail_node.key.as_mut_ptr());
                    }
                    let tail_node_ptr: *mut IndexedLruEntry<K, V> = &mut *tail_node;

                    self.detach(tail_node_ptr);
                    self.ghost_len -= 1;
                }
            } else {
                break;
            }
        }

        self.map.shrink_to_fit();
    }

    pub fn pop_lru(&mut self) -> Option<K> {
        let node = self.remove_last()?;
        // N.B.: Can't destructure directly because of https://github.com/rust-lang/rust/issues/28536
        let node = *node;
        let IndexedLruEntry { key, val, dropped, .. } = node;
        unsafe {
            if !dropped {
                let _val = val.assume_init();
            }
            Some(key.assume_init())
        }
    }

    pub fn clear(&mut self) {
        while self.pop_lru().is_some() {}
    }

    fn remove_last(&mut self) -> Option<Box<IndexedLruEntry<K, V>, A>> {
        let prev;
        unsafe { prev = (*self.tail).prev }
        if prev != self.head {
            let old_key = KeyRef {
                k: unsafe { &(*(*(*self.tail).prev).key.as_ptr()) },
            };
            let mut old_node = self.map.remove(&old_key).unwrap();
            let node_ptr: *mut IndexedLruEntry<K, V> = &mut *old_node;
            self.detach(node_ptr);
            Some(old_node)
        } else {
            None
        }
    }

    fn detach(&mut self, node: *mut IndexedLruEntry<K, V>) {
        unsafe {
            (*(*node).prev).next = (*node).next;
            (*(*node).next).prev = (*node).prev;
        }
    }

    fn attach(&mut self, node: *mut IndexedLruEntry<K, V>) {
        unsafe {
            (*node).epoch = self.cur_epoch;
            (*node).next = (*self.head).next;
            (*node).prev = self.head;
            (*self.head).next = node;
            (*(*node).next).prev = node;
        }
    }
}

unsafe impl<K: Send, V: Send, S: Send, A: Clone + Allocator + Send> Send
    for IndexedLruCache<K, V, S, A>
{
}
unsafe impl<K: Sync, V: Sync, S: Sync, A: Clone + Allocator + Sync> Sync
    for IndexedLruCache<K, V, S, A>
{
}

impl<K: Hash + Eq, V> fmt::Debug for IndexedLruCache<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LruCache")
            .field("len", &self.len())
            .field("cap", &self.cap())
            .finish()
    }
}

impl<K, V, S, A: Clone + Allocator> Drop for IndexedLruCache<K, V, S, A> {
    fn drop(&mut self) {
        self.map.values_mut().for_each(|e| unsafe {
            ptr::drop_in_place(e.key.as_mut_ptr());
            if !e.dropped {
                ptr::drop_in_place(e.val.as_mut_ptr());
            }
        });
        // We rebox the head/tail, and because these are maybe-uninit
        // they do not have the absent k/v dropped.
        unsafe {
            let _head = *Box::from_raw_in(self.head, self.alloc.clone());
            let _tail = *Box::from_raw_in(self.tail, self.alloc.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IndexedLruCache;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn test_evict_by_epoch() {
        let mut cache = IndexedLruCache::new(10);

        cache.put(1, "a", 1);
        cache.put(2, "b", 2);

        cache.update_epoch(1);

        cache.put(3, "c", 3);
        cache.put(4, "d", 4);

        cache.evict_by_epoch(1);

        assert_eq!(cache.len(), 4);
        let (val, index) = cache.peek_mut(&1);
        assert!(val.is_none());
        assert_eq!(index.unwrap(), 1);

        let (val, index) = cache.peek_mut(&2);
        assert!(val.is_none());
        assert_eq!(index.unwrap(), 2);

        let (val, index) = cache.peek_mut(&3);
        assert_eq!(val, Some(&mut "c"));
        assert_eq!(index.unwrap(), 3);

        let (val, index) = cache.peek_mut(&4);
        assert_eq!(val, Some(&mut "d"));
        assert_eq!(index.unwrap(), 4);

        cache.evict_by_epoch(2);

        assert_eq!(cache.len(), 2);
        let (val, index) = cache.peek_mut(&3);
        assert!(val.is_none());
        assert_eq!(index.unwrap(), 3);

        let (val, index) = cache.peek_mut(&4);
        assert!(val.is_none());
        assert_eq!(index.unwrap(), 4);
    }

    #[test]
    fn test_no_memory_leaks_evict_by_epoch() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;

        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let n = 100usize;

        for _ in 0..n {
            DROP_COUNT.store(0, Ordering::SeqCst);
            let mut cache = IndexedLruCache::unbounded();
            for i in 1..n + 1 {
                cache.update_epoch(i as u64);
                cache.put(i, DropCounter {}, 10);
            }
            cache.evict_by_epoch(51);
            assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 50);
            assert_eq!(cache.len(), 52);
        }
    }

    #[test]
    fn test_no_memory_leaks_with_clear() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;

        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let n = 100;
        for _ in 0..n {
            let mut cache = IndexedLruCache::unbounded();
            for i in 0..n {
                cache.put(i, DropCounter {}, 10);
            }
            cache.clear();
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), n * n);
    }

}
