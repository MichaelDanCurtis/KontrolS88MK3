//! Per-parameter send budget.
//!
//! Two independent suppression reasons are tracked separately because they
//! mean different things to a user: `deduped` means the value genuinely did
//! not change, while `rate_limited` means real movement was discarded to fit
//! the bus budget. The latter is the number that should worry someone.

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
}

#[derive(Debug, Clone, Copy)]
struct SlotState {
    last_raw: Option<u16>,
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
                last_raw: None,
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
            return false;
        };

        if slot.last_raw == Some(raw) {
            self.stats.deduped += 1;
            return false;
        }

        if let Some(last) = slot.last_sent_tick
            && tick.saturating_sub(last) < self.config.min_interval_ticks
        {
            self.stats.rate_limited += 1;
            return false;
        }

        slot.last_raw = Some(raw);
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
        let mut l = limiter(0);
        assert!(l.admit(0, 64, 0));
        assert!(!l.admit(0, 64, 100));
        assert_eq!(l.stats().deduped, 1);
        assert_eq!(l.stats().rate_limited, 0);
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
        // sent + deduped + rate_limited accounts for every call.
        let mut l = limiter(10);
        let mut calls = 0u64;
        for tick in 0..1000u64 {
            l.admit(0, (tick % 128) as u16, tick);
            calls += 1;
        }
        let s = l.stats();
        assert_eq!(s.sent + s.deduped + s.rate_limited, calls);
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
        assert_eq!(l.stats().sent, 0);
    }
}
