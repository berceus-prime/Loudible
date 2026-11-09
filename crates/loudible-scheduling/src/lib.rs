//! Pure keyed admission policy, ported from `LoudibleScheduling`. No dependencies.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<Id, Key> {
    pub id: Id,
    pub key: Key,
}

/// Pure admission policy. The owner runs work and releases reservations after cleanup.
#[derive(Clone, Debug)]
pub struct KeyedScheduler<Id, Key> {
    maximum_concurrent: usize,
    pending: Vec<Entry<Id, Key>>,
    active: HashMap<Id, Key>,
    paused: bool,
}

impl<Id: Clone + Eq + Hash, Key: Clone + Eq + Hash> KeyedScheduler<Id, Key> {
    pub fn new(maximum_concurrent: usize) -> Self {
        Self {
            maximum_concurrent: maximum_concurrent.max(1),
            pending: Vec::new(),
            active: HashMap::new(),
            paused: false,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.active.is_empty()
    }

    /// False when `id` is already pending or active.
    pub fn enqueue(&mut self, id: Id, key: Key) -> bool {
        if self.active.contains_key(&id) || self.pending.iter().any(|entry| entry.id == id) {
            return false;
        }
        self.pending.push(Entry { id, key });
        true
    }

    pub fn reserve_ready(&mut self) -> Vec<Entry<Id, Key>> {
        if self.paused {
            return Vec::new();
        }
        let mut keys: HashSet<Key> = self.active.values().cloned().collect();
        let mut ready = Vec::new();
        let mut waiting = Vec::new();
        for entry in std::mem::take(&mut self.pending) {
            if self.active.len() < self.maximum_concurrent && keys.insert(entry.key.clone()) {
                self.active.insert(entry.id.clone(), entry.key.clone());
                ready.push(entry);
            } else {
                waiting.push(entry);
            }
        }
        self.pending = waiting;
        ready
    }

    pub fn remove_pending(&mut self, id: &Id) -> bool {
        match self.pending.iter().position(|entry| entry.id == *id) {
            Some(index) => {
                self.pending.remove(index);
                true
            }
            None => false,
        }
    }

    pub fn remove_all_pending(&mut self) -> Vec<Entry<Id, Key>> {
        std::mem::take(&mut self.pending)
    }

    pub fn finish(&mut self, id: &Id) {
        self.active.remove(id);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<K>(entries: Vec<Entry<i32, K>>) -> Vec<i32> {
        entries.into_iter().map(|entry| entry.id).collect()
    }

    #[test]
    fn keys_serialize_without_blocking_other_keys() {
        let mut queue = KeyedScheduler::new(3);
        queue.enqueue(1, "a");
        queue.enqueue(2, "a");
        queue.enqueue(3, "b");
        queue.enqueue(4, "c");
        assert_eq!(ids(queue.reserve_ready()), [1, 3, 4]);
        assert!(queue.reserve_ready().is_empty());
        queue.finish(&1);
        assert_eq!(ids(queue.reserve_ready()), [2]);
    }

    #[test]
    fn capacity_is_bounded() {
        for (limit, expected) in [(0, 1), (1, 1), (3, 3)] {
            let mut queue = KeyedScheduler::new(limit);
            for id in 0..5 {
                queue.enqueue(id, id);
            }
            assert_eq!(
                ids(queue.reserve_ready()),
                (0..expected).collect::<Vec<_>>()
            );
            assert!(queue.reserve_ready().is_empty());
        }
    }

    #[test]
    fn removal_does_not_release_active_work() {
        let mut queue = KeyedScheduler::new(1);
        assert!(queue.is_idle());
        assert!(queue.enqueue(1, 0));
        assert!(!queue.enqueue(1, 1));
        assert_eq!(ids(queue.reserve_ready()), [1]);
        assert!(!queue.enqueue(1, 1));
        queue.enqueue(2, 0);
        queue.enqueue(3, 1);
        assert!(!queue.remove_pending(&1));
        assert!(!queue.remove_pending(&99));
        assert!(queue.remove_pending(&2));
        assert_eq!(ids(queue.remove_all_pending()), [3]);
        assert!(!queue.is_idle());
        queue.finish(&99);
        assert!(!queue.is_idle());
        queue.finish(&1);
        assert!(queue.is_idle());
        queue.enqueue(4, 0);
        assert_eq!(ids(queue.reserve_ready()), [4]);
    }

    #[test]
    fn pause_keeps_reservations_and_pending_order() {
        let mut queue = KeyedScheduler::new(2);
        queue.enqueue(1, 0);
        assert_eq!(ids(queue.reserve_ready()), [1]);
        queue.set_paused(true);
        queue.enqueue(2, 0);
        queue.enqueue(3, 1);
        assert!(queue.reserve_ready().is_empty());
        assert!(!queue.is_idle());
        assert!(queue.remove_pending(&3));
        queue.enqueue(4, 1);
        queue.set_paused(false);
        assert_eq!(ids(queue.reserve_ready()), [4]);
        queue.finish(&1);
        assert_eq!(ids(queue.reserve_ready()), [2]);
    }
}
