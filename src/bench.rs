use rand::distributions::Alphanumeric;
use rand::rngs::SmallRng;
use rand::Rng;

#[allow(dead_code)]
fn deterministic_random_string(rng: &mut SmallRng) -> String {
    rng.sample_iter(&Alphanumeric)
        .take(1024)
        .map(char::from)
        .collect()
    // String::with_capacity(1024)
}
#[allow(dead_code)]
fn is_sampled(rng: &mut SmallRng) -> bool {
    rng.gen_range(0..100) < 2
}

#[cfg(test)]
mod bench_tests {
    pub type DefaultHasher = hashbrown::hash_map::DefaultHashBuilder;

    use crate::bench::is_sampled;
    use crate::IndexedLruCache;
    use rand::rngs::SmallRng;
    use rand::{Rng, SeedableRng};
    use std::hash::{Hash, Hasher};
    use std::mem;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use super::deterministic_random_string;

    static K_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static V_DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct KCounter {
        num: u32,
        _num_t: u32,
    }
    impl KCounter {
        pub fn new(num: u32) -> Self {
            Self {
                num,
                _num_t: num + 1,
            }
        }
    }
    impl Drop for KCounter {
        fn drop(&mut self) {
            K_DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Hash for KCounter {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.num.hash(state);
        }
    }

    impl PartialEq for KCounter {
        fn eq(&self, other: &Self) -> bool {
            self.num == other.num
        }
    }

    impl Eq for KCounter {}

    struct VCounter {
        string: String,
    }
    impl VCounter {
        pub fn new(string: &str) -> Self {
            Self {
                string: String::from(string),
            }
        }
    }
    impl Drop for VCounter {
        fn drop(&mut self) {
            V_DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    const TEST_COUNT: usize = 1000 * 1000 * 100 * 1;
    const REAL_CAP_LIMIT: usize = 200 * 1000;
    const DELAY_KEY_RANGE: u32 = 200 * 1000 * 5;
    const NORMAL_KEY_RANGE: u32 = 5 * 1000 * 1;
    const GHOST_CAP: usize = 200 * 1000;
    const UPDATE_INTERVAL: u32 = REAL_CAP_LIMIT as u32 / 30;
    const GHOST_BUCKET_COUNT: usize = 30;

    #[test]
    #[ignore]
    fn test_indexed() {
        let rng = &mut SmallRng::seed_from_u64(0);

        let hasher = DefaultHasher::new();
        let mut cache: IndexedLruCache<KCounter, VCounter> = IndexedLruCache::unbounded_with_hasher(
            hasher,
            GHOST_CAP,
            UPDATE_INTERVAL,
            GHOST_BUCKET_COUNT,
        );

        let value_string = deterministic_random_string(rng);

        let mut contain_num = 0;
        let mut epoch = 101;
        let mut cur_size = 0;
        let mut k_start = DELAY_KEY_RANGE + NORMAL_KEY_RANGE;
        cache.update_epoch(epoch);

        let start_time = Instant::now();

        for i in 0..TEST_COUNT {
            let mut k_num = rng.gen_range(k_start - NORMAL_KEY_RANGE..k_start);
            if is_sampled(rng) {
                k_num -= rng.gen_range(0..DELAY_KEY_RANGE);
            }
            let key: KCounter = KCounter::new(k_num);
            let value = VCounter::new(&value_string.clone());
            if cache.contains(&key, false) {
                contain_num += 1;
            }
            cur_size += mem::size_of::<KCounter>();
            cur_size += value.string.len();
            let (old_v, sample_data) = cache.put_sample(key, value, false, false);
            if sample_data.is_some() {
                cur_size -= mem::size_of::<KCounter>();
            }
            if let Some(ov) = old_v {
                cur_size -= ov.string.len();
            }

            if (i + 1) % (TEST_COUNT / 20) == 0 {
                println!("i:{} now", i + 1);
                println!("  Cur size: {:?}", cur_size);
                println!("  Cur len: {:?}", cache.len());
                println!("  Cur ghost len: {:?}", cache.ghost_len());
                println!("  Cur real bucket count: {:?}", cache.bucket_count());
                println!("  Cur ghost bucket count: {:?}", cache.ghost_bucket_count());
                println!(
                    "  Hit rate: {:?}%, miss rate: {:?}%",
                    contain_num as f64 / (i + 1) as f64 * 100.0,
                    (i + 1 - contain_num) as f64 / (i + 1) as f64 * 100.0
                );
            }
            if i % 4096 == 0 {
                if cache.len() > REAL_CAP_LIMIT {
                    while let Some((key_op, old_v)) = cache.pop_lru_by_epoch(epoch - 100) {
                        if let Some(_k) = key_op {
                            cur_size -= mem::size_of::<KCounter>();
                        }
                        cur_size -= old_v.string.len();
                    }
                }
                cache.adjust_counters();
            }
            if i % 24576 == 0 {
                epoch += 1;
                cache.update_epoch(epoch);
            }
            if i % 16 == 0 {
                k_start += 1;
            }
        }

        let end_time = Instant::now();
        let elapsed = end_time - start_time;

        println!("Time elapsed: {:?}", elapsed);
        println!("Contain num: {:?}", contain_num);
        println!("Cur size: {:?}", cur_size);
    }

    #[test]
    fn test_raw() {}
}
