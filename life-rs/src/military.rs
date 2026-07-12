//! Deterministic ore and equipment accounting for resource-backed military production.
//!
//! This module owns pure data and clamped transitions only. `World` will own
//! physical movement, survival gates, extraction cadence, combat, and storage
//! so military state can remain additive to the fixed `LFB1` brain contract.

pub const MAX_ORE_PER_DEPOSIT: u16 = 240;
pub const MAX_CARRIED_ORE: u16 = 8;
pub const COMBAT_SCALE_MILLI: u16 = 1_000;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct OreDepositId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OreDeposit {
    pub id: OreDepositId,
    pub x: i32,
    pub y: i32,
    pub remaining: u16,
}

impl OreDeposit {
    pub fn new(id: OreDepositId, x: i32, y: i32, ore: u16) -> Self {
        Self {
            id,
            x,
            y,
            remaining: ore.min(MAX_ORE_PER_DEPOSIT),
        }
    }

    pub const fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub const fn is_depleted(&self) -> bool {
        self.remaining == 0
    }

    /// Removes at most the requested amount and returns the amount extracted.
    pub fn extract(&mut self, requested: u16) -> u16 {
        let extracted = self.remaining.min(requested);
        self.remaining -= extracted;
        extracted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntityOreCargo {
    pub entity_id: u32,
    pub ore: u16,
}

impl EntityOreCargo {
    pub const fn new(entity_id: u32) -> Self {
        Self { entity_id, ore: 0 }
    }

    /// Adds ore up to the physical carry limit and returns the amount accepted.
    pub fn add(&mut self, amount: u16) -> u16 {
        self.ore = self.ore.min(MAX_CARRIED_ORE);
        let accepted = amount.min(MAX_CARRIED_ORE - self.ore);
        self.ore += accepted;
        accepted
    }

    /// Removes at most the requested amount and returns the amount removed.
    pub fn take(&mut self, amount: u16) -> u16 {
        self.ore = self.ore.min(MAX_CARRIED_ORE);
        let removed = self.ore.min(amount);
        self.ore -= removed;
        removed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EquipmentSlot {
    Weapon,
    Armor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EquipmentKind {
    Spear,
    Sword,
    Armor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EquipmentRecipe {
    pub ore: i32,
    pub wood: i32,
    pub work: u16,
    pub unlock_tech: u8,
}

impl EquipmentKind {
    #[cfg(test)]
    pub const ALL: [EquipmentKind; 3] = [
        EquipmentKind::Spear,
        EquipmentKind::Sword,
        EquipmentKind::Armor,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            EquipmentKind::Spear => "spear",
            EquipmentKind::Sword => "sword",
            EquipmentKind::Armor => "armor",
        }
    }

    pub const fn slot(self) -> EquipmentSlot {
        match self {
            EquipmentKind::Spear | EquipmentKind::Sword => EquipmentSlot::Weapon,
            EquipmentKind::Armor => EquipmentSlot::Armor,
        }
    }

    /// Recipes consume no food. The caller supplies only wood that is safe to spend.
    pub const fn recipe(self) -> EquipmentRecipe {
        match self {
            EquipmentKind::Spear => EquipmentRecipe {
                ore: 2,
                wood: 4,
                work: 16,
                unlock_tech: 0,
            },
            EquipmentKind::Sword => EquipmentRecipe {
                ore: 5,
                wood: 2,
                work: 24,
                unlock_tech: 1,
            },
            EquipmentKind::Armor => EquipmentRecipe {
                ore: 8,
                wood: 2,
                work: 36,
                unlock_tech: 2,
            },
        }
    }

    pub const fn attack_bonus_milli(self) -> u16 {
        match self {
            EquipmentKind::Spear => 250,
            EquipmentKind::Sword => 450,
            EquipmentKind::Armor => 0,
        }
    }

    pub const fn protection_milli(self) -> u16 {
        match self {
            EquipmentKind::Armor => 250,
            EquipmentKind::Spear | EquipmentKind::Sword => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntityEquipment {
    pub entity_id: u32,
    pub weapon: Option<EquipmentKind>,
    pub armor: Option<EquipmentKind>,
}

impl EntityEquipment {
    pub const fn new(entity_id: u32) -> Self {
        Self {
            entity_id,
            weapon: None,
            armor: None,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.entity_id != 0
            && self
                .weapon
                .is_none_or(|kind| kind.slot() == EquipmentSlot::Weapon)
            && self
                .armor
                .is_none_or(|kind| kind.slot() == EquipmentSlot::Armor)
    }

    /// Equips one physical item and returns the replaced item, if any.
    pub fn equip(&mut self, kind: EquipmentKind) -> Option<EquipmentKind> {
        match kind.slot() {
            EquipmentSlot::Weapon => self.weapon.replace(kind),
            EquipmentSlot::Armor => self.armor.replace(kind),
        }
    }

    pub fn attack_bonus_milli(&self) -> u16 {
        self.weapon
            .map(EquipmentKind::attack_bonus_milli)
            .unwrap_or(0)
    }

    pub fn protection_milli(&self) -> u16 {
        self.armor
            .map(EquipmentKind::protection_milli)
            .unwrap_or(0)
            .min(COMBAT_SCALE_MILLI)
    }

    pub fn strength_milli(&self) -> u16 {
        COMBAT_SCALE_MILLI
            .saturating_add(self.attack_bonus_milli())
            .saturating_add(self.protection_milli())
    }

    /// Applies armor to finite positive damage and clamps malformed input to zero.
    pub fn protected_damage(&self, incoming: f32) -> f32 {
        if !incoming.is_finite() || incoming <= 0.0 {
            return 0.0;
        }
        let retained = COMBAT_SCALE_MILLI - self.protection_milli();
        incoming * retained as f32 / COMBAT_SCALE_MILLI as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EquipmentProject {
    pub recipient_entity_id: u32,
    pub kind: EquipmentKind,
    pub work: u16,
}

impl EquipmentProject {
    pub const fn new(recipient_entity_id: u32, kind: EquipmentKind) -> Self {
        Self {
            recipient_entity_id,
            kind,
            work: 0,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.work >= self.kind.recipe().work
    }

    pub fn completion_fraction(&self) -> f32 {
        let required = self.kind.recipe().work.max(1);
        self.work.min(required) as f32 / required as f32
    }

    /// Applies production work and returns the amount actually consumed.
    pub fn add_work(&mut self, amount: u16) -> u16 {
        let remaining = self.kind.recipe().work.saturating_sub(self.work);
        let consumed = remaining.min(amount);
        self.work += consumed;
        consumed
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProducedEquipment {
    pub recipient_entity_id: u32,
    pub kind: EquipmentKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProductionAdvance {
    pub work_applied: u16,
    pub completed: Option<ProducedEquipment>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MilitaryStats {
    pub ore_extracted: u32,
    pub ore_delivered: u32,
    pub production_work: u32,
    pub equipment_completed: u32,
    pub equipment_equipped: u32,
    pub bonus_damage_milli: u64,
    pub damage_prevented_milli: u64,
    pub equipment_lost: u32,
    pub equipped_member_ticks: u64,
    pub unsafe_work_ticks: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClanMilitary {
    pub clan_id: i32,
    pub ore_stockpile: i32,
    pub miner_entity_id: Option<u32>,
    pub project: Option<EquipmentProject>,
    pub stats: MilitaryStats,
}

impl ClanMilitary {
    pub const fn new(clan_id: i32) -> Self {
        Self {
            clan_id,
            ore_stockpile: 0,
            miner_entity_id: None,
            project: None,
            stats: MilitaryStats {
                ore_extracted: 0,
                ore_delivered: 0,
                production_work: 0,
                equipment_completed: 0,
                equipment_equipped: 0,
                bonus_damage_milli: 0,
                damage_prevented_milli: 0,
                equipment_lost: 0,
                equipped_member_ticks: 0,
                unsafe_work_ticks: 0,
            },
        }
    }

    /// Adds a positive delivery without allowing the stockpile to overflow.
    pub fn deliver_ore(&mut self, amount: i32) -> i32 {
        self.ore_stockpile = self.ore_stockpile.max(0);
        let delivered = amount.max(0).min(i32::MAX - self.ore_stockpile);
        self.ore_stockpile += delivered;
        self.stats.ore_delivered = self.stats.ore_delivered.saturating_add(delivered as u32);
        delivered
    }

    pub fn record_extraction(&mut self, amount: u16) {
        self.stats.ore_extracted = self.stats.ore_extracted.saturating_add(amount as u32);
    }

    /// Starts one project and reserves ore. `spendable_wood` must already exclude food and
    /// settlement safety buffers; the returned recipe tells `World` exactly what to deduct.
    pub fn begin_project(
        &mut self,
        recipient_entity_id: u32,
        kind: EquipmentKind,
        tech_level: u8,
        spendable_wood: i32,
    ) -> Option<EquipmentRecipe> {
        let recipe = kind.recipe();
        if recipient_entity_id == 0
            || self.project.is_some()
            || tech_level < recipe.unlock_tech
            || spendable_wood < recipe.wood
            || self.ore_stockpile.max(0) < recipe.ore
        {
            return None;
        }

        self.ore_stockpile = self.ore_stockpile.max(0) - recipe.ore;
        self.project = Some(EquipmentProject::new(recipient_entity_id, kind));
        Some(recipe)
    }

    /// Advances the active project, clamps excess work, and emits one owned item on completion.
    pub fn add_production_work(&mut self, amount: u16) -> ProductionAdvance {
        let Some(project) = self.project.as_mut() else {
            return ProductionAdvance::default();
        };

        let work_applied = project.add_work(amount);
        self.stats.production_work = self
            .stats
            .production_work
            .saturating_add(work_applied as u32);
        if !project.is_complete() {
            return ProductionAdvance {
                work_applied,
                completed: None,
            };
        }

        let completed_project = self.project.take().expect("completed project exists");
        self.stats.equipment_completed = self.stats.equipment_completed.saturating_add(1);
        ProductionAdvance {
            work_applied,
            completed: Some(ProducedEquipment {
                recipient_entity_id: completed_project.recipient_entity_id,
                kind: completed_project.kind,
            }),
        }
    }

    pub fn record_equipped(&mut self) {
        self.stats.equipment_equipped = self.stats.equipment_equipped.saturating_add(1);
    }

    pub fn record_bonus_damage(&mut self, amount: f32) {
        self.stats.bonus_damage_milli = self
            .stats
            .bonus_damage_milli
            .saturating_add(to_nonnegative_milli(amount));
    }

    pub fn record_damage_prevented(&mut self, amount: f32) {
        self.stats.damage_prevented_milli = self
            .stats
            .damage_prevented_milli
            .saturating_add(to_nonnegative_milli(amount));
    }

    pub fn record_equipment_lost(&mut self, count: u32) {
        self.stats.equipment_lost = self.stats.equipment_lost.saturating_add(count);
    }

    pub fn record_equipped_member_ticks(&mut self, count: u64) {
        self.stats.equipped_member_ticks = self.stats.equipped_member_ticks.saturating_add(count);
    }

    pub fn record_unsafe_work_tick(&mut self) {
        self.stats.unsafe_work_ticks = self.stats.unsafe_work_ticks.saturating_add(1);
    }
}

/// Finds carried ore in an entity-id-sorted cargo vector.
pub fn ore_cargo_for(cargo: &[EntityOreCargo], entity_id: u32) -> Option<&EntityOreCargo> {
    cargo
        .binary_search_by_key(&entity_id, |entry| entry.entity_id)
        .ok()
        .map(|index| &cargo[index])
}

/// Adds carried ore while retaining deterministic entity-id ordering.
pub fn add_entity_ore(cargo: &mut Vec<EntityOreCargo>, entity_id: u32, amount: u16) -> u16 {
    if entity_id == 0 || amount == 0 {
        return 0;
    }

    match cargo.binary_search_by_key(&entity_id, |entry| entry.entity_id) {
        Ok(index) => cargo[index].add(amount),
        Err(index) => {
            let mut entry = EntityOreCargo::new(entity_id);
            let accepted = entry.add(amount);
            cargo.insert(index, entry);
            accepted
        }
    }
}

/// Removes carried ore and drops empty cargo records from the sorted vector.
pub fn take_entity_ore(cargo: &mut Vec<EntityOreCargo>, entity_id: u32, amount: u16) -> u16 {
    let Ok(index) = cargo.binary_search_by_key(&entity_id, |entry| entry.entity_id) else {
        return 0;
    };
    let removed = cargo[index].take(amount);
    if cargo[index].ore == 0 {
        cargo.remove(index);
    }
    removed
}

/// Removes all ore carried by a disappearing entity.
pub fn remove_entity_ore_cargo(cargo: &mut Vec<EntityOreCargo>, entity_id: u32) -> u16 {
    let Ok(index) = cargo.binary_search_by_key(&entity_id, |entry| entry.entity_id) else {
        return 0;
    };
    cargo.remove(index).ore.min(MAX_CARRIED_ORE)
}

/// Finds equipment in an entity-id-sorted ownership vector.
pub fn equipment_for(equipment: &[EntityEquipment], entity_id: u32) -> Option<&EntityEquipment> {
    equipment
        .binary_search_by_key(&entity_id, |loadout| loadout.entity_id)
        .ok()
        .map(|index| &equipment[index])
}

/// Assigns a completed item while retaining deterministic entity-id ordering.
pub fn assign_equipment(
    equipment: &mut Vec<EntityEquipment>,
    produced: ProducedEquipment,
) -> Option<EquipmentKind> {
    if produced.recipient_entity_id == 0 {
        return None;
    }

    match equipment.binary_search_by_key(&produced.recipient_entity_id, |loadout| loadout.entity_id)
    {
        Ok(index) => equipment[index].equip(produced.kind),
        Err(index) => {
            let mut loadout = EntityEquipment::new(produced.recipient_entity_id);
            loadout.equip(produced.kind);
            equipment.insert(index, loadout);
            None
        }
    }
}

/// Removes and returns one entity's physical loadout from an id-sorted vector.
pub fn remove_entity_equipment(
    equipment: &mut Vec<EntityEquipment>,
    entity_id: u32,
) -> Option<EntityEquipment> {
    let index = equipment
        .binary_search_by_key(&entity_id, |loadout| loadout.entity_id)
        .ok()?;
    Some(equipment.remove(index))
}

fn to_nonnegative_milli(value: f32) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value as f64 * 1_000.0).min(u64::MAX as f64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ore_deposit_creation_and_extraction_are_clamped() {
        let mut deposit = OreDeposit::new(OreDepositId(4), 7, 9, u16::MAX);
        assert_eq!(deposit.position(), (7, 9));
        assert_eq!(deposit.remaining, MAX_ORE_PER_DEPOSIT);
        assert_eq!(deposit.extract(12), 12);
        assert_eq!(deposit.extract(u16::MAX), MAX_ORE_PER_DEPOSIT - 12);
        assert!(deposit.is_depleted());
        assert_eq!(deposit.extract(1), 0);
    }

    #[test]
    fn ore_cargo_helpers_are_clamped_sorted_and_remove_empty_records() {
        let mut cargo = Vec::new();
        assert_eq!(add_entity_ore(&mut cargo, 9, 3), 3);
        assert_eq!(add_entity_ore(&mut cargo, 2, u16::MAX), MAX_CARRIED_ORE);
        assert_eq!(add_entity_ore(&mut cargo, 9, u16::MAX), MAX_CARRIED_ORE - 3);
        assert_eq!(add_entity_ore(&mut cargo, 0, 4), 0);
        assert_eq!(
            cargo
                .iter()
                .map(|entry| entry.entity_id)
                .collect::<Vec<_>>(),
            vec![2, 9]
        );
        assert_eq!(ore_cargo_for(&cargo, 9).unwrap().ore, MAX_CARRIED_ORE);

        assert_eq!(take_entity_ore(&mut cargo, 9, 3), 3);
        assert_eq!(
            take_entity_ore(&mut cargo, 9, u16::MAX),
            MAX_CARRIED_ORE - 3
        );
        assert!(ore_cargo_for(&cargo, 9).is_none());
        assert_eq!(remove_entity_ore_cargo(&mut cargo, 2), MAX_CARRIED_ORE);
        assert!(cargo.is_empty());
    }

    #[test]
    fn recipes_are_non_food_progression_with_a_bootstrap_weapon() {
        assert_eq!(EquipmentKind::ALL.len(), 3);
        assert_eq!(EquipmentKind::Spear.label(), "spear");
        assert_eq!(EquipmentKind::Spear.recipe().unlock_tech, 0);
        assert_eq!(EquipmentKind::Sword.recipe().unlock_tech, 1);
        assert_eq!(EquipmentKind::Armor.recipe().unlock_tech, 2);
        assert!(EquipmentKind::ALL.iter().all(|kind| {
            let recipe = kind.recipe();
            recipe.ore > 0 && recipe.wood > 0 && recipe.work > 0
        }));
    }

    #[test]
    fn equipment_replaces_only_its_slot_and_clamps_damage() {
        let mut loadout = EntityEquipment::new(8);
        assert_eq!(loadout.equip(EquipmentKind::Spear), None);
        assert_eq!(loadout.equip(EquipmentKind::Armor), None);
        assert_eq!(
            loadout.equip(EquipmentKind::Sword),
            Some(EquipmentKind::Spear)
        );
        assert_eq!(loadout.weapon, Some(EquipmentKind::Sword));
        assert_eq!(loadout.armor, Some(EquipmentKind::Armor));
        assert_eq!(loadout.strength_milli(), 1_700);
        assert_eq!(loadout.protected_damage(4.0), 3.0);
        assert_eq!(loadout.protected_damage(f32::NAN), 0.0);
        assert_eq!(loadout.protected_damage(-4.0), 0.0);
        assert!(loadout.is_valid());
    }

    #[test]
    fn project_start_is_atomic_and_uses_only_declared_spendable_wood() {
        let mut military = ClanMilitary::new(3);
        assert_eq!(military.deliver_ore(10), 10);
        assert_eq!(military.deliver_ore(-5), 0);

        assert_eq!(
            military.begin_project(21, EquipmentKind::Armor, 1, 20),
            None
        );
        assert_eq!(military.ore_stockpile, 10);
        assert_eq!(military.begin_project(21, EquipmentKind::Spear, 0, 3), None);
        assert_eq!(military.ore_stockpile, 10);

        let recipe = military
            .begin_project(21, EquipmentKind::Spear, 0, 4)
            .expect("safe resource budget starts project");
        assert_eq!(recipe, EquipmentKind::Spear.recipe());
        assert_eq!(military.ore_stockpile, 8);
        assert!(military.project.is_some());
        assert_eq!(
            military.begin_project(22, EquipmentKind::Spear, 0, 99),
            None
        );
        assert_eq!(military.ore_stockpile, 8);
    }

    #[test]
    fn production_work_is_clamped_and_emits_exactly_one_item() {
        let mut military = ClanMilitary::new(6);
        military.deliver_ore(20);
        military
            .begin_project(31, EquipmentKind::Sword, 1, 10)
            .expect("start sword");

        let first = military.add_production_work(7);
        assert_eq!(first.work_applied, 7);
        assert_eq!(first.completed, None);
        assert_eq!(military.project.unwrap().completion_fraction(), 7.0 / 24.0);

        let final_work = military.add_production_work(u16::MAX);
        assert_eq!(final_work.work_applied, 17);
        assert_eq!(
            final_work.completed,
            Some(ProducedEquipment {
                recipient_entity_id: 31,
                kind: EquipmentKind::Sword,
            })
        );
        assert!(military.project.is_none());
        assert_eq!(military.stats.production_work, 24);
        assert_eq!(military.stats.equipment_completed, 1);
        assert_eq!(
            military.add_production_work(20),
            ProductionAdvance::default()
        );
    }

    #[test]
    fn ownership_helpers_keep_entity_ids_sorted_and_return_replaced_gear() {
        let mut equipment = Vec::new();
        assign_equipment(
            &mut equipment,
            ProducedEquipment {
                recipient_entity_id: 9,
                kind: EquipmentKind::Spear,
            },
        );
        assign_equipment(
            &mut equipment,
            ProducedEquipment {
                recipient_entity_id: 2,
                kind: EquipmentKind::Armor,
            },
        );
        assert_eq!(
            equipment
                .iter()
                .map(|gear| gear.entity_id)
                .collect::<Vec<_>>(),
            vec![2, 9]
        );
        assert_eq!(
            equipment_for(&equipment, 9).unwrap().weapon,
            Some(EquipmentKind::Spear)
        );
        assert_eq!(
            assign_equipment(
                &mut equipment,
                ProducedEquipment {
                    recipient_entity_id: 9,
                    kind: EquipmentKind::Sword,
                },
            ),
            Some(EquipmentKind::Spear)
        );
        assert!(equipment_for(&equipment, 0).is_none());
        assert_eq!(
            remove_entity_equipment(&mut equipment, 2)
                .unwrap()
                .entity_id,
            2
        );
        assert_eq!(equipment.len(), 1);
    }

    #[test]
    fn counters_saturate_and_ignore_non_finite_or_negative_damage() {
        let mut military = ClanMilitary::new(2);
        military.stats.ore_extracted = u32::MAX - 1;
        military.record_extraction(9);
        assert_eq!(military.stats.ore_extracted, u32::MAX);

        military.record_bonus_damage(1.25);
        military.record_bonus_damage(f32::INFINITY);
        military.record_damage_prevented(0.75);
        military.record_damage_prevented(-1.0);
        assert_eq!(military.stats.bonus_damage_milli, 1_250);
        assert_eq!(military.stats.damage_prevented_milli, 750);
    }

    #[test]
    fn military_state_round_trips_through_bincode() {
        let state = (
            OreDeposit::new(OreDepositId(3), 4, 5, 80),
            EntityOreCargo {
                entity_id: 12,
                ore: 5,
            },
            EntityEquipment {
                entity_id: 12,
                weapon: Some(EquipmentKind::Sword),
                armor: Some(EquipmentKind::Armor),
            },
            ClanMilitary {
                clan_id: 4,
                ore_stockpile: 17,
                miner_entity_id: Some(11),
                project: Some(EquipmentProject {
                    recipient_entity_id: 12,
                    kind: EquipmentKind::Armor,
                    work: 9,
                }),
                stats: MilitaryStats::default(),
            },
        );
        let encoded = bincode::serialize(&state).expect("serialize military state");
        let decoded: (OreDeposit, EntityOreCargo, EntityEquipment, ClanMilitary) =
            bincode::deserialize(&encoded).expect("deserialize military state");
        assert_eq!(decoded, state);
    }
}
