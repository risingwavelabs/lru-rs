#[cfg(test)]
mod indexed_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::IndexedLruCache;

    #[test]
    fn test_evict_by_epoch_peek_mut() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        struct DropCounter {
            string: String,
        }
        impl DropCounter {
            pub fn new(string: &str) -> Self {
                Self {
                    string: String::from(string),
                }
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut cache = IndexedLruCache::new(6, 2, 1);

        cache.put(1, DropCounter::new("a"));
        cache.put(2, DropCounter::new("b"));

        cache.update_epoch(1);
        println!("WKXLOG: cache: 1:{:?}", cache);

        cache.put(3, DropCounter::new("c"));
        cache.put(4, DropCounter::new("d"));

        cache.evict_by_epoch(1);
        println!("WKXLOG: cache: 2:{:?}", cache);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.peek_mut(&1);
        assert!(val.is_none());

        let val = cache.peek_mut(&2);
        assert!(val.is_none());

        let val = cache.peek_mut(&3);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("c"));

        let val = cache.peek_mut(&4);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("d"));

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ghost_len(), 2);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);

        println!("WKXLOG: cache: 2.4:{:?}", cache);
        cache.evict_by_epoch(2);
        println!("WKXLOG: cache: 3:{:?}", cache);

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.peek_mut(&1);
        assert!(val.is_none());

        let val = cache.peek_mut(&2);
        assert!(val.is_none());

        let val = cache.peek_mut(&3);
        assert!(val.is_none());

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 4);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.ghost_len(), 2);
        println!("WKXLOG: cache: 4:{:?}", cache);
        cache.clear();
        assert_eq!(cache.ghost_len(), 0);
        println!("WKXLOG: cache: 5:{:?}", cache);
    }

    #[test]
    fn test_evict_by_epoch_get_mut() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        struct DropCounter {
            string: String,
        }
        impl DropCounter {
            pub fn new(string: &str) -> Self {
                Self {
                    string: String::from(string),
                }
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut cache = IndexedLruCache::new(6, 2, 1);

        cache.put(1, DropCounter::new("a"));
        cache.put(2, DropCounter::new("b"));

        cache.update_epoch(1);
        println!("WKXLOG: cache: 1:{:?}", cache);

        cache.put(3, DropCounter::new("c"));
        cache.put(4, DropCounter::new("d"));

        cache.evict_by_epoch(1);
        println!("WKXLOG: cache: 2:{:?}", cache);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.get_mut(&1, false);
        assert!(val.is_none());

        let val = cache.get_mut(&2, false);
        assert!(val.is_none());

        let val = cache.get_mut(&3, false);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("c"));

        let val = cache.get_mut(&4, false);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("d"));

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ghost_len(), 2);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);

        cache.evict_by_epoch(2);
        println!("WKXLOG: cache: 3:{:?}", cache);

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.get_mut(&1, false);
        assert!(val.is_none());

        let val = cache.get_mut(&2, false);
        assert!(val.is_none());

        let val = cache.get_mut(&3, false);
        assert!(val.is_none());

        let val = cache.get_mut(&4, false);
        assert!(val.is_none());
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 4);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.ghost_len(), 2);

        cache.clear();
        assert_eq!(cache.ghost_len(), 0);
        println!("WKXLOG: cache: 5:{:?}", cache);
        cache.check_clear();
    }

    #[test]
    fn test_evict_by_bound_peek_mut() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        struct DropCounter {
            string: String,
        }
        impl DropCounter {
            pub fn new(string: &str) -> Self {
                Self {
                    string: String::from(string),
                }
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut cache = IndexedLruCache::new(3, 2, 1);

        cache.put(1, DropCounter::new("a"));
        cache.put(2, DropCounter::new("b"));
        cache.put(3, DropCounter::new("c"));
        cache.put(4, DropCounter::new("d"));
        cache.put(5, DropCounter::new("e"));
        cache.put(6, DropCounter::new("f"));

        println!("WKXLOG: cache: 2:{:?}", cache);

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.peek_mut(&1);
        assert!(val.is_none());

        let val = cache.peek_mut(&2);
        assert!(val.is_none());

        let val = cache.peek_mut(&3);
        assert!(val.is_none());
        assert_eq!(cache.global_index, 5);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.peek_mut(&4);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("d"));
        assert_eq!(cache.global_index, 5);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.peek_mut(&5);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("e"));
        assert_eq!(cache.global_index, 5);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.peek_mut(&6);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("f"));
        assert_eq!(cache.global_index, 5);
        assert_eq!(cache.ghost_global_index, 2);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);

        println!("WKXLOG: cache: 3:{:?}", cache);

        assert_eq!(cache.ghost_len(), 2);
        cache.clear();
        assert_eq!(cache.ghost_len(), 0);
        assert_eq!(cache.len(), 0);
        println!("WKXLOG: cache: 5:{:?}", cache);
        cache.check_clear();
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn test_evict_by_bound_get_mut() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
        struct DropCounter {
            string: String,
        }
        impl DropCounter {
            pub fn new(string: &str) -> Self {
                Self {
                    string: String::from(string),
                }
            }
        }
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut cache = IndexedLruCache::new(3, 2, 1);

        cache.put(1, DropCounter::new("a"));
        cache.put(2, DropCounter::new("b"));
        cache.put(3, DropCounter::new("c"));
        cache.put(4, DropCounter::new("d"));
        cache.put(5, DropCounter::new("e"));
        cache.put(6, DropCounter::new("f"));

        println!("WKXLOG: cache: 2:{:?}", cache);

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.ghost_len(), 2);
        let val = cache.get_mut(&1, false);
        assert!(val.is_none());

        let val = cache.get_mut(&2, false);
        assert!(val.is_none());

        let val = cache.get_mut(&3, false);
        assert!(val.is_none());
        assert_eq!(cache.global_index, 5);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.get_mut(&4, false);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("d"));
        assert_eq!(cache.global_index, 6);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.get_mut(&5, false);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("e"));
        assert_eq!(cache.global_index, 7);
        assert_eq!(cache.ghost_global_index, 2);

        let val = cache.get_mut(&6, false);
        assert!(val.is_some());
        assert_eq!(val.unwrap().string, String::from("f"));
        assert_eq!(cache.global_index, 8);
        assert_eq!(cache.ghost_global_index, 2);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);

        println!("WKXLOG: cache: 3:{:?}", cache);

        cache.clear();
        assert_eq!(cache.ghost_len(), 0);
        assert_eq!(cache.len(), 0);
        println!("WKXLOG: cache: 5:{:?}", cache);
        cache.check_clear();
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 6);
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
            let mut cache = IndexedLruCache::unbounded(2, 1);
            for i in 1..n + 1 {
                cache.update_epoch(i as u64);
                cache.put(i, DropCounter {});
            }
            cache.evict_by_epoch(51);
            assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 50);
            assert_eq!(cache.len(), 50);
            assert_eq!(cache.ghost_len(), 2);
            cache.clear();
            cache.check_clear();
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
            let mut cache = IndexedLruCache::unbounded(2, 1);
            for i in 0..n {
                cache.put(i, DropCounter {});
            }
            cache.clear();
            cache.check_clear();
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), n * n);
    }
}
