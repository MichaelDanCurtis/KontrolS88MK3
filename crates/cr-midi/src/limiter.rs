//! Per-parameter send budget.
//!
//! Suppression reasons are tracked separately because they mean different
//! things to a user: `deduped` means the value genuinely did not change,
//! `rate_limited` means real movement was discarded to fit the bus budget,
//! and `unknown_slot` means a stale or invalid key — a programming error, not
//! a bus-capacity problem. Conflating the last two would send someone
//! simplifying their patch when the real fault is a bad key.
//!
//! Every `admit` call increments exactly one counter, so
//! `sent + deduped + rate_limited + unknown_slot` equals the call count.
//! That identity is what makes silent suppression impossible.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimiterConfig {
    /// Minimum ticks between two sends for the same parameter.
    pub min_interval_ticks: u64,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        // 10 ticks at 1 kHz => 100 messages/second/parameter.
        Self {
            min_interval_ticks: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimiterStats {
    pub sent: u64,
    pub deduped: u64,
    pub rate_limited: u64,
    pub unknown_slot: u64,
}

#[derive(Debug, Clone, Copy)]
struct SlotState {
    last_sent_raw: Option<u16>,
    last_sent_tick: Option<u64>,
}

pub struct Limiter {
    config: LimiterConfig,
    slots: Vec<SlotState>,
    stats: LimiterStats,
}

impl Limiter {
    pub fn new(config: LimiterConfig) -> Self {
        Self {
            config,
            slots: Vec::new(),
            stats: LimiterStats::default(),
        }
    }

    /// Size the slot table. Call once per graph apply, never inside the tick.
    pub fn reset(&mut self, slots: usize) {
        self.slots = vec![
            SlotState {
                last_sent_raw: None,
                last_sent_tick: None,
            };
            slots
        ];
        self.stats = LimiterStats::default();
    }

    /// Decide whether a parameter update should reach the wire.
    ///
    /// Returns `true` to send. Allocation-free.
    pub fn admit(&mut self, key: usize, raw: u16, tick: u64) -> bool {
        let Some(slot) = self.slots.get_mut(key) else {
            // Unknown slot: fail closed rather than flooding the bus.
            // Count this as a suppression so the accounting invariant remains true.
            self.stats.unknown_slot += 1;
            return false;
        };

        if slot.last_sent_raw == Some(raw) {
            self.stats.deduped += 1;
            return false;
        }

        if let Some(last) = slot.last_sent_tick
            && tick.saturating_sub(last) < self.config.min_interval_ticks
        {
            self.stats.rate_limited += 1;
            return false;
        }

        slot.last_sent_raw = Some(raw);
        slot.last_sent_tick = Some(tick);
        self.stats.sent += 1;
        true
    }

    pub fn stats(&self) -> LimiterStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(interval: u64) -> Limiter {
        let mut l = Limiter::new(LimiterConfig {
            min_interval_ticks: interval,
        });
        l.reset(1);
        l
    }

    #[test]
    fn first_update_is_always_admitted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert_eq!(l.stats().sent, 1);
    }

    #[test]
    fn identical_value_is_deduped_not_rate_limited() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        // Send same value within the rate-limit window to prove dedup is checked first
        assert!(
            !l.admit(0, 64, 5),
            "identical value should be deduped even within window"
        );
        assert_eq!(l.stats().deduped, 1, "dedup must be counted");
        assert_eq!(
            l.stats().rate_limited,
            0,
            "dedup does not count as rate_limited"
        );
    }

    #[test]
    fn changed_value_too_soon_is_rate_limited_and_counted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert!(!l.admit(0, 65, 5), "5 ticks < 10 tick interval");
        assert_eq!(l.stats().rate_limited, 1);
        assert_eq!(l.stats().deduped, 0);
    }

    #[test]
    fn changed_value_after_interval_is_admitted() {
        let mut l = limiter(10);
        assert!(l.admit(0, 64, 0));
        assert!(l.admit(0, 65, 10));
        assert_eq!(l.stats().sent, 2);
    }

    #[test]
    fn suppressed_values_are_never_silently_lost() {
        // Every rejected admit must land in exactly one counter, so
        // sent + deduped + rate_limited + unknown_slot accounts for every call.
        let mut l = limiter(10);
        let mut calls = 0u64;
        for tick in 0..1000u64 {
            l.admit(0, (tick % 128) as u16, tick);
            calls += 1;
        }
        // Also test unknown slot calls
        for tick in 1000..1010u64 {
            l.admit(99, 42, tick);
            calls += 1;
        }
        let s = l.stats();
        assert_eq!(
            s.sent + s.deduped + s.rate_limited + s.unknown_slot,
            calls,
            "All suppression reasons must be accounted for"
        );
    }

    #[test]
    fn parameters_have_independent_budgets() {
        let mut l = Limiter::new(LimiterConfig {
            min_interval_ticks: 10,
        });
        l.reset(2);
        assert!(l.admit(0, 1, 0));
        assert!(l.admit(1, 1, 0), "second parameter has its own budget");
    }

    #[test]
    fn unknown_slot_fails_closed() {
        let mut l = limiter(10);
        assert!(!l.admit(99, 1, 0));
        let s = l.stats();
        assert_eq!(s.sent, 0);
        assert_eq!(
            s.unknown_slot, 1,
            "unknown slot suppression must be counted"
        );
        // Verify the accounting invariant: one call should produce exactly one counter increment
        assert_eq!(s.sent + s.deduped + s.rate_limited + s.unknown_slot, 1);
    }
}
