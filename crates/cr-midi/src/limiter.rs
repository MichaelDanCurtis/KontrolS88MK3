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

/// Aggregate MIDI-bus budget expressed against the engine's deterministic
/// tick clock.
///
/// The limiter uses a token bucket with integer arithmetic. One token is one
/// complete MIDI message, regardless of which parameter produced it. Tokens
/// accrue at `max_messages_per_second`; `burst_messages` bounds both idle-time
/// accumulation and same-tick bursts. No wall clock is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusLimiterConfig {
    /// Number of engine ticks in one second (normally 1,000).
    pub ticks_per_second: u64,
    /// Sustained aggregate allowance across every parameter on the bus.
    pub max_messages_per_second: u64,
    /// Maximum accumulated allowance. This must be at least as large as the
    /// biggest atomic encoding (four messages for NRPN) if that encoding is
    /// expected to pass.
    pub burst_messages: u64,
}

impl Default for BusLimiterConfig {
    fn default() -> Self {
        Self {
            ticks_per_second: 1_000,
            // Leaves headroom below the project's conservative roughly
            // 320-message/second DIN operating budget.
            max_messages_per_second: 300,
            burst_messages: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BusLimiterStats {
    /// Parameter updates admitted by the aggregate budget.
    pub admitted_updates: u64,
    /// Complete MIDI messages represented by admitted updates.
    pub admitted_messages: u64,
    /// Parameter updates suppressed by the aggregate budget.
    pub rate_limited_updates: u64,
    /// Complete MIDI messages represented by suppressed updates.
    pub rate_limited_messages: u64,
}

/// Allocation-free aggregate limiter. Usually accessed through
/// [`BudgetedLimiter`], which makes aggregate and per-parameter admission
/// atomic.
pub struct BusLimiter {
    config: BusLimiterConfig,
    /// Token units use `ticks_per_second` units per complete MIDI message.
    available_units: u128,
    last_tick: Option<u64>,
    stats: BusLimiterStats,
}

impl BusLimiter {
    pub fn new(mut config: BusLimiterConfig) -> Self {
        // A zero timebase would make every update cost zero and effectively
        // disable limiting. Normalize it here so even unvalidated external
        // configuration fails safe without introducing a panic in the audio
        // process.
        config.ticks_per_second = config.ticks_per_second.max(1);
        let available_units =
            u128::from(config.burst_messages) * u128::from(config.ticks_per_second);
        Self {
            config,
            available_units,
            last_tick: None,
            stats: BusLimiterStats::default(),
        }
    }

    /// Reset timing and accounting. The bucket starts with its configured
    /// burst allowance so the first atomic update can be sent immediately.
    pub fn reset(&mut self) {
        self.available_units =
            u128::from(self.config.burst_messages) * u128::from(self.config.ticks_per_second);
        self.last_tick = None;
        self.stats = BusLimiterStats::default();
    }

    /// Decide whether an atomic update containing `messages` complete MIDI
    /// messages fits the aggregate budget.
    ///
    /// A zero-message update is admitted and accounted as an update without
    /// consuming capacity. An update larger than `burst_messages` can never
    /// fit and therefore fails closed.
    pub fn admit(&mut self, messages: u8, tick: u64) -> bool {
        self.refill(tick);

        let messages = u64::from(messages);
        let cost = u128::from(messages) * u128::from(self.config.ticks_per_second);
        if cost > self.available_units {
            self.stats.rate_limited_updates += 1;
            self.stats.rate_limited_messages += messages;
            return false;
        }

        self.available_units -= cost;
        self.stats.admitted_updates += 1;
        self.stats.admitted_messages += messages;
        true
    }

    pub fn stats(&self) -> BusLimiterStats {
        self.stats
    }

    fn refill(&mut self, tick: u64) {
        let previous = self.last_tick.unwrap_or(tick);
        let effective_tick = tick.max(previous);
        self.last_tick = Some(effective_tick);
        let elapsed = effective_tick - previous;
        let earned = u128::from(elapsed) * u128::from(self.config.max_messages_per_second);
        let capacity =
            u128::from(self.config.burst_messages) * u128::from(self.config.ticks_per_second);
        self.available_units = self.available_units.saturating_add(earned).min(capacity);
    }
}

/// Suppression accounting for the combined per-parameter and aggregate
/// limiter. Every [`BudgetedLimiter::admit`] call increments exactly one of
/// `sent`, `deduped`, `parameter_rate_limited`, `bus_rate_limited`, or
/// `unknown_slot`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BudgetedLimiterStats {
    pub sent: u64,
    pub sent_messages: u64,
    pub deduped: u64,
    pub parameter_rate_limited: u64,
    pub bus_rate_limited: u64,
    pub bus_rate_limited_messages: u64,
    pub unknown_slot: u64,
}

/// Combined per-parameter and aggregate MIDI-bus limiter.
///
/// This type exists instead of naively chaining [`Limiter::admit`] and
/// [`BusLimiter::admit`]. Chaining would commit a parameter value as "sent"
/// before the bus limiter could reject it, causing subsequent updates to be
/// incorrectly deduplicated. This limiter commits both budgets atomically.
pub struct BudgetedLimiter {
    parameter_config: LimiterConfig,
    slots: Vec<SlotState>,
    bus: BusLimiter,
    stats: BudgetedLimiterStats,
}

impl BudgetedLimiter {
    pub fn new(parameter_config: LimiterConfig, bus_config: BusLimiterConfig) -> Self {
        Self {
            parameter_config,
            slots: Vec::new(),
            bus: BusLimiter::new(bus_config),
            stats: BudgetedLimiterStats::default(),
        }
    }

    /// Size the parameter table and reset both budgets. Call once per graph
    /// apply, never inside the tick.
    pub fn reset(&mut self, slots: usize) {
        self.slots = vec![
            SlotState {
                last_sent_raw: None,
                last_sent_tick: None,
            };
            slots
        ];
        self.bus.reset();
        self.stats = BudgetedLimiterStats::default();
    }

    /// Atomically apply deduplication, the per-parameter interval, and the
    /// aggregate bus budget. Allocation-free after [`Self::reset`].
    pub fn admit(&mut self, key: usize, raw: u16, tick: u64, messages: u8) -> bool {
        let Some(slot) = self.slots.get_mut(key) else {
            self.stats.unknown_slot += 1;
            return false;
        };

        if slot.last_sent_raw == Some(raw) {
            self.stats.deduped += 1;
            return false;
        }

        if let Some(last) = slot.last_sent_tick
            && tick.saturating_sub(last) < self.parameter_config.min_interval_ticks
        {
            self.stats.parameter_rate_limited += 1;
            return false;
        }

        if !self.bus.admit(messages, tick) {
            self.stats.bus_rate_limited += 1;
            self.stats.bus_rate_limited_messages += u64::from(messages);
            return false;
        }

        slot.last_sent_raw = Some(raw);
        slot.last_sent_tick = Some(tick);
        self.stats.sent += 1;
        self.stats.sent_messages += u64::from(messages);
        true
    }

    pub fn stats(&self) -> BudgetedLimiterStats {
        self.stats
    }

    pub fn bus_stats(&self) -> BusLimiterStats {
        self.bus.stats()
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

    fn bus_limiter(rate: u64, burst: u64) -> BusLimiter {
        BusLimiter::new(BusLimiterConfig {
            ticks_per_second: 1_000,
            max_messages_per_second: rate,
            burst_messages: burst,
        })
    }

    #[test]
    fn aggregate_budget_is_shared_by_all_callers() {
        let mut bus = bus_limiter(100, 4);
        assert!(bus.admit(2, 0));
        assert!(bus.admit(2, 0));
        assert!(!bus.admit(1, 0), "the shared burst budget is exhausted");

        let stats = bus.stats();
        assert_eq!(stats.admitted_messages, 4);
        assert_eq!(stats.rate_limited_updates, 1);
        assert_eq!(stats.rate_limited_messages, 1);
    }

    #[test]
    fn aggregate_refill_is_deterministic_and_tick_based() {
        let mut bus = bus_limiter(100, 1);
        assert!(bus.admit(1, 0));
        assert!(!bus.admit(1, 9));
        assert!(bus.admit(1, 10), "10 ticks at 100/s earns one message");

        // Rewinding the caller's tick cannot create capacity.
        assert!(!bus.admit(1, 5));
        assert!(bus.admit(1, 20));
    }

    #[test]
    fn zero_tick_rate_is_normalized_instead_of_disabling_the_budget() {
        let mut bus = BusLimiter::new(BusLimiterConfig {
            ticks_per_second: 0,
            max_messages_per_second: 1,
            burst_messages: 1,
        });
        assert!(bus.admit(1, 0));
        assert!(!bus.admit(1, 0));
        assert!(bus.admit(1, 1));
    }

    #[test]
    fn aggregate_budget_counts_multi_message_encodings_atomically() {
        let mut bus = bus_limiter(100, 4);
        assert!(bus.admit(4, 0), "one NRPN update consumes four messages");
        assert!(!bus.admit(1, 0));
        assert!(
            !bus.admit(4, 10),
            "only one message has accrued after 10 ticks"
        );
        assert!(
            bus.admit(4, 40),
            "four messages have accrued after 40 ticks"
        );
    }

    #[test]
    fn aggregate_rate_is_bounded_across_many_parameters() {
        let mut limiter = BudgetedLimiter::new(
            LimiterConfig {
                min_interval_ticks: 0,
            },
            BusLimiterConfig {
                ticks_per_second: 1_000,
                max_messages_per_second: 100,
                burst_messages: 1,
            },
        );
        limiter.reset(8);

        for tick in 0..1_000 {
            for key in 0..8 {
                limiter.admit(key, (tick + key as u64) as u16, tick, 1);
            }
        }

        let stats = limiter.stats();
        // Initial burst of one plus 99 refills within ticks 0..999.
        assert_eq!(stats.sent_messages, 100);
        assert!(stats.bus_rate_limited > 0);
        assert_eq!(
            stats.sent
                + stats.deduped
                + stats.parameter_rate_limited
                + stats.bus_rate_limited
                + stats.unknown_slot,
            8_000,
        );
    }

    #[test]
    fn bus_rejection_does_not_commit_parameter_state() {
        let mut limiter = BudgetedLimiter::new(
            LimiterConfig {
                min_interval_ticks: 10,
            },
            BusLimiterConfig {
                ticks_per_second: 1_000,
                max_messages_per_second: 100,
                burst_messages: 1,
            },
        );
        limiter.reset(2);

        assert!(limiter.admit(0, 10, 0, 1));
        assert!(!limiter.admit(1, 20, 0, 1));
        assert!(
            limiter.admit(1, 20, 10, 1),
            "a bus-rejected value must remain eligible once capacity returns"
        );

        let stats = limiter.stats();
        assert_eq!(stats.sent, 2);
        assert_eq!(stats.bus_rate_limited, 1);
        assert_eq!(stats.deduped, 0);
        assert_eq!(stats.parameter_rate_limited, 0);
    }

    #[test]
    fn combined_suppression_reasons_are_distinct_and_exhaustive() {
        let mut limiter = BudgetedLimiter::new(
            LimiterConfig {
                min_interval_ticks: 10,
            },
            BusLimiterConfig {
                ticks_per_second: 1_000,
                max_messages_per_second: 100,
                burst_messages: 1,
            },
        );
        limiter.reset(2);

        assert!(limiter.admit(0, 1, 0, 1)); // sent
        assert!(!limiter.admit(0, 1, 1, 1)); // deduped
        assert!(!limiter.admit(0, 2, 1, 1)); // per-parameter interval
        assert!(!limiter.admit(1, 3, 1, 1)); // aggregate budget
        assert!(!limiter.admit(99, 4, 1, 1)); // unknown slot

        let stats = limiter.stats();
        assert_eq!(stats.sent, 1);
        assert_eq!(stats.deduped, 1);
        assert_eq!(stats.parameter_rate_limited, 1);
        assert_eq!(stats.bus_rate_limited, 1);
        assert_eq!(stats.unknown_slot, 1);
        assert_eq!(
            stats.sent
                + stats.deduped
                + stats.parameter_rate_limited
                + stats.bus_rate_limited
                + stats.unknown_slot,
            5,
        );
    }
}
