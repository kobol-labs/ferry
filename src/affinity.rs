use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub slot_id: usize,
    pub last_active: Instant,
    pub turns_count: u64,
}

#[derive(Debug)]
pub struct SessionAffinityTracker {
    enabled: bool,
    max_slots: usize,
    sessions: RwLock<HashMap<String, SessionEntry>>,
    slot_busy: Vec<AtomicBool>,
}

impl SessionAffinityTracker {
    pub fn new(enabled: bool, max_slots: usize) -> Self {
        let mut slot_busy = Vec::with_capacity(max_slots);
        for _ in 0..max_slots {
            slot_busy.push(AtomicBool::new(false));
        }
        Self {
            enabled,
            max_slots,
            sessions: RwLock::new(HashMap::new()),
            slot_busy,
        }
    }

    /// Attempts to acquire an execution slot for a session.
    /// Returns Some((slot_id, is_affinity_hit)) if a slot is available.
    pub fn try_acquire_slot(&self, session_id: &str) -> Option<(usize, bool)> {
        if !self.enabled || self.max_slots == 0 {
            return Some((0, false));
        }

        // 1. Check if session already has an assigned slot
        let existing_slot = {
            let read = self.sessions.read().ok()?;
            read.get(session_id).map(|e| e.slot_id)
        };

        if let Some(preferred) = existing_slot {
            if preferred < self.max_slots {
                // Try to acquire the preferred slot
                if !self.slot_busy[preferred].swap(true, Ordering::SeqCst) {
                    // Successfully acquired preferred slot (Affinity HIT!)
                    if let Ok(mut write) = self.sessions.write() {
                        if let Some(entry) = write.get_mut(session_id) {
                            entry.last_active = Instant::now();
                            entry.turns_count += 1;
                        }
                    }
                    return Some((preferred, true));
                }
            }
        }

        // 2. Preferred slot is busy or session is new.
        // Identify which slots are currently assigned to active sessions.
        let assigned_slots: HashMap<usize, Instant> = {
            let read = self.sessions.read().ok()?;
            read.values().map(|e| (e.slot_id, e.last_active)).collect()
        };

        // First pass: try to acquire an idle slot that is NOT currently assigned to any session.
        for slot in 0..self.max_slots {
            if !assigned_slots.contains_key(&slot) {
                if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                    if let Ok(mut write) = self.sessions.write() {
                        write.insert(
                            session_id.to_string(),
                            SessionEntry {
                                slot_id: slot,
                                last_active: Instant::now(),
                                turns_count: 1,
                            },
                        );
                    }
                    return Some((slot, false));
                }
            }
        }

        // Second pass: all idle slots are assigned to other sessions.
        // Pick an idle slot whose assigned session was active longest ago (LRU eviction).
        let mut candidates: Vec<usize> = (0..self.max_slots).collect();
        candidates.sort_by_key(|slot| assigned_slots.get(slot).copied().unwrap_or_else(Instant::now));

        for slot in candidates {
            if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                if let Ok(mut write) = self.sessions.write() {
                    write.retain(|k, v| k == session_id || v.slot_id != slot);
                    write.insert(
                        session_id.to_string(),
                        SessionEntry {
                            slot_id: slot,
                            last_active: Instant::now(),
                            turns_count: 1,
                        },
                    );
                }
                let is_hit = existing_slot == Some(slot);
                return Some((slot, is_hit));
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
}
