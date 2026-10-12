use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AffinityType {
    SessionHit, // Same session mapped back to its preferred slot
    PrefixHit,  // New or alternate session re-using an idle slot with matching warm prefix
    NewSlot,    // Fresh slot allocation
}

impl AffinityType {
    pub fn is_hit(&self) -> bool {
        matches!(self, AffinityType::SessionHit | AffinityType::PrefixHit)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AffinityType::SessionHit => "HIT",
            AffinityType::PrefixHit => "PREFIX_HIT",
            AffinityType::NewSlot => "NEW",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub slot_id: usize,
    pub prefix_hash: u64,
    pub last_active: Instant,
    pub turns_count: u64,
}

#[derive(Debug)]
pub struct SessionAffinityTracker {
    enabled: bool,
    max_slots: usize,
    sessions: RwLock<HashMap<String, SessionEntry>>,
    slot_busy: Vec<AtomicBool>,
    slot_prefix: RwLock<Vec<Option<u64>>>,
    slot_last_active: RwLock<Vec<Instant>>,
}

/// Hashes the first system prompt or initial turn content to identify warm KV cache prefixes.
pub fn compute_prefix_hash(messages: &[serde_json::Value]) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Some(first) = messages.first() {
        if let Some(role) = first.get("role").and_then(|r| r.as_str()) {
            role.hash(&mut hasher);
        }
        if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
            content[..content.len().min(8192)].hash(&mut hasher);
        }
    }
    hasher.finish()
}

impl SessionAffinityTracker {
    pub fn new(enabled: bool, max_slots: usize) -> Self {
        let mut slot_busy = Vec::with_capacity(max_slots);
        let mut slot_prefix = Vec::with_capacity(max_slots);
        let mut slot_last_active = Vec::with_capacity(max_slots);
        let now = Instant::now();
        for _ in 0..max_slots {
            slot_busy.push(AtomicBool::new(false));
            slot_prefix.push(None);
            slot_last_active.push(now);
        }
        Self {
            enabled,
            max_slots,
            sessions: RwLock::new(HashMap::new()),
            slot_busy,
            slot_prefix: RwLock::new(slot_prefix),
            slot_last_active: RwLock::new(slot_last_active),
        }
    }

    /// Backwards compatible method
    pub fn try_acquire_slot(&self, session_id: &str) -> Option<(usize, bool)> {
        self.try_acquire_slot_with_prefix(session_id, 0).map(|(s, aff)| (s, aff.is_hit()))
    }

    /// Attempts to acquire an execution slot for a session (with eviction allowed).
    pub fn try_acquire_slot_with_prefix(&self, session_id: &str, prefix_hash: u64) -> Option<(usize, AffinityType)> {
        self.try_acquire_slot_opt(session_id, prefix_hash, true)
    }

    /// Attempts to acquire an execution slot for a session with optional eviction.
    /// If session has a preferred slot that is idle, acquires it (SessionHit).
    /// If session is new or preferred slot is busy, looks for an idle slot with matching prefix_hash (PrefixHit).
    /// Otherwise acquires an unassigned idle slot.
    /// If allow_eviction is true, can evict an idle slot assigned to an inactive session (LRU).
    pub fn try_acquire_slot_opt(&self, session_id: &str, prefix_hash: u64, allow_eviction: bool) -> Option<(usize, AffinityType)> {
        if !self.enabled || self.max_slots == 0 {
            return Some((0, AffinityType::NewSlot));
        }

        let now = Instant::now();

        // 1. Check if session already has an assigned slot
        let existing_slot = {
            let read = self.sessions.read().ok()?;
            read.get(session_id).map(|e| e.slot_id)
        };

        if let Some(preferred) = existing_slot {
            if preferred < self.max_slots {
                // Try to acquire the preferred slot
                if !self.slot_busy[preferred].swap(true, Ordering::SeqCst) {
                    if let Ok(mut write) = self.sessions.write() {
                        if let Some(entry) = write.get_mut(session_id) {
                            entry.last_active = now;
                            entry.turns_count += 1;
                            if prefix_hash != 0 {
                                entry.prefix_hash = prefix_hash;
                            }
                        }
                    }
                    if let Ok(mut p_write) = self.slot_prefix.write() {
                        if prefix_hash != 0 && preferred < p_write.len() {
                            p_write[preferred] = Some(prefix_hash);
                        }
                    }
                    if let Ok(mut la_write) = self.slot_last_active.write() {
                        if preferred < la_write.len() {
                            la_write[preferred] = now;
                        }
                    }
                    return Some((preferred, AffinityType::SessionHit));
                }
            }
        }

        // 2. Preferred slot is busy or session is new:
        // Check for an IDLE slot that already has the MATCHING prefix_hash (Warm Prefix Reuse).
        if prefix_hash != 0 {
            if let Ok(p_read) = self.slot_prefix.read() {
                for slot in 0..self.max_slots {
                    if slot < p_read.len() && p_read[slot] == Some(prefix_hash) {
                        if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                            // Acquired warm prefix slot!
                            if let Ok(mut write) = self.sessions.write() {
                                write.insert(
                                    session_id.to_string(),
                                    SessionEntry {
                                        slot_id: slot,
                                        prefix_hash,
                                        last_active: now,
                                        turns_count: 1,
                                    },
                                );
                            }
                            if let Ok(mut la_write) = self.slot_last_active.write() {
                                if slot < la_write.len() {
                                    la_write[slot] = now;
                                }
                            }
                            return Some((slot, AffinityType::PrefixHit));
                        }
                    }
                }
            }
        }

