//! Hybrid Logical Clock.
//!
//! The total order is `(physical_ms, logical_counter, actor)` lexicographic.
//! Actor is the final tiebreak so concurrent ops from different replicas
//! cannot sort identically. See `docs/crdt.md`.

use crate::id::ActorId;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A Hybrid Logical Clock timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hlc {
    /// Wall-clock component in milliseconds since the Unix epoch.
    pub physical_ms: u64,
    /// Logical counter; increments when two physical components collide.
    pub logical: u32,
    /// Producer of the op; final tiebreak.
    pub actor: ActorId,
}

impl Hlc {
    /// Compose an HLC from its parts.
    pub fn new(physical_ms: u64, logical: u32, actor: ActorId) -> Self {
        Self {
            physical_ms,
            logical,
            actor,
        }
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.physical_ms
            .cmp(&other.physical_ms)
            .then(self.logical.cmp(&other.logical))
            .then(self.actor.0.cmp(&other.actor.0))
    }
}

/// Generates monotonic HLCs for one actor.
///
/// `next()` is cheap and safe to call concurrently via `Arc`. `observe()`
/// folds in a remote timestamp so future local ops are guaranteed to sort
/// after anything we've seen.
#[derive(Debug, Clone)]
pub struct HlcGenerator {
    actor: ActorId,
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Clone, Copy)]
struct State {
    physical_ms: u64,
    logical: u32,
}

impl HlcGenerator {
    /// Build a fresh generator with logical counter at zero.
    pub fn new(actor: ActorId) -> Self {
        Self {
            actor,
            state: Arc::new(Mutex::new(State {
                physical_ms: 0,
                logical: 0,
            })),
        }
    }

    /// Build a generator at a given starting state. Useful for tests.
    pub fn with_state(actor: ActorId, physical_ms: u64, logical: u32) -> Self {
        Self {
            actor,
            state: Arc::new(Mutex::new(State {
                physical_ms,
                logical,
            })),
        }
    }

    /// Actor id of this generator.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Produce the next monotonic HLC.
    pub fn next(&self) -> Hlc {
        let now = wall_clock_ms();
        let mut s = self.state.lock();
        if now > s.physical_ms {
            s.physical_ms = now;
            s.logical = 0;
        } else {
            s.logical = s.logical.saturating_add(1);
        }
        Hlc::new(s.physical_ms, s.logical, self.actor)
    }

    /// Fold a remote HLC into the local clock, then return a fresh HLC
    /// guaranteed to sort after both the previous local state and the
    /// observed remote.
    pub fn observe(&self, remote: Hlc) -> Hlc {
        let now = wall_clock_ms();
        let mut s = self.state.lock();
        let max_physical = s.physical_ms.max(remote.physical_ms).max(now);
        let new_logical = match (
            max_physical == s.physical_ms,
            max_physical == remote.physical_ms,
        ) {
            (true, true) => s.logical.max(remote.logical).saturating_add(1),
            (true, false) => s.logical.saturating_add(1),
            (false, true) => remote.logical.saturating_add(1),
            (false, false) => 0,
        };
        s.physical_ms = max_physical;
        s.logical = new_logical;
        Hlc::new(s.physical_ms, s.logical, self.actor)
    }
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_lexicographic_on_physical_logical_actor() {
        let a = ActorId::new();
        let b = ActorId::new();
        let h1 = Hlc::new(10, 0, a);
        let h2 = Hlc::new(10, 0, b);
        let h3 = Hlc::new(10, 1, a);
        let h4 = Hlc::new(11, 0, a);
        // Different actor breaks tie.
        assert_ne!(h1.cmp(&h2), Ordering::Equal);
        // Logical wins over actor.
        assert!(h1 < h3);
        // Physical wins over everything.
        assert!(h3 < h4);
    }

    #[test]
    fn generator_is_monotonic() {
        let actor = ActorId::new();
        let g = HlcGenerator::with_state(actor, 0, 0);
        let mut last = g.next();
        for _ in 0..100 {
            let next = g.next();
            assert!(next > last, "HLC went backwards: {last:?} → {next:?}");
            last = next;
        }
    }

    #[test]
    fn observe_advances_clock_past_remote() {
        let me = ActorId::new();
        let them = ActorId::new();
        let g = HlcGenerator::with_state(me, 0, 0);
        let remote = Hlc::new(1_000_000_000_000, 5, them);
        let after = g.observe(remote);
        assert!(after > remote, "observed HLC must dominate remote");
    }
}
