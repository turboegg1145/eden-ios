//! A byte-bounded LRU cache.
//!
//! iOS <= 14.4 enforces a hard Jetsam limit, so every decoded resource cache in
//! eden-rs is bounded by *bytes*, not by entry count. The default used by the
//! renderer and the VFS is 256 MiB.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

struct Entry<V> {
    value: V,
    tick: u64,
    weight: usize,
}

/// LRU cache with a byte budget and an explicit per-value weigher.
pub struct WeightedLru<K, V> {
    capacity: usize,
    used: usize,
    tick: u64,
    map: HashMap<K, Entry<V>>,
    order: BTreeMap<(u64, K), ()>,
}

impl<K, V> WeightedLru<K, V>
where
    K: Clone + Eq + Hash + Ord,
{
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity: capacity_bytes,
            used: 0,
            tick: 0,
            map: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.map.get_mut(key)?;
        let old = entry.tick;
        entry.tick = tick;
        self.order.remove(&(old, key.clone()));
        self.order.insert((tick, key.clone()), ());
        Some(&entry.value)
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|e| &e.value)
    }

    /// Insert a value. Values larger than the whole cache are rejected.
    pub fn insert(&mut self, key: K, value: V, weight: usize) -> bool {
        if weight > self.capacity {
            return false;
        }
        if let Some(old) = self.map.remove(&key) {
            self.used = self.used.saturating_sub(old.weight);
            self.order.remove(&(old.tick, key.clone()));
        }
        while self.used + weight > self.capacity {
            let Some(((_, oldest), ())) = self.order.pop_first() else {
                break;
            };
            if let Some(old) = self.map.remove(&oldest) {
                self.used = self.used.saturating_sub(old.weight);
            }
        }
        self.tick += 1;
        self.used += weight;
        self.order.insert((self.tick, key.clone()), ());
        self.map.insert(
            key,
            Entry {
                value,
                tick: self.tick,
                weight,
            },
        );
        true
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.used = 0;
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let old = self.map.remove(key)?;
        self.order.remove(&(old.tick, key.clone()));
        self.used = self.used.saturating_sub(old.weight);
        Some(old.value)
    }
}

/// Byte-bounded LRU for raw file bytes / decoded script text.
pub type ByteLru = WeightedLru<String, Vec<u8>>;

impl ByteLru {
    pub fn insert_bytes(&mut self, key: String, bytes: Vec<u8>) -> bool {
        let len = bytes.len();
        self.insert(key, bytes, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let mut c = ByteLru::new(10);
        assert!(c.insert_bytes("a".into(), vec![1; 4]));
        assert!(c.insert_bytes("b".into(), vec![2; 4]));
        assert!(c.get(&"a".to_string()).is_some());
        assert!(c.insert_bytes("c".into(), vec![3; 4]));
        // b was the LRU entry and must be evicted.
        assert!(c.get(&"b".to_string()).is_none());
        assert!(c.get(&"a".to_string()).is_some());
        assert!(c.get(&"c".to_string()).is_some());
        assert_eq!(c.used(), 8);
    }

    #[test]
    fn rejects_oversize_value() {
        let mut c = ByteLru::new(4);
        assert!(!c.insert_bytes("big".into(), vec![0; 5]));
        assert!(c.is_empty());
    }

    #[test]
    fn replacing_updates_usage() {
        let mut c = ByteLru::new(20);
        c.insert_bytes("a".into(), vec![0; 8]);
        c.insert_bytes("a".into(), vec![0; 4]);
        assert_eq!(c.used(), 4);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn remove_updates_usage() {
        let mut c = ByteLru::new(20);
        c.insert_bytes("a".into(), vec![0; 8]);
        assert!(c.remove(&"a".to_string()).is_some());
        assert_eq!(c.used(), 0);
        assert!(c.is_empty());
    }
}
