use std::{
    alloc::Allocator,
    borrow::Borrow,
    hash::{BuildHasher, Hash},
    marker::PhantomData,
};

use crate::LruCache;

/// Phantom lifetime type that can only be subtyped by exactly the same lifetime `'brand`.
/// This is a trick in rust to create a unique identity.
///
/// See <https://doc.rust-lang.org/nomicon/subtyping.html> for details.
type InvariantLifetime<'brand> = PhantomData<fn(&'brand ()) -> &'brand ()>;

/// `'brand` is the identity of scope.
///
/// See [`LruCache::scope_for_multi_get`].
pub struct GetScope<'cache, 'brand, K, V, S, A: Clone + Allocator> {
    cache: &'cache mut LruCache<K, V, S, A>,
    _lifetime: InvariantLifetime<'brand>,
}

pub struct GetToken<'brand> {
    _lifetime: InvariantLifetime<'brand>,
}

impl<K, V, S, A: Allocator + Clone> LruCache<K, V, S, A> {
    /// Create a scope, and inside the scope, you can hold multiple immutable references at the same time.
    pub fn scope_for_multi_get<'cache, F, R>(&'cache mut self, func: F) -> R
    where
        for<'brand> F: FnOnce(GetScope<'cache, 'brand, K, V, S, A>, GetToken<'brand>) -> R,
    {
        let scope = GetScope {
            cache: self,
            _lifetime: Default::default(),
        };
        let token = GetToken {
            _lifetime: Default::default(),
        };
        func(scope, token)
    }
}

impl<'cache, 'brand, K: Hash + Eq, V, S: BuildHasher, A: Clone + Allocator>
    GetScope<'cache, 'brand, K, V, S, A>
{
    pub fn get<'scope, 'token, Q>(
        &'scope mut self,
        _token: &'token GetToken<'brand>,
        k: &Q,
    ) -> Option<&'token V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        // SAFETY:
        //
        // Here, we transmute the lifetime of the value reference from `'cache` to `'token`. The safety is based on the several facts below:
        // 1. Both `scope` and `token` are only valid inside the same unique (guarded by `'brand`) scope.
        // 2. Inside the scope, according to the implementaiton of `LruCache::get`, it never invalidate any value references.
        // 3. Outside of the scope, since the `scope` exclusively borrowed inner `cache`, nothing will invalidate any value references.
        unsafe { std::mem::transmute(self.cache.get(k)) }
    }
}

#[cfg(test)]
mod tests {
    use crate::LruCache;

    #[test]
    fn test_scope_for_multi_get() {
        let mut cache = LruCache::unbounded();
        assert_eq!(cache.put("apple", "red".to_string()), None);
        assert_eq!(cache.put("banana", "yellow".to_string()), None);
        let joined = cache.scope_for_multi_get(|mut scope, token| {
            let apple: &str = scope.get(&token, "apple").unwrap().as_str();
            let banana: &str = scope.get(&token, "banana").unwrap().as_str();
            [apple, banana].join(" ")
        });

        assert_eq!(joined, "red yellow");
    }
}
