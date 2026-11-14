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
    idle_reclaim_secs: u64,
    sessions: RwLock<HashMap<String, SessionEntry>>,
    slot_busy: Vec<AtomicBool>,
    slot_prefix: RwLock<Vec<Option<u64>>>,
    slot_last_active: RwLock<Vec<Instant>>,
}

/// Hashes the first system prompt or initial turn content to identify warm KV cache prefixes.
pub fn compute_prefix_hash(messages: &[serde_json::Value]) -> u64 {
    const MAX_PREFIX_BYTES: usize = 8192;
    let Some(first) = messages.first() else {
        return 0;
    };

    let mut hasher = DefaultHasher::new();
    let mut hashed_bytes: usize = 0;

    if let Some(role) = first.get("role").and_then(|r| r.as_str()) {
        role.hash(&mut hasher);
    }

    if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
        let bytes = content.as_bytes();
        let take = bytes.len().min(MAX_PREFIX_BYTES);
        if take > 0 {
            bytes[..take].hash(&mut hasher);
            hashed_bytes += take;
        }
    } else if let Some(parts) = first.get("content").and_then(|c| c.as_array()) {
        for part in parts {
            if hashed_bytes >= MAX_PREFIX_BYTES {
                break;
            }
            let text_opt = part
                .get("text")
                .and_then(|t| t.as_str())
                .or_else(|| part.as_str());
            if let Some(text) = text_opt {
                let bytes = text.as_bytes();
                let take = bytes.len().min(MAX_PREFIX_BYTES - hashed_bytes);
                if take > 0 {
                    bytes[..take].hash(&mut hasher);
                    hashed_bytes += take;
                }
            }
        }
    }

    if hashed_bytes == 0 {
        return 0;
    }
    let h = hasher.finish();
    if h == 0 { 1 } else { h }
}

impl SessionAffinityTracker {
    pub fn new(enabled: bool, max_slots: usize) -> Self {
        Self::with_idle_reclaim(enabled, max_slots, 10)
    }

