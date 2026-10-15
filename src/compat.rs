//! Minimal associative map for dual-mode (host/kernel) builds — replaces
//! `std::collections::HashMap` so the whole compiler lib compiles under
//! `#![no_std] + alloc` (SPEC Principle 5: self-containment; P6b Task 2:
//! the compiler runs inside the kernel). Vec-backed: our tables hold tens of
//! entries (functions/globals/literals/live borrows); linear scan beats
//! hashing at this scale and removes SipHash (std-only) from the kernel.
//! Total functions only — no panics.

use alloc::vec::Vec;
use core::borrow::Borrow;

pub struct Map<K, V> {
    entries: Vec<(K, V)>,
}

impl<K: PartialEq, V> Map<K, V> {
    pub const fn new() -> Self {
        Map { entries: Vec::new() }
    }
    /// Lookup by borrowed form (String key with &str query), mirroring HashMap's
    /// `Borrow` ergonomics.
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: PartialEq,
    {
        self.entries
            .iter()
            .find(|(ek, _)| ek.borrow() == k)
            .map(|(_, v)| v)
    }
    pub fn get_mut<Q: ?Sized>(&mut self, k: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: PartialEq,
    {
        self.entries
            .iter_mut()
            .find(|(ek, _)| ek.borrow() == k)
            .map(|(_, v)| v)
    }
    pub fn contains_key<Q: ?Sized>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: PartialEq,
    {
        self.entries.iter().any(|(ek, _)| ek.borrow() == k)
    }
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        match self.entries.iter_mut().find(|(ek, _)| *ek == k) {
            Some(slot) => Some(core::mem::replace(&mut slot.1, v)),
            None => {
                self.entries.push((k, v));
                None
            }
        }
    }
    /// `HashMap::entry(k).or_insert(v)` equivalent. The index is in-bounds by
    /// construction (just found or just pushed).
    pub fn entry_or_insert(&mut self, k: K, v: V) -> &mut V {
        let pos = match self.entries.iter().position(|(ek, _)| *ek == k) {
            Some(p) => p,
            None => {
                self.entries.push((k, v));
                self.entries.len() - 1
            }
        };
        &mut self.entries[pos].1
    }
    pub fn remove<Q: ?Sized>(&mut self, k: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: PartialEq,
    {
        match self.entries.iter().position(|(ek, _)| ek.borrow() == k) {
            Some(i) => Some(self.entries.remove(i).1),
            None => None,
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(k, _)| k)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use alloc::string::String;

    #[test]
    fn map_semantics_match_our_hashmap_usage() {
        let mut m: Map<String, u32> = Map::new();
        assert!(m.is_empty());
        assert!(m.insert("a".into(), 1).is_none());
        assert_eq!(m.insert("a".into(), 2), Some(1)); // replace returns old
        assert_eq!(m.get("a"), Some(&2));
        assert!(m.contains_key("a"));
        *m.entry_or_insert("b".into(), 10) += 5;
        assert_eq!(m.get("b"), Some(&15));
        *m.entry_or_insert("b".into(), 999) += 1; // existing key keeps value
        assert_eq!(m.get("b"), Some(&16));
        assert_eq!(m.remove("a"), Some(2));
        assert_eq!(m.remove("a"), None);
        assert_eq!(m.len(), 1);
        m.clear();
        assert!(m.get("b").is_none());
    }
}
