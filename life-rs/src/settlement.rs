//! Deterministic settlement buildings and technology progression.
//!
//! This module contains data and pure accounting only. `World` owns placement,
//! resource spending, worker movement, and tick scheduling so settlement state
//! can be integrated without changing the fixed `LFB1` brain dimensions.

pub const MAX_TECH_LEVEL: u8 = 3;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct BuildingId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BuildingKind {
    House,
    Granary,
    Workshop,
    Market,
    Wall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildingCost {
    pub wood: i32,
    pub work: u16,
}

impl BuildingKind {
    #[cfg(test)]
    pub const ALL: [BuildingKind; 5] = [
        BuildingKind::House,
        BuildingKind::Granary,
        BuildingKind::Workshop,
        BuildingKind::Market,
        BuildingKind::Wall,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            BuildingKind::House => "house",
            BuildingKind::Granary => "granary",
            BuildingKind::Workshop => "workshop",
            BuildingKind::Market => "market",
            BuildingKind::Wall => "wall",
        }
    }

    pub const fn cost(self) -> BuildingCost {
        match self {
            BuildingKind::House => BuildingCost { wood: 12, work: 24 },
            BuildingKind::Granary => BuildingCost { wood: 18, work: 36 },
            BuildingKind::Workshop => BuildingCost { wood: 24, work: 48 },
            BuildingKind::Market => BuildingCost { wood: 30, work: 60 },
            BuildingKind::Wall => BuildingCost { wood: 10, work: 20 },
        }
    }

    pub const fn unlock_level(self) -> u8 {
        match self {
            BuildingKind::House | BuildingKind::Granary | BuildingKind::Workshop => 0,
            BuildingKind::Wall => 1,
            BuildingKind::Market => 2,
        }
    }

    pub const fn max_hp(self) -> u16 {
        match self {
            BuildingKind::House => 80,
            BuildingKind::Granary => 100,
            BuildingKind::Workshop => 120,
            BuildingKind::Market => 100,
            BuildingKind::Wall => 180,
        }
    }

    pub const fn development_value(self) -> u32 {
        match self {
            BuildingKind::House | BuildingKind::Wall => 1,
            BuildingKind::Granary => 2,
            BuildingKind::Workshop => 3,
            BuildingKind::Market => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Building {
    pub id: BuildingId,
    pub clan_id: i32,
    pub x: i32,
    pub y: i32,
    pub kind: BuildingKind,
    pub construction: u16,
    pub hp: u16,
}

impl Building {
    pub fn new(id: BuildingId, clan_id: i32, x: i32, y: i32, kind: BuildingKind) -> Self {
        Self {
            id,
            clan_id,
            x,
            y,
            kind,
            construction: 0,
            hp: kind.max_hp(),
        }
    }

    pub const fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn is_complete(&self) -> bool {
        self.construction >= self.kind.cost().work
    }

    pub const fn is_destroyed(&self) -> bool {
        self.hp == 0
    }

    pub fn is_active(&self) -> bool {
        self.is_complete() && !self.is_destroyed()
    }

    pub fn completion_fraction(&self) -> f32 {
        let required = self.kind.cost().work.max(1);
        (self.construction.min(required) as f32) / (required as f32)
    }

    /// Applies construction work and returns the amount actually consumed.
    pub fn add_construction(&mut self, work: u16) -> u16 {
        let remaining = self.kind.cost().work.saturating_sub(self.construction);
        let consumed = remaining.min(work);
        self.construction += consumed;
        consumed
    }

    /// Applies damage and returns the amount actually dealt.
    #[cfg(test)]
    pub fn take_damage(&mut self, damage: u16) -> u16 {
        let dealt = self.hp.min(damage);
        self.hp -= dealt;
        dealt
    }

    /// Restores hit points and returns the amount actually repaired.
    #[cfg(test)]
    pub fn repair(&mut self, amount: u16) -> u16 {
        let missing = self.kind.max_hp().saturating_sub(self.hp);
        let repaired = missing.min(amount);
        self.hp += repaired;
        repaired
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildingCounts {
    pub houses: u16,
    pub granaries: u16,
    pub workshops: u16,
    pub markets: u16,
    pub walls: u16,
}

impl BuildingCounts {
    #[cfg(test)]
    pub const fn get(self, kind: BuildingKind) -> u16 {
        match kind {
            BuildingKind::House => self.houses,
            BuildingKind::Granary => self.granaries,
            BuildingKind::Workshop => self.workshops,
            BuildingKind::Market => self.markets,
            BuildingKind::Wall => self.walls,
        }
    }

    fn add(&mut self, kind: BuildingKind) {
        let count = match kind {
            BuildingKind::House => &mut self.houses,
            BuildingKind::Granary => &mut self.granaries,
            BuildingKind::Workshop => &mut self.workshops,
            BuildingKind::Market => &mut self.markets,
            BuildingKind::Wall => &mut self.walls,
        };
        *count = count.saturating_add(1);
    }
}

/// Counts every surviving building owned by `clan_id`, including construction sites.
pub fn building_counts(buildings: &[Building], clan_id: i32) -> BuildingCounts {
    counts_matching(buildings, clan_id, |building| !building.is_destroyed())
}

/// Counts only completed, surviving buildings whose effects may be applied.
pub fn active_building_counts(buildings: &[Building], clan_id: i32) -> BuildingCounts {
    counts_matching(buildings, clan_id, Building::is_active)
}

/// Fixed score from completed, surviving buildings. It is independent of vector order.
pub fn development_score(buildings: &[Building], clan_id: i32) -> u32 {
    buildings
        .iter()
        .filter(|building| building.clan_id == clan_id && building.is_active())
        .map(|building| building.kind.development_value())
        .sum()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TechState {
    pub level: u8,
    pub research: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SettlementStats {
    pub construction_work: u32,
    pub buildings_completed: u32,
    pub research_ticks: u32,
    pub tech_levels_gained: u32,
    pub granary_food_stored: u32,
    pub shelter_healing_milli: u64,
    pub market_material_delivered: u32,
    pub wall_damage_prevented_milli: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClanSettlement {
    pub clan_id: i32,
    pub tech: TechState,
    pub build_target: Option<BuildingId>,
    pub stats: SettlementStats,
}

impl TechState {
    pub const fn can_build(self, kind: BuildingKind) -> bool {
        self.level >= kind.unlock_level()
    }

    pub const fn next_level_cost(self) -> Option<u32> {
        research_cost_for_level(self.level.saturating_add(1))
    }

    /// Applies research, carrying overflow across levels, and returns levels gained.
    pub fn add_research(&mut self, amount: u32) -> u8 {
        self.level = self.level.min(MAX_TECH_LEVEL);
        if self.level == MAX_TECH_LEVEL {
            self.research = 0;
            return 0;
        }

        self.research = self.research.saturating_add(amount);
        let starting_level = self.level;
        while let Some(cost) = self.next_level_cost() {
            if self.research < cost {
                break;
            }
            self.research -= cost;
            self.level += 1;
        }
        if self.level == MAX_TECH_LEVEL {
            self.research = 0;
        }
        self.level - starting_level
    }
}

pub const fn research_cost_for_level(level: u8) -> Option<u32> {
    match level {
        1 => Some(40),
        2 => Some(90),
        3 => Some(160),
        _ => None,
    }
}

fn counts_matching(
    buildings: &[Building],
    clan_id: i32,
    include: impl Fn(&Building) -> bool,
) -> BuildingCounts {
    let mut counts = BuildingCounts::default();
    for building in buildings {
        if building.clan_id == clan_id && include(building) {
            counts.add(building.kind);
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_contracts_are_fixed_and_complete() {
        assert_eq!(BuildingKind::ALL.len(), 5);
        assert_eq!(BuildingKind::House.label(), "house");
        assert_eq!(
            BuildingKind::Market.cost(),
            BuildingCost { wood: 30, work: 60 }
        );
        assert_eq!(BuildingKind::Granary.unlock_level(), 0);
        assert_eq!(BuildingKind::Workshop.unlock_level(), 0);
        assert_eq!(BuildingKind::Market.unlock_level(), 2);
        assert!(BuildingKind::ALL
            .iter()
            .all(|kind| kind.cost().wood > 0 && kind.cost().work > 0));
    }

    #[test]
    fn construction_damage_and_repair_are_clamped() {
        let mut building = Building::new(BuildingId(7), 3, 9, 11, BuildingKind::House);
        assert_eq!(building.position(), (9, 11));
        assert_eq!(building.add_construction(10), 10);
        assert_eq!(building.completion_fraction(), 10.0 / 24.0);
        assert_eq!(building.add_construction(99), 14);
        assert!(building.is_active());
        assert_eq!(building.take_damage(500), 80);
        assert!(building.is_destroyed());
        assert_eq!(building.repair(15), 15);
        assert!(building.is_active());
        assert_eq!(building.repair(500), 65);
    }

    #[test]
    fn counts_and_development_ignore_order_other_clans_and_ruins() {
        let mut house = completed(1, 4, BuildingKind::House);
        let granary = completed(2, 4, BuildingKind::Granary);
        let workshop_site = Building::new(BuildingId(3), 4, 3, 3, BuildingKind::Workshop);
        let market = completed(4, 8, BuildingKind::Market);
        house.take_damage(house.kind.max_hp());
        let buildings = vec![market, workshop_site, granary, house];

        let all = building_counts(&buildings, 4);
        assert_eq!(all.granaries, 1);
        assert_eq!(all.get(BuildingKind::Granary), 1);
        assert_eq!(all.workshops, 1);
        assert_eq!(all.houses, 0);
        assert_eq!(active_building_counts(&buildings, 4).granaries, 1);
        assert_eq!(active_building_counts(&buildings, 4).workshops, 0);
        assert_eq!(development_score(&buildings, 4), 2);
        assert_eq!(development_score(&buildings, 8), 4);
    }

    #[test]
    fn technology_carries_over_and_stops_at_the_cap() {
        let mut tech = TechState::default();
        assert!(tech.can_build(BuildingKind::Granary));
        assert!(tech.can_build(BuildingKind::Workshop));

        assert_eq!(tech.add_research(50), 1);
        assert_eq!(
            tech,
            TechState {
                level: 1,
                research: 10
            }
        );
        assert!(tech.can_build(BuildingKind::Workshop));
        assert_eq!(tech.add_research(300), 2);
        assert_eq!(
            tech,
            TechState {
                level: 3,
                research: 0
            }
        );
        assert!(tech.can_build(BuildingKind::Market));
        assert_eq!(tech.next_level_cost(), None);
        assert_eq!(tech.add_research(u32::MAX), 0);
    }

    #[test]
    fn settlement_state_round_trips_through_bincode() {
        let state = (
            completed(42, 9, BuildingKind::Wall),
            TechState {
                level: 2,
                research: 37,
            },
        );
        let encoded = bincode::serialize(&state).expect("serialize settlement state");
        let decoded: (Building, TechState) =
            bincode::deserialize(&encoded).expect("deserialize settlement state");
        assert_eq!(decoded, state);
    }

    fn completed(id: u32, clan_id: i32, kind: BuildingKind) -> Building {
        let mut building = Building::new(BuildingId(id), clan_id, id as i32, 2, kind);
        building.add_construction(kind.cost().work);
        building
    }
}