    pub fn with_idle_reclaim(enabled: bool, max_slots: usize, idle_reclaim_secs: u64) -> Self {
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
            idle_reclaim_secs,
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
                        let prev_turns = write.get(session_id).map(|e| e.turns_count).unwrap_or(0);
                        let prev_prefix = write.get(session_id).map(|e| e.prefix_hash).unwrap_or(0);
                        write.retain(|k, v| k == session_id || v.slot_id != preferred);
                        write.insert(
                            session_id.to_string(),
                            SessionEntry {
                                slot_id: preferred,
                                prefix_hash: if prefix_hash != 0 { prefix_hash } else { prev_prefix },
                                last_active: now,
                                turns_count: prev_turns + 1,
                            },
                        );
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

        // Compute active leases held by other sessions (< idle_reclaim_secs), using deterministic max timestamp per slot.
        let active_assigned_slots: HashMap<usize, Instant> = {
            let read = self.sessions.read().ok()?;
            let mut map: HashMap<usize, Instant> = HashMap::new();
            for (k, e) in read.iter() {
                if k != session_id && now.duration_since(e.last_active).as_secs() < self.idle_reclaim_secs {
                    map.entry(e.slot_id)
                        .and_modify(|t| {
                            if e.last_active > *t {
                                *t = e.last_active;
                            }
                        })
                        .or_insert(e.last_active);
                }
            }
            map
        };

        // 2. Preferred slot is busy or session is new:
        // Check for an IDLE slot that already has the MATCHING prefix_hash (Warm Prefix Reuse).
        // Prefer unleased/expired warm slots first; only steal actively leased warm slots if allow_eviction is true.
        if prefix_hash != 0 {
            let matching_slots: Vec<usize> = if let Ok(p_read) = self.slot_prefix.read() {
                (0..self.max_slots)
                    .filter(|&slot| slot < p_read.len() && p_read[slot] == Some(prefix_hash))
                    .collect()
            } else {
                Vec::new()
            };

            let mut candidates: Vec<usize> = matching_slots
                .iter()
                .copied()
                .filter(|slot| !active_assigned_slots.contains_key(slot))
                .collect();

            if allow_eviction {
                let mut leased_warm: Vec<usize> = matching_slots
                    .into_iter()
                    .filter(|slot| active_assigned_slots.contains_key(slot))
                    .collect();
                leased_warm.sort_by_key(|slot| active_assigned_slots.get(slot).copied().unwrap_or(now));
                candidates.extend(leased_warm);
            }

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
                        if slot < p_write.len() {
                            p_write[slot] = Some(prefix_hash);
                        }
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

        // 3. Find an idle slot, prioritizing unassigned slots or slots whose lease expired (idle >= idle_reclaim_secs).
        for slot in 0..self.max_slots {
            if !active_assigned_slots.contains_key(&slot) {
                if !self.slot_busy[slot].swap(true, Ordering::SeqCst) {
                    if let Ok(mut write) = self.sessions.write() {
                        // Evict stale inactive session if one previously held this slot
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
                        if slot < p_write.len() {
                            p_write[slot] = if prefix_hash != 0 { Some(prefix_hash) } else { None };
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

        // 4. All idle slots are actively assigned to other recent sessions (< idle_reclaim_secs).
        if !allow_eviction {
            return None;
        }

        // Pick an idle slot whose assigned session was active longest ago (LRU eviction), using deterministic max timestamp per slot.
        let all_assigned_slots: HashMap<usize, Instant> = {
            let read = self.sessions.read().ok()?;
            let mut map: HashMap<usize, Instant> = HashMap::new();
            for e in read.values() {
                map.entry(e.slot_id)
                    .and_modify(|t| {
                        if e.last_active > *t {
                            *t = e.last_active;
                        }
                    })
                    .or_insert(e.last_active);
            }
            map
        };

        let mut candidates: Vec<usize> = (0..self.max_slots).collect();
        candidates.sort_by_key(|slot| {
            (
                all_assigned_slots.contains_key(slot),
                all_assigned_slots.get(slot).copied().unwrap_or(now),
            )
        });

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
                    if slot < p_write.len() {
                        p_write[slot] = if prefix_hash != 0 { Some(prefix_hash) } else { None };
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

    #[test]
    fn test_idle_lease_reclaim_without_eviction_flag() {
        // Tracker with 0s idle reclaim threshold: once a slot is released, it is immediately reclaimable
        let tracker = SessionAffinityTracker::with_idle_reclaim(true, 1, 0);

        let (s1, _) = tracker.try_acquire_slot_opt("session-1", 100, true).unwrap();
        assert_eq!(s1, 0);
        tracker.release_slot(s1);

        // Session 2 arrives with allow_eviction = false.
        // Because session-1's slot is idle and its lease expired (>= 0s), session-2 acquires it immediately!
        let s2 = tracker.try_acquire_slot_opt("session-2", 200, false);
        assert!(s2.is_some(), "Expired idle slot should be reclaimable even with allow_eviction = false");
        let (slot2, aff2) = s2.unwrap();
        assert_eq!(slot2, 0);
        assert_eq!(aff2, AffinityType::NewSlot);
    }

    #[test]
    fn test_prefix_hit_one_to_one_slot_ownership_and_unleased_preference() {
        let tracker = SessionAffinityTracker::with_idle_reclaim(true, 2, 60);
        let shared_prefix = 0xDEAD_BEEF_9999;

        // Many distinct subagent sessions reuse warm slots via PrefixHit (allow_eviction = true)
        for i in 0..40 {
            let sid = format!("subagent-{i}");
            let (slot, _aff) = tracker
                .try_acquire_slot_opt(&sid, shared_prefix, true)
                .unwrap();
            tracker.release_slot(slot);
        }

        // Invariant: self.sessions must NEVER leak unbounded entries or map multiple sessions to one slot_id
        let sessions = tracker.sessions.read().unwrap();
        assert!(
            sessions.len() <= 2,
            "SessionAffinityTracker leaked SessionEntry records on PrefixHit: len={}",
            sessions.len()
        );
        let mut seen_slots = std::collections::HashSet::new();
        for entry in sessions.values() {
            assert!(
                seen_slots.insert(entry.slot_id),
                "Duplicate sessions mapped to the same slot_id={}",
                entry.slot_id
            );
        }
        drop(sessions);

        // Also verify unleased/expired warm slot is preferred over actively leased warm slot
        let tracker_pref = SessionAffinityTracker::with_idle_reclaim(true, 2, 60);
        // Prime both slot 0 and slot 1 with shared_prefix
        let (s0, _) = tracker_pref.try_acquire_slot_opt("sess-a", shared_prefix, true).unwrap();
        let (s1, _) = tracker_pref.try_acquire_slot_opt("sess-b", shared_prefix, true).unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        tracker_pref.release_slot(s0);
        tracker_pref.release_slot(s1);

        // Expire sess-b's lease on slot 1 while keeping sess-a's active lease on slot 0
        {
            let mut w = tracker_pref.sessions.write().unwrap();
            if let Some(entry_b) = w.get_mut("sess-b") {
                entry_b.last_active = Instant::now() - std::time::Duration::from_secs(120);
            }
        }

        // Caller with allow_eviction = false MUST skip leased slot 0 and acquire expired warm slot 1 as PrefixHit!
        let (chosen_slot, chosen_aff) = tracker_pref
            .try_acquire_slot_opt("sess-c", shared_prefix, false)
            .expect("Expired warm slot 1 should be acquired via PrefixHit even with !allow_eviction");
        assert_eq!(chosen_slot, 1, "Must prefer unleased/expired warm slot 1 over leased warm slot 0");
        assert_eq!(chosen_aff, AffinityType::PrefixHit);
        tracker_pref.release_slot(chosen_slot);
    }
}
