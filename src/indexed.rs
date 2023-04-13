extern crate hashbrown;
use alloc::alloc::Global;
use alloc::borrow::Borrow;
use alloc::boxed::Box;
use hashbrown::HashMap;
use std::alloc::Allocator;
use std::cmp::min;
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
    /// without ghost
    cap: usize,
    /// ghost
    ghost_cap: usize,
    ghost_len: usize,

    // head and tail are sigil nodes to faciliate inserting entries
    head: *mut IndexedLruEntry<K, V>,
    ghost_head: *mut IndexedLruEntry<K, V>,
    tail: *mut IndexedLruEntry<K, V>,

    /// used for epoch based eviction
    cur_epoch: Epoch,

    /// for index
    pub(crate) global_index: u32,
    current_index_count: u32,
    update_interval: u32,
    pub(crate) counters: HashMap<u32, u32>,

    /// for index for ghost
    pub(crate) ghost_global_index: u32,
    ghost_current_index_count: u32,
    ghost_update_interval: u32,
    pub(crate) ghost_counters: HashMap<u32, u32>,

    alloc: A,
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    pub fn with_hasher_in(
        cap: usize,
        hash_builder: S,
        alloc: A,
        ghost_cap: usize,
        update_interval: u32,
    ) -> Self {
        IndexedLruCache::construct_in(
            cap,
            ghost_cap,
            update_interval,
            HashMap::with_capacity_and_hasher_in(cap, hash_builder, alloc.clone()),
            alloc,
        )
    }

    pub fn unbounded_with_hasher_in(
        hash_builder: S,
        alloc: A,
        ghost_cap: usize,
        update_interval: u32,
    ) -> Self {
        IndexedLruCache::construct_in(
            usize::MAX,
            ghost_cap,
            update_interval,
            HashMap::with_hasher_in(hash_builder, alloc.clone()),
            alloc,
        )
    }

    /// Creates a new LRU Cache with the given capacity and allocator.
    fn construct_in(
        cap: usize,
        ghost_cap: usize,
        update_interval: u32,
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
            ghost_cap,
            ghost_len: 0,
            head,
            ghost_head,
            tail,
            cur_epoch: 0,
            alloc,
            global_index: 0,
            current_index_count: 0,
            update_interval,
            counters: HashMap::new(),
            ghost_global_index: 0,
            ghost_current_index_count: 0,
            ghost_update_interval: ((min(ghost_cap, usize::MAX - 10) + 10) / 10) as u32,
            ghost_counters: HashMap::new(),
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
    pub fn new(cap: usize, ghost_cap: usize, update_interval: u32) -> IndexedLruCache<K, V> {
        IndexedLruCache::construct_in(
            cap,
            ghost_cap,
            update_interval,
            HashMap::with_capacity(cap),
            Global,
        )
    }

    /// Creates a new LRU Cache that never automatically evicts items.
    ///
    /// # Example
    ///
    /// ```
    /// use lru::IndexedLruCache;
    /// let mut cache: IndexedLruCache<isize, &str> = IndexedLruCache::unbounded();
    /// ```
    pub fn unbounded(ghost_cap: usize, update_interval: u32) -> IndexedLruCache<K, V> {
        IndexedLruCache::construct_in(
            usize::MAX,
            ghost_cap,
            update_interval,
            HashMap::default(),
            Global,
        )
    }
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    #[inline]
    fn get_index(&mut self) -> u32 {
        if self.current_index_count >= self.update_interval {
            assert_eq!(self.current_index_count, self.update_interval);
            self.counters
                .insert(self.global_index, self.update_interval);
            self.current_index_count = 0;
            self.global_index += 1;
        }
        self.current_index_count += 1;
        self.global_index
    }

    #[inline]
    fn get_ghost_index(&mut self) -> u32 {
        if self.ghost_current_index_count >= self.ghost_update_interval {
            assert_eq!(self.ghost_current_index_count, self.ghost_update_interval);
            self.ghost_counters
                .insert(self.ghost_global_index, self.ghost_update_interval);
            self.ghost_current_index_count = 0;
            self.ghost_global_index += 1;
        }
        self.ghost_current_index_count += 1;
        self.ghost_global_index
    }

    fn update_counters(&mut self, old_index: &u32) {
        if *old_index == self.global_index {
            self.current_index_count -= 1;
        } else {
            *self.counters.get_mut(old_index).unwrap() -= 1;
        }
    }

    fn update_ghost_counters(&mut self, old_index: &u32) {
        if *old_index == self.ghost_global_index {
            self.ghost_current_index_count -= 1;
        } else {
            *self.ghost_counters.get_mut(old_index).unwrap() -= 1;
        }
        self.ghost_len -= 1;
    }

    fn shift_real_tail_to_ghost(&mut self) {
        let node = unsafe { (*self.ghost_head).prev };
        // drop value
        let new_index = self.get_ghost_index();
        let old_index;
        // make ghost
        unsafe {
            assert!(!(*node).dropped);
            ptr::drop_in_place((*node).val.as_mut_ptr());
            (*node).dropped = true;
            old_index = (*node).index;
            (*node).index = new_index;
        }
        self.update_counters(&old_index);

        // update global
        self.ghost_len += 1;
        self.ghost_head = unsafe { (*self.ghost_head).prev };
    }
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    pub fn put(&mut self, k: K, mut v: V) {
        let node_ref = self.map.get_mut(&KeyRef { k: &k });

        match node_ref {
            Some(node_ref) => {
                let old_index = node_ref.index;
                let is_ghost = node_ref.dropped;
                let node_ptr: *mut IndexedLruEntry<K, V> = &mut **node_ref;
                unsafe {
                    if is_ghost {
                        // move to real
                        (*node_ptr).index = self.get_index();
                        (*node_ptr).dropped = false;
                        (*node_ptr).val = mem::MaybeUninit::new(v);

                        if node_ptr == self.ghost_head {
                            self.ghost_head = (*self.ghost_head).next;
                        }
                        // delete from ghost
                        self.update_ghost_counters(&old_index);
                        self.detach(node_ptr);
                        self.attach(node_ptr);
                        // if real is full, shift
                        if self.len() >= self.cap() {
                            assert_eq!(self.len(), self.cap());
                            assert!(self.ghost_len < self.ghost_cap);
                            self.shift_real_tail_to_ghost();
                        }
                    } else {
                        self.update_counters(&old_index);
                        if old_index != self.global_index {
                            (*node_ptr).index = self.get_index();
                        }
                        mem::swap(&mut v, &mut (*(*node_ptr).val.as_mut_ptr()) as &mut V);
                        self.detach(node_ptr);
                        self.attach(node_ptr);
                    }
                }
            }
            None => {
                // if the capacity is zero, do nothing
                if self.cap() == 0 {
                    return;
                }
                let index = self.get_index();
                let mut node = self.replace_or_create_node(k, v, index);

                let node_ptr: *mut IndexedLruEntry<K, V> = &mut *node;
                self.attach(node_ptr);

                let keyref = unsafe { (*node_ptr).key.as_ptr() };
                self.map.insert(KeyRef { k: keyref }, node);
            }
        }
    }

    /// `peek_mut` does not update the LRU list so the key's position will be unchanged.
    /// if in real cache, return v and index.
    /// if in ghost cache, return none and index, remove it.
    /// if not in map, return none and none.
    pub fn peek_mut<'a, Q>(&'a mut self, k: &Q) -> Option<&'a mut V>
    where
        KeyRef<K>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        match self.map.get_mut(k) {
            None => None,
            Some(node) => {
                // let old_index = (*node).index;
                let is_ghost = (*node).dropped;
                if is_ghost {
                    // report when update
                    // remove from ghost
                    // let mut node = self.map.remove(&k).unwrap();
                    // unsafe {
                    //     ptr::drop_in_place(node.key.as_mut_ptr());
                    // }
                    // let node_ptr: *mut IndexedLruEntry<K, V> = &mut *node;
                    // if node_ptr == self.ghost_head {
                    //     self.ghost_head = unsafe { (*self.ghost_head).next };
                    // }
                    // self.detach(node_ptr);
                    // self.update_ghost_counters(&old_index);
                    // // destructure
                    // let _node = *node;
                    None
                } else {
                    Some(unsafe { &mut (*(*node).val.as_mut_ptr()) as &mut V })
                }
            }
        }
    }

    /// Moves the key to the head of the LRU list if it exists.
    /// if in real cache, return v and index, move it.
    /// if in ghost cache && check_ghost, return none and index.
    /// if in ghost cache && !check_ghost, return none and none, remove it.
    /// if not in map, return none and none.
    pub fn get_mut<'a, Q>(&'a mut self, k: &Q, check_ghost: bool) -> Option<&'a mut V>
    where
        KeyRef<K>: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(node) = self.map.get_mut(k) {
            let is_ghost = (*node).dropped;
            let node_ptr: *mut IndexedLruEntry<K, V> = &mut **node;

            if !check_ghost && is_ghost {
                None
            } else if check_ghost && is_ghost {
                todo!()
                // self.ghost_len -= 1;
                // let mut node = self.map.remove(&k).unwrap();
                // unsafe {
                //     ptr::drop_in_place(node.key.as_mut_ptr());
                // }
                // let node_ptr: *mut IndexedLruEntry<K, V> = &mut *node;
                // self.detach(node_ptr);
                // None
            } else {
                // must be real cache
                let old_index = (*node).index;
                self.update_counters(&old_index);
                if old_index != self.global_index {
                    unsafe {
                        (*node_ptr).index = self.get_index();
                    }
                }

                self.detach(node_ptr);
                self.attach(node_ptr);
                Some(unsafe { &mut (*(*node_ptr).val.as_mut_ptr()) as &mut V })
            }
        } else {
            None
        }
    }

    pub fn evict_by_epoch(&mut self, epoch: Epoch) {
        loop {
            if self.is_real_empty() {
                break;
            }

            let node = unsafe { (*self.ghost_head).prev };
            let node_epoch = unsafe { (*node).epoch };
            if node_epoch < epoch {
                self.shift_real_tail_to_ghost();
                if self.ghost_len > self.ghost_cap {
                    let tail = unsafe { (*self.tail).prev };
                    let tail_key = KeyRef {
                        k: unsafe { &(*(*tail).key.as_ptr()) },
                    };
                    let mut tail_node = self.map.remove(&tail_key).unwrap();
                    unsafe {
                        ptr::drop_in_place(tail_node.key.as_mut_ptr());
                    }
                    let tail_node_ptr: *mut IndexedLruEntry<K, V> = &mut *tail_node;
                    if tail_node_ptr == self.ghost_head {
                        self.ghost_head = unsafe { (*self.ghost_head).next };
                    }
                    self.detach(tail_node_ptr);
                    self.update_ghost_counters(&tail_node.index);
                    let _tail_node = *tail_node;
                }
            } else {
                break;
            }
        }

        self.map.shrink_to_fit();
    }

    fn replace_or_create_node(&mut self, k: K, v: V, index: u32) -> Box<IndexedLruEntry<K, V>, A> {
        if self.len() == self.cap() && self.ghost_cap > 0 {
            // return shift real tail to ghost
            self.shift_real_tail_to_ghost();
            if self.ghost_len > self.ghost_cap {
                // return tail of ghost
                let old_key_ghost = KeyRef {
                    k: unsafe { &(*(*(*self.tail).prev).key.as_ptr()) },
                };
                let mut old_node_ghost = self.map.remove(&old_key_ghost).unwrap();

                // read out the node's old key and value and then replace it
                unsafe {
                    ptr::drop_in_place(old_node_ghost.key.as_mut_ptr());
                }
                let old_index = old_node_ghost.index;
                self.update_ghost_counters(&old_index);

                old_node_ghost.dropped = false;
                old_node_ghost.key = mem::MaybeUninit::new(k);
                old_node_ghost.val = mem::MaybeUninit::new(v);
                old_node_ghost.index = index;

                let node_ptr_ghost: *mut IndexedLruEntry<K, V> = &mut *old_node_ghost;
                if node_ptr_ghost == self.ghost_head {
                    self.ghost_head = unsafe { (*self.ghost_head).next };
                }
                self.detach(node_ptr_ghost);

                old_node_ghost
            } else {
                Box::<_, A>::new_in(
                    IndexedLruEntry::new(k, v, self.cur_epoch, index),
                    self.alloc.clone(),
                )
            }
        } else if self.len() == self.cap() {
            // if the cache is full, remove the last entry so we can use it for the new key
            let old_key = KeyRef {
                k: unsafe { &(*(*(*self.tail).prev).key.as_ptr()) },
            };
            let mut old_node = self.map.remove(&old_key).unwrap();

            // read out the node's old key and value and then replace it
            unsafe {
                let _ = (old_node.key.assume_init(), old_node.val.assume_init());
            }
            let old_index = old_node.index;
            old_node.key = mem::MaybeUninit::new(k);
            old_node.val = mem::MaybeUninit::new(v);

            let node_ptr: *mut IndexedLruEntry<K, V> = &mut *old_node;
            self.update_counters(&old_index);
            self.detach(node_ptr);
            old_node
        } else {
            // if the cache is not full allocate a new IndexedLruEntry
            Box::<_, A>::new_in(
                IndexedLruEntry::new(k, v, self.cur_epoch, index),
                self.alloc.clone(),
            )
        }
    }

    // pub fn contains<Q>(&self, k: &Q) -> bool
    // where
    //     KeyRef<K>: Borrow<Q>,
    //     Q: Hash + Eq + ?Sized,
    // {
    //     self.map.contains_key(k)
    // }

    pub fn is_real_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn update_epoch(&mut self, epoch: Epoch) {
        assert!(epoch > self.cur_epoch);
        self.cur_epoch = epoch;
    }

    pub fn pop_lru(&mut self) -> Option<K> {
        let node = self.remove_last()?;
        // N.B.: Can't destructure directly because of https://github.com/rust-lang/rust/issues/28536
        let node = *node;
        let IndexedLruEntry {
            key, val, dropped, ..
        } = node;
        unsafe {
            if !dropped {
                let _val = val.assume_init();
            }
            Some(key.assume_init())
        }
    }

    pub fn resize_ghost(&mut self, ghost_cap: usize) {
        // return early if capacity doesn't change
        if ghost_cap == self.ghost_cap {
            return;
        }

        while self.ghost_len > ghost_cap {
            self.pop_lru();
        }
        self.map.shrink_to_fit();

        self.ghost_cap = ghost_cap;
    }

    pub fn clear(&mut self) {
        while self.pop_lru().is_some() {}
    }

    pub fn check_clear(&self) {
        assert_eq!(self.len(), 0);
        assert_eq!(self.ghost_len(), 0);
        assert_eq!(self.current_index_count, 0);
        assert_eq!(self.ghost_current_index_count, 0);
        self.counters.iter().for_each(|(k, v)| {
            assert!(*k <= self.global_index);
            assert_eq!(*v, 0);
        });
        self.ghost_counters.iter().for_each(|(k, v)| {
            assert!(*k <= self.ghost_global_index);
            assert_eq!(*v, 0);
        });
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
            if node_ptr == self.ghost_head {
                self.ghost_head = unsafe { (*self.ghost_head).next };
            }
            if old_node.dropped {
                self.update_ghost_counters(&old_node.index);
            } else {
                self.update_counters(&&old_node.index);
            }
            self.detach(node_ptr);
            Some(old_node)
        } else {
            None
        }
    }
}

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
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

impl<K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator> IndexedLruCache<K, V, S, A> {
    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.map.len() - self.ghost_len
    }

    pub fn ghost_cap(&self) -> usize {
        self.ghost_cap
    }

    pub fn ghost_len(&self) -> usize {
        self.ghost_len
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
            .field("ghost_len", &self.ghost_len())
            .field("ghost_cap", &self.ghost_cap())
            .field("global_index", &self.global_index)
            .field("current_index_count", &self.current_index_count)
            .field("update_interval", &self.update_interval)
            .field("counters", &self.counters)
            .field("ghost_global_index", &self.ghost_global_index)
            .field("ghost_current_index_count", &self.ghost_current_index_count)
            .field("ghost_update_interval", &self.ghost_update_interval)
            .field("ghost_counters", &self.ghost_counters)
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
