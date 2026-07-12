//! Deterministic inter-clan relationship memory.
//!
//! The ledger deliberately uses a sorted `Vec` instead of a hash map. Clan
//! counts are small, iteration order is stable, and the plain record layout can
//! be copied into a future versioned world snapshot without exposing a map's
//! implementation details.

pub type ClanId = i32;
pub type Tick = i32;

#[derive(Clone, Debug, PartialEq)]
pub struct Relationship {
    /// Canonical pair key. `clan_low` is always less than `clan_high`.
    pub clan_low: ClanId,
    pub clan_high: ClanId,
    /// Symmetric relationship memory in `-1.0..=1.0`.
    pub trust: f32,
    /// Pact is active while the current tick is strictly less than this value.
    pub pact_expires_tick: Option<Tick>,
    /// Decaying volume delivered in either direction between the pair.
    pub recent_food_delivered: f32,
    pub recent_wood_delivered: f32,
    /// Most recent completed delivery, independent of call order.
    pub last_trade_tick: Option<Tick>,
}

impl Relationship {
    pub fn pact_active(&self, current_tick: Tick) -> bool {
        match self.pact_expires_tick {
            Some(expires_tick) => current_tick < expires_tick,
            None => false,
        }
    }

    fn new(clan_low: ClanId, clan_high: ClanId) -> Self {
        Self {
            clan_low,
            clan_high,
            trust: 0.0,
            pact_expires_tick: None,
            recent_food_delivered: 0.0,
            recent_wood_delivered: 0.0,
            last_trade_tick: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiplomacyLedger {
    relationships: Vec<Relationship>,
}

impl DiplomacyLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stable snapshot/inspection order, sorted by `(clan_low, clan_high)`.
    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    /// Looks up either ordering of a clan pair. Self-relationships are invalid.
    pub fn lookup(&self, first: ClanId, second: ClanId) -> Option<&Relationship> {
        let key = canonical_pair(first, second)?;
        let index = self.find(key).ok()?;
        self.relationships.get(index)
    }

    /// Adds to symmetric trust, clamped to `-1.0..=1.0`.
    pub fn adjust(&mut self, first: ClanId, second: ClanId, delta: f32) -> Option<&Relationship> {
        let delta = finite_or_zero(delta);
        let relationship = self.get_or_insert(first, second)?;
        relationship.trust = (relationship.trust + delta).clamp(-1.0, 1.0);
        Some(relationship)
    }

    /// Sets or replaces a temporary pact expiry.
    pub fn set_pact(
        &mut self,
        first: ClanId,
        second: ClanId,
        expires_tick: Tick,
    ) -> Option<&Relationship> {
        let relationship = self.get_or_insert(first, second)?;
        relationship.pact_expires_tick = Some(expires_tick);
        Some(relationship)
    }

    /// Records material that was actually delivered, not merely offered.
    pub fn record_trade(
        &mut self,
        first: ClanId,
        second: ClanId,
        food_delivered: u32,
        wood_delivered: u32,
        tick: Tick,
    ) -> Option<&Relationship> {
        if food_delivered == 0 && wood_delivered == 0 {
            return self.lookup(first, second);
        }
        let relationship = self.get_or_insert(first, second)?;
        relationship.recent_food_delivered =
            saturating_f32_add(relationship.recent_food_delivered, food_delivered);
        relationship.recent_wood_delivered =
            saturating_f32_add(relationship.recent_wood_delivered, wood_delivered);
        relationship.last_trade_tick = Some(match relationship.last_trade_tick {
            Some(previous) => previous.max(tick),
            None => tick,
        });
        Some(relationship)
    }

    /// Applies one deterministic decay interval and clears expired pacts.
    ///
    /// Retention factors are clamped to `0.0..=1.0`; non-finite factors retain
    /// the prior value rather than contaminating persistent state with NaNs.
    pub fn decay(&mut self, current_tick: Tick, trust_retention: f32, trade_retention: f32) {
        let trust_retention = retention(trust_retention);
        let trade_retention = retention(trade_retention);

        for relationship in &mut self.relationships {
            relationship.trust *= trust_retention;
            relationship.recent_food_delivered *= trade_retention;
            relationship.recent_wood_delivered *= trade_retention;

            if matches!(relationship.pact_expires_tick, Some(expiry) if expiry <= current_tick) {
                relationship.pact_expires_tick = None;
            }
        }
    }

    /// Removes every relationship referencing a clan outside `live_clan_ids`.
    pub fn prune(&mut self, live_clan_ids: &[ClanId]) {
        let mut live = live_clan_ids.to_vec();
        live.sort_unstable();
        live.dedup();
        self.relationships.retain(|relationship| {
            live.binary_search(&relationship.clan_low).is_ok()
                && live.binary_search(&relationship.clan_high).is_ok()
        });
    }

    fn get_or_insert(&mut self, first: ClanId, second: ClanId) -> Option<&mut Relationship> {
        let key = canonical_pair(first, second)?;
        let index = match self.find(key) {
            Ok(index) => index,
            Err(index) => {
                self.relationships
                    .insert(index, Relationship::new(key.0, key.1));
                index
            }
        };
        self.relationships.get_mut(index)
    }

    fn find(&self, key: (ClanId, ClanId)) -> Result<usize, usize> {
        self.relationships
            .binary_search_by_key(&key, |relationship| {
                (relationship.clan_low, relationship.clan_high)
            })
    }
}

fn canonical_pair(first: ClanId, second: ClanId) -> Option<(ClanId, ClanId)> {
    if first == second {
        return None;
    }
    Some(if first < second {
        (first, second)
    } else {
        (second, first)
    })
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn retention(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn saturating_f32_add(current: f32, delivered: u32) -> f32 {
    (current + delivered as f32).min(f32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_lookup_is_symmetric_and_storage_is_sorted() {
        let mut ledger = DiplomacyLedger::new();
        ledger.adjust(8, 3, 0.25);
        ledger.adjust(5, 2, -0.5);
        ledger.adjust(1, 9, 0.75);

        assert_eq!(ledger.lookup(3, 8), ledger.lookup(8, 3));
        assert_eq!(ledger.lookup(8, 3).unwrap().trust, 0.25);
        assert_eq!(
            ledger
                .relationships()
                .iter()
                .map(|r| (r.clan_low, r.clan_high))
                .collect::<Vec<_>>(),
            vec![(1, 9), (2, 5), (3, 8)]
        );
        assert!(ledger.lookup(3, 3).is_none());
        assert_eq!(ledger.relationships().len(), 3);
    }

    #[test]
    fn trust_clamps_and_non_finite_adjustments_are_ignored() {
        let mut ledger = DiplomacyLedger::new();
        ledger.adjust(1, 2, 3.0);
        assert_eq!(ledger.lookup(1, 2).unwrap().trust, 1.0);

        ledger.adjust(2, 1, -4.0);
        assert_eq!(ledger.lookup(1, 2).unwrap().trust, -1.0);

        ledger.adjust(1, 2, f32::NAN);
        assert_eq!(ledger.lookup(1, 2).unwrap().trust, -1.0);
    }

    #[test]
    fn pacts_are_temporary_and_expire_during_decay() {
        let mut ledger = DiplomacyLedger::new();
        ledger.set_pact(4, 7, 120);

        let relationship = ledger.lookup(7, 4).unwrap();
        assert!(relationship.pact_active(119));
        assert!(!relationship.pact_active(120));

        ledger.decay(120, 1.0, 1.0);
        assert_eq!(ledger.lookup(4, 7).unwrap().pact_expires_tick, None);
    }

    #[test]
    fn delivered_trade_accumulates_and_keeps_latest_tick() {
        let mut ledger = DiplomacyLedger::new();
        assert!(ledger.record_trade(9, 2, 0, 0, 70).is_none());
        ledger.record_trade(9, 2, 12, 3, 80);
        ledger.record_trade(2, 9, 5, 7, 75);

        let relationship = ledger.lookup(9, 2).unwrap();
        assert_eq!(relationship.recent_food_delivered, 17.0);
        assert_eq!(relationship.recent_wood_delivered, 10.0);
        assert_eq!(relationship.last_trade_tick, Some(80));
    }

    #[test]
    fn decay_is_stable_and_prune_removes_dead_clans() {
        let mut ledger = DiplomacyLedger::new();
        ledger.adjust(1, 2, 0.8);
        ledger.record_trade(1, 2, 20, 10, 40);
        ledger.adjust(2, 3, -0.4);

        ledger.decay(50, 0.5, 0.25);
        let relationship = ledger.lookup(1, 2).unwrap();
        assert_eq!(relationship.trust, 0.4);
        assert_eq!(relationship.recent_food_delivered, 5.0);
        assert_eq!(relationship.recent_wood_delivered, 2.5);

        ledger.prune(&[3, 2, 2]);
        assert!(ledger.lookup(1, 2).is_none());
        assert!(ledger.lookup(2, 3).is_some());
        assert_eq!(ledger.relationships().len(), 1);
    }
}
