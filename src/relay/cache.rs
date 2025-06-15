use dashmap::{mapref::one::Ref, DashMap};

///
/// Uses DashMap internally to enable sharing of cache between threads
/// Values are never deleted for now
pub struct Cache<K, V> {
    inner: DashMap<K, V>,
}

impl<K, V> Cache<K, V>
where
    K: std::cmp::Eq + std::hash::Hash,
    V: Clone,
{
    pub fn new(initial_capacity: usize) -> Self {
        Cache {
            inner: DashMap::with_capacity(initial_capacity),
        }
    }

    pub fn get(&self, key: &K) -> Option<Ref<K, V>> {
        self.inner.get(key)
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self.inner.insert(key, value)
    }
}
