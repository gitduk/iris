use chrono::Utc;
use uuid::Uuid;

use crate::types::ContextEntry;

/// In-process working memory: ring buffer with capacity limit and eviction.
#[derive(Debug)]
pub struct WorkingMemory {
    entries: Vec<ContextEntry>,
    capacity: usize,
    ttl_secs: f64,
}

impl WorkingMemory {
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            ttl_secs: ttl_secs as f64,
        }
    }

    /// Insert a new entry. Evicts lowest-value unpinned entry if at capacity.
    pub fn insert(&mut self, mut entry: ContextEntry) {
        entry.last_accessed = Utc::now();
        if self.entries.len() >= self.capacity {
            self.evict_one();
        }
        self.entries.push(entry);
    }

    /// Touch an entry (update last_accessed). Returns false if not found.
    pub fn touch(&mut self, id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.last_accessed = Utc::now();
            true
        } else {
            false
        }
    }

    /// Pin an entry so it won't be evicted.
    pub fn pin(&mut self, id: Uuid, reason: impl Into<String>) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.pinned_by = Some(reason.into());
            true
        } else {
            false
        }
    }

    /// Unpin an entry, making it eligible for eviction again.
    pub fn unpin(&mut self, id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.pinned_by = None;
            true
        } else {
            false
        }
    }

    /// Get an entry by ID (also touches it).
    pub fn get(&mut self, id: Uuid) -> Option<&ContextEntry> {
        self.touch(id);
        self.entries.iter().find(|e| e.id == id)
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return recent entries sorted by creation time (oldest first), up to `limit`.
    pub fn recent(&self, limit: usize) -> Vec<&ContextEntry> {
        let mut refs: Vec<&ContextEntry> = self.entries.iter().collect();
        refs.sort_by_key(|e| e.created_at);
        if refs.len() > limit {
            refs.split_off(refs.len() - limit)
        } else {
            refs
        }
    }

    /// Number of distinct active topics.
    pub fn active_topics(&self) -> usize {
        let mut topics: Vec<Uuid> = self.entries.iter()
            .filter_map(|e| e.topic_id)
            .collect();
        topics.sort();
        topics.dedup();
        topics.len()
    }

    /// Evict the unpinned entry with the highest evict score.
    fn evict_one(&mut self) {
        let now = Utc::now();
        let victim = self.entries.iter()
            .enumerate()
            .filter(|(_, e)| e.pinned_by.is_none())
            .max_by(|(_, a), (_, b)| {
                a.evict_score(now, self.ttl_secs)
                    .partial_cmp(&b.evict_score(now, self.ttl_secs))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        if let Some(idx) = victim {
            self.entries.swap_remove(idx);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_entry(salience: f32) -> ContextEntry {
        ContextEntry {
            id: Uuid::new_v4(),
            topic_id: None,
            content: "test".into(),
            salience_score: salience,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            pinned_by: None,
            is_response: false,
        }
    }

    #[test]
    fn insert_and_len() {
        let mut wm = WorkingMemory::new(4, 1800);
        wm.insert(make_entry(0.5));
        wm.insert(make_entry(0.6));
        assert_eq!(wm.len(), 2);
    }

    #[test]
    fn evicts_at_capacity() {
        let mut wm = WorkingMemory::new(2, 1800);
        let e1 = make_entry(0.1);
        let e2 = make_entry(0.9);
        wm.insert(e1);
        wm.insert(e2);
        // At capacity, inserting a third should evict one
        wm.insert(make_entry(0.5));
        assert_eq!(wm.len(), 2);
    }

    #[test]
    fn pinned_entry_survives_eviction() {
        let mut wm = WorkingMemory::new(2, 1800);
        let e1 = make_entry(0.1); // low salience, would normally be evicted
        let id1 = e1.id;
        wm.insert(e1);
        wm.pin(id1, "important");
        wm.insert(make_entry(0.9));
        wm.insert(make_entry(0.5)); // triggers eviction
        // Pinned entry should survive
        assert!(wm.get(id1).is_some());
    }

    #[test]
    fn touch_updates_access() {
        let mut wm = WorkingMemory::new(4, 1800);
        let e = make_entry(0.5);
        let id = e.id;
        wm.insert(e);
        assert!(wm.touch(id));
        assert!(!wm.touch(Uuid::new_v4())); // nonexistent
    }

    #[test]
    fn active_topics_count() {
        let mut wm = WorkingMemory::new(8, 1800);
        let topic_a = Uuid::new_v4();
        let topic_b = Uuid::new_v4();
        let mut e1 = make_entry(0.5);
        e1.topic_id = Some(topic_a);
        let mut e2 = make_entry(0.5);
        e2.topic_id = Some(topic_a); // same topic
        let mut e3 = make_entry(0.5);
        e3.topic_id = Some(topic_b);
        wm.insert(e1);
        wm.insert(e2);
        wm.insert(e3);
        assert_eq!(wm.active_topics(), 2);
    }
}