        // 3. Find an idle slot, prioritizing slots NOT assigned to any session.
        let assigned_slots: HashMap<usize, Instant> = {
            let read = self.sessions.read().ok()?;
            read.values().map(|e| (e.slot_id, e.last_active)).collect()
        };

        for slot in 0..self.max_slots {
            if !assigned_slots.contains_key(&slot) {
                if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                    if let Ok(mut write) = self.sessions.write() {
                        write.insert(
                            session_id.to_string(),
                            SessionEntry {
                                slot_id: slot,
                                prefix_hash,
                                last_active: now,
                                turns_count: 1,
                            },
                        );
                    }
                    if let Ok(mut p_write) = self.slot_prefix.write() {
                        if prefix_hash != 0 && slot < p_write.len() {
                            p_write[slot] = Some(prefix_hash);
                        }
                    }
                    if let Ok(mut la_write) = self.slot_last_active.write() {
                        if slot < la_write.len() {
                            la_write[slot] = now;
                        }
                    }
                    return Some((slot, AffinityType::NewSlot));
                }
            }
        }

        // 4. All idle slots are assigned to other sessions.
        if !allow_eviction {
            return None;
        }

        // Pick an idle slot whose assigned session was active longest ago (LRU eviction).
        let mut candidates: Vec<usize> = (0..self.max_slots).collect();
        candidates.sort_by_key(|slot| assigned_slots.get(slot).copied().unwrap_or(now));

        for slot in candidates {
            if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                if let Ok(mut write) = self.sessions.write() {
                    write.retain(|k, v| k == session_id || v.slot_id != slot);
                    write.insert(
                        session_id.to_string(),
                        SessionEntry {
                            slot_id: slot,
                            prefix_hash,
                            last_active: now,
                            turns_count: 1,
                        },
                    );
                }
                if let Ok(mut p_write) = self.slot_prefix.write() {
                    if prefix_hash != 0 && slot < p_write.len() {
                        p_write[slot] = Some(prefix_hash);
                    }
                }
                if let Ok(mut la_write) = self.slot_last_active.write() {
                    if slot < la_write.len() {
                        la_write[slot] = now;
                    }
                }
                return Some((slot, AffinityType::NewSlot));
            }
        }

        None
    }

    /// Releases a slot back to idle state
    pub fn release_slot(&self, slot_id: usize) {
        if self.enabled && slot_id < self.max_slots {
            self.slot_busy[slot_id].store(false, Ordering::SeqCst);
        }
    }

    pub fn get_affinity(&self, session_id: &str) -> Option<usize> {
        self.sessions.read().ok()?.get(session_id).map(|e| e.slot_id)
    }

    pub fn is_slot_busy(&self, slot_id: usize) -> bool {
        if slot_id < self.max_slots {
            self.slot_busy[slot_id].load(Ordering::SeqCst)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affinity_stickiness_and_slot_reuse() {
        let tracker = SessionAffinityTracker::new(true, 3);

        // Turn 1 for session A
        let (slot_a1, is_hit1) = tracker.try_acquire_slot("agent-alpha").unwrap();
        assert_eq!(slot_a1, 0);
        assert!(!is_hit1, "First turn should be a new allocation");

        // Release slot after turn 1 completes
        tracker.release_slot(slot_a1);

        // Turn 2 for session A -> must get slot 0 again with affinity hit!
        let (slot_a2, is_hit2) = tracker.try_acquire_slot("agent-alpha").unwrap();
        assert_eq!(slot_a2, slot_a1);
        assert!(is_hit2, "Second turn for same session must be an affinity HIT");
        tracker.release_slot(slot_a2);

        // Session B should get unassigned slot 1
        let (slot_b, is_hit_b) = tracker.try_acquire_slot("agent-beta").unwrap();
        assert_eq!(slot_b, 1);
        assert!(!is_hit_b);
        tracker.release_slot(slot_b);
    }

    #[test]
    fn test_busy_slot_fallback_to_idle_slot() {
        let tracker = SessionAffinityTracker::new(true, 3);

        // Session A acquires slot 0
        let (slot_a, _) = tracker.try_acquire_slot("agent-alpha").unwrap();
        assert_eq!(slot_a, 0);

        // While slot 0 is busy, another request from session A arrives -> falls back to idle slot 1
        let (slot_a_fallback, is_hit) = tracker.try_acquire_slot("agent-alpha").unwrap();
        assert_eq!(slot_a_fallback, 1);
        assert!(!is_hit, "Busy slot fallback should not be counted as affinity hit");

        tracker.release_slot(slot_a);
        tracker.release_slot(slot_a_fallback);
    }

    #[test]
    fn test_prefix_affinity_warm_cache_reuse_for_subagents() {
        let tracker = SessionAffinityTracker::new(true, 3);
        let common_prompt_prefix_hash: u64 = 0xdeadbeefcafe;

        // Root Agent acquires slot 0 with prefix hash
        let (slot_root, aff_root) = tracker.try_acquire_slot_with_prefix("root-agent", common_prompt_prefix_hash).unwrap();
        assert_eq!(slot_root, 0);
        assert_eq!(aff_root, AffinityType::NewSlot);

        // Root agent finishes turn 1 and waits for subagents (releases slot 0)
        tracker.release_slot(slot_root);

        // Subagent 1 arrives (new session id) with the SAME prompt prefix hash
        let (slot_sub1, aff_sub1) = tracker.try_acquire_slot_with_prefix("subagent-discord", common_prompt_prefix_hash).unwrap();
        // It must get Slot 0 because Slot 0 already has the warm prefix in KV cache!
        assert_eq!(slot_sub1, 0);
        assert_eq!(aff_sub1, AffinityType::PrefixHit);

        // While Slot 0 is busy with Subagent 1, Subagent 2 arrives with the same prefix hash
        let (slot_sub2, aff_sub2) = tracker.try_acquire_slot_with_prefix("subagent-concurrency", common_prompt_prefix_hash).unwrap();
        // Slot 0 was busy, so Subagent 2 acquires clean Slot 1
        assert_eq!(slot_sub2, 1);
        assert_eq!(aff_sub2, AffinityType::NewSlot);

        tracker.release_slot(slot_sub1);
        tracker.release_slot(slot_sub2);
    }

    #[test]
    fn test_anti_eviction_hysteresis() {
        let tracker = SessionAffinityTracker::new(true, 2);

        // Session 1 acquires and occupies slot 0
        let (s1, _) = tracker.try_acquire_slot_opt("session-1", 100, true).unwrap();
        assert_eq!(s1, 0);

        // Session 2 acquires and occupies slot 1
        let (s2, _) = tracker.try_acquire_slot_opt("session-2", 200, true).unwrap();
        assert_eq!(s2, 1);

        // Session 1 and 2 finish turn and release their slots (both slots are idle, but assigned to session-1 and session-2)
        tracker.release_slot(s1);
        tracker.release_slot(s2);

        // Session 3 arrives with prefix 300, with allow_eviction = false.
        // Both slot 0 and slot 1 are assigned to other sessions and have different prefixes.
        // With allow_eviction = false, it must NOT evict either session!
        let no_evict = tracker.try_acquire_slot_opt("session-3", 300, false);
        assert!(no_evict.is_none(), "Should not evict assigned slots when allow_eviction is false");

        // When allow_eviction = true (e.g. after queue timeout threshold), it can evict LRU slot
        let evict = tracker.try_acquire_slot_opt("session-3", 300, true);
        assert!(evict.is_some(), "Should evict assigned slot when allow_eviction is true");
    }
}
