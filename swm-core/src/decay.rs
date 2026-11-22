use crate::field::NodeId;
use std::collections::HashMap;

/// Where a window stands for automatic decay. Only a window entirely off
/// screen counts down; one you can see, even partly, never decays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecayClass {
    Protected,
    OnScreen,
    OffScreen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedDecayPolicy {
    /// How long a window must stay off screen before it collapses.
    pub delay_ms: u64,
}

impl Default for TimedDecayPolicy {
    fn default() -> Self {
        Self { delay_ms: 600_000 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Eligibility {
    class: DecayClass,
    since_ms: u64,
}

impl Eligibility {
    fn deadline(self, policy: TimedDecayPolicy) -> Option<u64> {
        match self.class {
            DecayClass::Protected | DecayClass::OnScreen => None,
            DecayClass::OffScreen => Some(self.since_ms.saturating_add(policy.delay_ms)),
        }
    }
}

/// Pure monotonic eligibility state. A class change always starts a new
/// clock, which is the key invariant that prevents an old last-focus time
/// from collapsing a window as soon as focus leaves it, or as soon as it
/// leaves the screen again.
#[derive(Default)]
pub struct DecayTracker {
    eligibility: HashMap<NodeId, Eligibility>,
}

impl DecayTracker {
    pub fn update(
        &mut self,
        id: NodeId,
        class: DecayClass,
        now_ms: u64,
        policy: TimedDecayPolicy,
    ) -> Option<u64> {
        let entry = self.eligibility.entry(id).or_insert(Eligibility {
            class,
            since_ms: now_ms,
        });
        if entry.class != class {
            *entry = Eligibility {
                class,
                since_ms: now_ms,
            };
        }
        entry
            .deadline(policy)
            .filter(|deadline| now_ms >= *deadline)
    }

    pub fn next_deadline(&self, now_ms: u64, policy: TimedDecayPolicy) -> Option<u64> {
        self.eligibility
            .values()
            .filter_map(|entry| entry.deadline(policy))
            .filter(|deadline| *deadline > now_ms)
            .min()
    }

    pub fn remove(&mut self, id: NodeId) {
        self.eligibility.remove(&id);
    }

    pub fn reset(&mut self) {
        self.eligibility.clear();
    }
}

#[cfg(test)]
mod timed_tests {
    use super::{DecayClass, DecayTracker, TimedDecayPolicy};
    use crate::field::NodeId;

    fn id(value: u64) -> NodeId {
        NodeId::new(value)
    }

    const POLICY: TimedDecayPolicy = TimedDecayPolicy { delay_ms: 100 };

    #[test]
    fn windows_on_screen_never_decay() {
        let mut tracker = DecayTracker::default();
        assert_eq!(tracker.update(id(1), DecayClass::OnScreen, 0, POLICY), None);
        assert_eq!(
            tracker.update(id(1), DecayClass::OnScreen, u64::MAX, POLICY),
            None
        );
        assert_eq!(tracker.next_deadline(0, POLICY), None);
    }

    #[test]
    fn off_screen_windows_decay_after_the_delay() {
        let mut tracker = DecayTracker::default();
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 10, POLICY),
            None
        );
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 109, POLICY),
            None
        );
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 110, POLICY),
            Some(110)
        );
    }

    #[test]
    fn coming_back_on_screen_restarts_the_timer() {
        let mut tracker = DecayTracker::default();
        tracker.update(id(1), DecayClass::OffScreen, 0, POLICY);
        tracker.update(id(1), DecayClass::OnScreen, 99, POLICY);
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 500, POLICY),
            None
        );
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 599, POLICY),
            None
        );
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 600, POLICY),
            Some(600)
        );
    }

    #[test]
    fn focus_protection_does_not_reuse_an_old_deadline() {
        let mut tracker = DecayTracker::default();
        tracker.update(id(1), DecayClass::OffScreen, 0, POLICY);
        tracker.update(id(1), DecayClass::Protected, 99, POLICY);
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 500, POLICY),
            None
        );
        assert_eq!(
            tracker.update(id(1), DecayClass::OffScreen, 600, POLICY),
            Some(600)
        );
    }

    #[test]
    fn next_deadline_counts_only_off_screen_windows() {
        let mut tracker = DecayTracker::default();
        tracker.update(id(1), DecayClass::Protected, 0, POLICY);
        tracker.update(id(2), DecayClass::OnScreen, 10, POLICY);
        tracker.update(id(3), DecayClass::OffScreen, 20, POLICY);
        assert_eq!(tracker.next_deadline(20, POLICY), Some(120));
    }
}
