//! Versioned full-world persistence.
//!
//! `LFB1` remains the independent deployable-brain format. A world file owns a
//! complete `WorldSnapshotV1` payload behind a small checksummed envelope. Any
//! incompatible field-layout change must introduce a new envelope version and
//! an explicit migration instead of silently reinterpreting V1 bytes.

use super::{Params, Tree, World};
use crate::brain::Brain;
use crate::clan::Clan;
use crate::diplomacy::DiplomacyLedger;
use crate::entity::Entity;
use crate::grid::{terrain, Grid, NO_OWNER};
use crate::military::{
    ClanMilitary, EntityEquipment, EntityOreCargo, OreDeposit, MAX_CARRIED_ORE, MAX_ORE_PER_DEPOSIT,
};
use crate::rng::Rng;
use crate::settlement::{Building, ClanSettlement, MAX_TECH_LEVEL};
use bincode::Options;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Error, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"LIFEWRLD";
const VERSION_V1: u16 = 1;
const VERSION_V2: u16 = 2;
const VERSION_V3: u16 = 3;
const HEADER_LEN: usize = 24;
const MAX_PAYLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_GRID_SIZE: i32 = 4096;

#[derive(Serialize, Deserialize)]
struct WorldSnapshotV1 {
    grid: Grid,
    tick: i32,
    entities: Vec<Entity>,
    trees: Vec<Tree>,
    clans: Vec<Clan>,
    rng: Rng,
    params: Params,
    diplomacy: DiplomacyLedger,
    next_entity_id: u32,
    next_clan_id: i32,
    pellet_total: u64,
    deaths_starved: u64,
    deaths_combat: u64,
    births: u64,
    maintain_pop: i32,
    maintain_clans: i32,
    champion: Option<Brain>,
    disaster_level: f32,
}

#[derive(Serialize, Deserialize)]
struct WorldSnapshotV2 {
    base: WorldSnapshotV1,
    buildings: Vec<Building>,
    building_cells: Vec<u32>,
    settlements: Vec<ClanSettlement>,
    community_settlement: bool,
    next_building_id: u32,
}

#[derive(Serialize, Deserialize)]
struct WorldSnapshotV3 {
    base: WorldSnapshotV2,
    ore_deposits: Vec<OreDeposit>,
    ore_cargo: Vec<EntityOreCargo>,
    equipment: Vec<EntityEquipment>,
    militaries: Vec<ClanMilitary>,
    community_military: bool,
    next_ore_deposit_id: u32,
}

impl World {
    /// Atomically writes a validated, versioned snapshot of every persistent
    /// world field. Scratch flood-fill and occupancy buffers are rebuilt after
    /// load and intentionally do not appear in the wire format.
    pub fn save_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let snapshot = WorldSnapshotV3::capture(self);
        snapshot.validate()?;
        let payload = encode_snapshot(&snapshot)?;
        write_envelope(path.as_ref(), VERSION_V3, &payload)
    }

    /// Loads and validates a full-world snapshot. The returned world owns the
    /// exact saved RNG state, so continuing it produces the same future ticks.
    pub fn load_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let (version, payload) = read_envelope(path.as_ref())?;
        match version {
            VERSION_V1 => {
                let snapshot: WorldSnapshotV1 = decode_snapshot(&payload, "V1")?;
                snapshot.validate()?;
                Ok(snapshot.restore())
            }
            VERSION_V2 => {
                let snapshot: WorldSnapshotV2 = decode_snapshot(&payload, "V2")?;
                snapshot.validate()?;
                Ok(snapshot.restore())
            }
            VERSION_V3 => {
                let snapshot: WorldSnapshotV3 = decode_snapshot(&payload, "V3")?;
                snapshot.validate()?;
                Ok(snapshot.restore())
            }
            _ => Err(invalid(format!("unsupported world version {version}"))),
        }
    }
}

impl WorldSnapshotV1 {
    fn capture(world: &World) -> Self {
        Self {
            grid: world.grid.clone(),
            tick: world.tick,
            entities: world.entities.clone(),
            trees: world.trees.clone(),
            clans: world.clans.clone(),
            rng: world.rng.clone(),
            params: world.params.clone(),
            diplomacy: world.diplomacy.clone(),
            next_entity_id: world.next_entity_id,
            next_clan_id: world.next_clan_id,
            pellet_total: world.pellet_total as u64,
            deaths_starved: world.deaths_starved,
            deaths_combat: world.deaths_combat,
            births: world.births,
            maintain_pop: world.maintain_pop,
            maintain_clans: world.maintain_clans,
            champion: world.champion.clone(),
            disaster_level: world.disaster_level,
        }
    }

    fn restore(self) -> World {
        let cell_count = self.grid.terrain.len();
        let mut world = World {
            grid: self.grid,
            tick: self.tick,
            entities: self.entities,
            trees: self.trees,
            clans: self.clans,
            rng: self.rng,
            params: self.params,
            diplomacy: self.diplomacy,
            buildings: Vec::new(),
            building_cells: vec![0; cell_count],
            settlements: Vec::new(),
            community_settlement: true,
            ore_deposits: Vec::new(),
            ore_cargo: Vec::new(),
            militaries: Vec::new(),
            equipment: Vec::new(),
            community_military: true,
            next_entity_id: self.next_entity_id,
            next_clan_id: self.next_clan_id,
            next_building_id: 1,
            next_ore_deposit_id: 1,
            pellet_total: self.pellet_total as usize,
            deaths_starved: self.deaths_starved,
            deaths_combat: self.deaths_combat,
            births: self.births,
            maintain_pop: self.maintain_pop,
            maintain_clans: self.maintain_clans,
            champion: self.champion,
            disaster_level: self.disaster_level,
            reach: Vec::new(),
            occupied: Vec::new(),
        };
        world.initialize_military_resources();
        world
    }

    fn validate(&self) -> io::Result<()> {
        self.validate_grid()?;
        self.validate_params()?;
        if self.tick < 0 || self.maintain_pop < 0 || self.maintain_clans < 0 {
            return Err(invalid("negative world counter"));
        }
        if !self.rng.has_valid_state() {
            return Err(invalid("xoshiro state may not be all zero"));
        }
        if !finite_between(self.disaster_level, 0.0, 1.0) {
            return Err(invalid("invalid disaster level"));
        }
        if self
            .champion
            .as_ref()
            .is_some_and(|brain| !brain.has_valid_persistent_state())
        {
            return Err(invalid("invalid champion brain"));
        }
        if self.trees.iter().any(|tree| {
            !point_in_bounds((tree.x, tree.y), self.grid.size) || tree.last_spawn > self.tick
        }) {
            return Err(invalid("invalid tree state"));
        }
        self.validate_entities_and_clans()?;
        self.validate_diplomacy()?;
        Ok(())
    }

    fn validate_grid(&self) -> io::Result<()> {
        let size = self.grid.size;
        if !(1..=MAX_GRID_SIZE).contains(&size) {
            return Err(invalid("grid size outside supported range"));
        }
        let cells = (size as usize)
            .checked_mul(size as usize)
            .ok_or_else(|| invalid("grid dimensions overflow"))?;
        let lengths = [
            self.grid.terrain.len(),
            self.grid.fertility.len(),
            self.grid.owner.len(),
            self.grid.road.len(),
            self.grid.wood.len(),
            self.grid.traffic.len(),
            self.grid.pellet.len(),
            self.grid.depletion.len(),
        ];
        if lengths.iter().any(|&length| length != cells) {
            return Err(invalid("grid layer length mismatch"));
        }
        if self.grid.terrain.iter().any(|&tile| tile > terrain::SAND) {
            return Err(invalid("unknown terrain tag"));
        }
        let pellets = self
            .grid
            .pellet
            .iter()
            .filter(|&&amount| amount != 0)
            .count() as u64;
        if pellets != self.pellet_total {
            return Err(invalid("pellet counter does not match grid"));
        }
        Ok(())
    }

    fn validate_params(&self) -> io::Result<()> {
        let p = &self.params;
        let nonnegative = [
            p.tree_interval,
            p.tree_per_cycle,
            p.tree_radius,
            p.pellet_energy,
            p.starve_ticks,
            p.vision_radius,
            p.carry_limit,
            p.attack_cooldown,
            p.clan_grace_ticks,
            p.recruit_radius,
            p.claim_interval,
            p.members_per_claim,
            p.farm_interval,
            p.home_range,
            p.expand_claim_radius,
            p.season_length,
            p.birth_interval,
            p.birth_food_cost,
        ];
        if nonnegative.iter().any(|&value| value < 0) || p.starve_ticks == 0 {
            return Err(invalid("invalid negative or zero simulation parameter"));
        }
        let nonnegative_floats = [
            p.starve_damage,
            p.heal_rate,
            p.base_health,
            p.leader_health,
            p.min_speed,
            p.max_speed,
            p.hunger_min,
            p.hunger_max,
            p.leader_chance,
            p.max_pellet_fraction,
            p.attack_damage,
            p.war_threshold,
            p.farm_yield,
            p.season_amp,
            p.soil_depletion_rate,
            p.disaster_rate,
            p.birth_chance,
            p.water_level,
            p.mountain_level,
        ];
        if nonnegative_floats
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(invalid("invalid floating-point simulation parameter"));
        }
        Ok(())
    }

    fn validate_entities_and_clans(&self) -> io::Result<()> {
        let size = self.grid.size;
        let mut entity_ids = HashSet::with_capacity(self.entities.len());
        let mut entity_by_id = HashMap::with_capacity(self.entities.len());
        for (index, entity) in self.entities.iter().enumerate() {
            if entity.id == 0 || !entity_ids.insert(entity.id) {
                return Err(invalid("duplicate or zero entity id"));
            }
            entity_by_id.insert(entity.id, index);
            validate_entity(entity, size)?;
        }
        if self.next_entity_id == 0
            || entity_ids
                .iter()
                .max()
                .is_some_and(|&id| self.next_entity_id <= id)
        {
            return Err(invalid("next entity id is not ahead of live ids"));
        }

        let mut clan_ids = HashSet::with_capacity(self.clans.len());
        for clan in &self.clans {
            if clan.id < 0 || !clan_ids.insert(clan.id) {
                return Err(invalid("duplicate or negative clan id"));
            }
            validate_clan(clan, size)?;
        }
        if self.next_clan_id < 1
            || clan_ids
                .iter()
                .max()
                .is_some_and(|&id| self.next_clan_id <= id)
        {
            return Err(invalid("next clan id is not ahead of live ids"));
        }
        if self
            .grid
            .owner
            .iter()
            .any(|owner| *owner != NO_OWNER && !clan_ids.contains(owner))
        {
            return Err(invalid("grid references an unknown clan"));
        }

        let mut roster = HashSet::new();
        for clan in &self.clans {
            let leader = entity_by_id
                .get(&clan.leader_id)
                .map(|&index| &self.entities[index])
                .ok_or_else(|| invalid("clan leader reference is missing"))?;
            if leader.clan != clan.id || !leader.is_leader || !roster.insert(leader.id) {
                return Err(invalid("invalid or duplicated clan leader"));
            }
            for member_id in &clan.members {
                let member = entity_by_id
                    .get(member_id)
                    .map(|&index| &self.entities[index])
                    .ok_or_else(|| invalid("clan member reference is missing"))?;
                if member.clan != clan.id || member.is_leader || !roster.insert(*member_id) {
                    return Err(invalid("invalid or duplicated clan member"));
                }
            }
            if clan.trade_partner.is_some_and(|partner| {
                partner < 1 || partner >= self.next_clan_id || partner == clan.id
            }) {
                return Err(invalid("invalid clan trade partner cache"));
            }
            for reference in [clan.recruit_target, clan.trade_route_threat] {
                if reference.is_some_and(|id| id == 0 || id >= self.next_entity_id) {
                    return Err(invalid("invalid clan entity cache"));
                }
            }
        }
        for entity in &self.entities {
            if entity.clan >= 0
                && (!clan_ids.contains(&entity.clan) || !roster.contains(&entity.id))
            {
                return Err(invalid("entity clan ownership is inconsistent"));
            }
            if entity.trade_target_clan >= 0
                && (entity.trade_target_clan == entity.clan
                    || entity.trade_target_clan < 1
                    || entity.trade_target_clan >= self.next_clan_id)
            {
                return Err(invalid("invalid entity trade target"));
            }
            for reference in [entity.rescue_target, entity.carried_by] {
                if reference
                    .is_some_and(|id| id == 0 || id == entity.id || id >= self.next_entity_id)
                {
                    return Err(invalid("invalid entity care reference"));
                }
            }
        }
        Ok(())
    }

    fn validate_diplomacy(&self) -> io::Result<()> {
        let mut previous = None;
        for relationship in self.diplomacy.relationships() {
            let key = (relationship.clan_low, relationship.clan_high);
            if relationship.clan_low >= relationship.clan_high
                || previous.is_some_and(|prior| prior >= key)
                || relationship.clan_low < 1
                || relationship.clan_high >= self.next_clan_id
                || !finite_between(relationship.trust, -1.0, 1.0)
                || !relationship.recent_food_delivered.is_finite()
                || relationship.recent_food_delivered < 0.0
                || !relationship.recent_wood_delivered.is_finite()
                || relationship.recent_wood_delivered < 0.0
            {
                return Err(invalid("invalid diplomacy relationship"));
            }
            previous = Some(key);
        }
        Ok(())
    }
}

impl WorldSnapshotV2 {
    fn capture(world: &World) -> Self {
        Self {
            base: WorldSnapshotV1::capture(world),
            buildings: world.buildings.clone(),
            building_cells: world.building_cells.clone(),
            settlements: world.settlements.clone(),
            community_settlement: world.community_settlement,
            next_building_id: world.next_building_id,
        }
    }

    fn restore(self) -> World {
        let mut world = self.base.restore();
        world.buildings = self.buildings;
        world.building_cells = self.building_cells;
        world.settlements = self.settlements;
        world.community_settlement = self.community_settlement;
        world.next_building_id = self.next_building_id;
        world.initialize_military_resources();
        world
    }

    fn validate(&self) -> io::Result<()> {
        self.base.validate()?;
        let cells = self.base.grid.terrain.len();
        if self.building_cells.len() != cells {
            return Err(invalid("building layer length mismatch"));
        }
        let clan_ids: HashSet<i32> = self.base.clans.iter().map(|clan| clan.id).collect();
        let mut ids = HashSet::with_capacity(self.buildings.len());
        for building in &self.buildings {
            if building.id.0 == 0
                || !ids.insert(building.id.0)
                || building.id.0 >= self.next_building_id
                || !clan_ids.contains(&building.clan_id)
                || !point_in_bounds(building.position(), self.base.grid.size)
                || building.construction > building.kind.cost().work
                || building.hp > building.kind.max_hp()
                || self.building_cells[self.base.grid.idx(building.x, building.y)] != building.id.0
            {
                return Err(invalid("invalid building state"));
            }
        }
        if self.next_building_id == 0
            || ids
                .iter()
                .max()
                .is_some_and(|&id| self.next_building_id <= id)
        {
            return Err(invalid("next building id is not ahead of live ids"));
        }
        for (cell, &id) in self.building_cells.iter().enumerate() {
            if id == 0 {
                continue;
            }
            let Some(building) = self.buildings.iter().find(|building| building.id.0 == id) else {
                return Err(invalid("building layer references a missing building"));
            };
            if self.base.grid.idx(building.x, building.y) != cell {
                return Err(invalid("building footprint does not match its position"));
            }
        }
        let mut previous = None;
        let mut targeted_buildings = HashSet::new();
        for state in &self.settlements {
            if state.clan_id < 1
                || !clan_ids.contains(&state.clan_id)
                || previous.is_some_and(|prior| prior >= state.clan_id)
                || state.tech.level > MAX_TECH_LEVEL
                || state
                    .tech
                    .next_level_cost()
                    .is_some_and(|cost| state.tech.research >= cost)
                || state.tech.level == MAX_TECH_LEVEL && state.tech.research != 0
                || state.build_target.is_some_and(|id| {
                    id.0 == 0
                        || id.0 >= self.next_building_id
                        || !self.buildings.iter().any(|building| {
                            building.id == id
                                && building.clan_id == state.clan_id
                                && !building.is_complete()
                                && !building.is_destroyed()
                        })
                })
            {
                return Err(invalid("invalid clan settlement state"));
            }
            if let Some(target) = state.build_target {
                targeted_buildings.insert(target.0);
            }
            previous = Some(state.clan_id);
        }
        if self
            .buildings
            .iter()
            .filter(|building| !building.is_complete() && !building.is_destroyed())
            .any(|building| !targeted_buildings.contains(&building.id.0))
        {
            return Err(invalid("incomplete building has no active clan project"));
        }
        Ok(())
    }
}

impl WorldSnapshotV3 {
    fn capture(world: &World) -> Self {
        Self {
            base: WorldSnapshotV2::capture(world),
            ore_deposits: world.ore_deposits.clone(),
            ore_cargo: world.ore_cargo.clone(),
            equipment: world.equipment.clone(),
            militaries: world.militaries.clone(),
            community_military: world.community_military,
            next_ore_deposit_id: world.next_ore_deposit_id,
        }
    }

    fn restore(self) -> World {
        let mut world = self.base.restore();
        world.ore_deposits = self.ore_deposits;
        world.ore_cargo = self.ore_cargo;
        world.equipment = self.equipment;
        world.militaries = self.militaries;
        world.community_military = self.community_military;
        world.next_ore_deposit_id = self.next_ore_deposit_id;
        world
    }

    fn validate(&self) -> io::Result<()> {
        self.base.validate()?;
        let entity_clans: HashMap<u32, i32> = self
            .base
            .base
            .entities
            .iter()
            .filter(|entity| !entity.dead)
            .map(|entity| (entity.id, entity.clan))
            .collect();
        let clan_ids: HashSet<i32> = self
            .base
            .base
            .clans
            .iter()
            .filter(|clan| !clan.disbanded)
            .map(|clan| clan.id)
            .collect();

        let mut previous_deposit = 0;
        let mut positions = HashSet::new();
        for deposit in &self.ore_deposits {
            if deposit.id.0 == 0
                || deposit.id.0 <= previous_deposit
                || deposit.id.0 >= self.next_ore_deposit_id
                || deposit.remaining > MAX_ORE_PER_DEPOSIT
                || !point_in_bounds(deposit.position(), self.base.base.grid.size)
                || !positions.insert(deposit.position())
            {
                return Err(invalid("invalid ore deposit state"));
            }
            previous_deposit = deposit.id.0;
        }
        if self.next_ore_deposit_id == 0 {
            return Err(invalid("invalid next ore deposit id"));
        }

        let mut previous_entity = 0;
        for cargo in &self.ore_cargo {
            if cargo.entity_id == 0
                || cargo.entity_id <= previous_entity
                || cargo.ore == 0
                || cargo.ore > MAX_CARRIED_ORE
                || !entity_clans.contains_key(&cargo.entity_id)
            {
                return Err(invalid("invalid entity ore cargo"));
            }
            previous_entity = cargo.entity_id;
        }

        previous_entity = 0;
        for loadout in &self.equipment {
            if loadout.entity_id == 0
                || loadout.entity_id <= previous_entity
                || !loadout.is_valid()
                || loadout.weapon.is_none() && loadout.armor.is_none()
                || !entity_clans.contains_key(&loadout.entity_id)
            {
                return Err(invalid("invalid entity equipment"));
            }
            previous_entity = loadout.entity_id;
        }

        let tech_level = |clan_id| {
            self.base
                .settlements
                .iter()
                .find(|state| state.clan_id == clan_id)
                .map_or(0, |state| state.tech.level)
        };
        let mut previous_clan = 0;
        for state in &self.militaries {
            if state.clan_id <= previous_clan
                || !clan_ids.contains(&state.clan_id)
                || state.ore_stockpile < 0
                || state
                    .miner_entity_id
                    .is_some_and(|entity_id| entity_clans.get(&entity_id) != Some(&state.clan_id))
                || state.project.is_some_and(|project| {
                    project.recipient_entity_id == 0
                        || project.work >= project.kind.recipe().work
                        || tech_level(state.clan_id) < project.kind.recipe().unlock_tech
                        || entity_clans.get(&project.recipient_entity_id) != Some(&state.clan_id)
                })
            {
                return Err(invalid("invalid clan military state"));
            }
            previous_clan = state.clan_id;
        }
        Ok(())
    }
}

fn validate_entity(entity: &Entity, size: i32) -> io::Result<()> {
    if !point_in_bounds((entity.x, entity.y), size)
        || entity
            .last_food
            .is_some_and(|point| !point_in_bounds(point, size))
        || !entity.speed.is_finite()
        || entity.speed < 0.0
        || !entity.move_budget.is_finite()
        || !entity.health.is_finite()
        || entity.health < 0.0
        || !entity.max_health.is_finite()
        || entity.max_health <= 0.0
        || entity.health > entity.max_health
        || !finite_between(entity.hunger_threshold, 0.0, 1.0)
        || entity.food < 0
        || entity.wood < 0
        || entity.trade_food < 0
        || entity.trade_wood < 0
        || entity.attack_cooldown < 0
        || entity.incapacitated_until < 0
    {
        return Err(invalid(format!(
            "invalid entity state {}: pos=({},{}), speed={}, budget={}, health={}/{}, hunger={}, food={}, wood={}, trade={}/{}, hungry_ticks={}, cooldown={}, incapacitated_until={}",
            entity.id,
            entity.x,
            entity.y,
            entity.speed,
            entity.move_budget,
            entity.health,
            entity.max_health,
            entity.hunger_threshold,
            entity.food,
            entity.wood,
            entity.trade_food,
            entity.trade_wood,
            entity.ticks_since_food,
            entity.attack_cooldown,
            entity.incapacitated_until
        )));
    }
    Ok(())
}

fn validate_clan(clan: &Clan, size: i32) -> io::Result<()> {
    let cached_points = [
        clan.stockpile,
        clan.enemy_pos,
        clan.neutral_pos,
        clan.expand_target,
        clan.trespasser_pos,
    ];
    if cached_points
        .iter()
        .flatten()
        .any(|&point| !point_in_bounds(point, size))
        || !clan.brain.has_valid_persistent_state()
        || clan.food < 0
        || clan.wood < 0
        || clan.reserve_food < 0
        || !clan.fertile_capacity.is_finite()
        || clan.fertile_capacity < 0.0
        || !finite_between(clan.soil_depletion, 0.0, 1.0)
        || !finite_between(clan.aggression, 0.0, 1.0)
        || !clan.stats.hunger_tick_sum.is_finite()
        || clan.stats.hunger_tick_sum < 0.0
    {
        return Err(invalid("invalid clan state"));
    }
    Ok(())
}

fn encode_snapshot(snapshot: &impl Serialize) -> io::Result<Vec<u8>> {
    codec()
        .with_limit(MAX_PAYLOAD_BYTES)
        .serialize(snapshot)
        .map_err(|error| invalid(format!("could not encode world payload: {error}")))
}

fn decode_snapshot<'a, T: Deserialize<'a>>(payload: &'a [u8], label: &str) -> io::Result<T> {
    codec()
        .with_limit(MAX_PAYLOAD_BYTES)
        .deserialize(payload)
        .map_err(|error| invalid(format!("invalid {label} payload: {error}")))
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
        .reject_trailing_bytes()
}

fn write_envelope(path: &Path, version: u16, payload: &[u8]) -> io::Result<()> {
    if payload.len() as u64 > MAX_PAYLOAD_BYTES {
        return Err(invalid("world payload exceeds size limit"));
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    bytes.extend_from_slice(payload);

    let temp = temporary_path(path);
    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn read_envelope(path: &Path) -> io::Result<(u16, Vec<u8>)> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < HEADER_LEN as u64 {
        return Err(invalid("world file is truncated"));
    }
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    if &header[0..8] != MAGIC {
        return Err(invalid("not a LIFE world file"));
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if u16::from_le_bytes([header[10], header[11]]) != 0 {
        return Err(invalid("unsupported world envelope flags"));
    }
    let payload_len = u64::from_le_bytes(header[12..20].try_into().unwrap());
    if payload_len > MAX_PAYLOAD_BYTES || file_len != HEADER_LEN as u64 + payload_len {
        return Err(invalid("world payload length mismatch"));
    }
    let expected_checksum = u32::from_le_bytes(header[20..24].try_into().unwrap());
    let mut payload = vec![0u8; payload_len as usize];
    file.read_exact(&mut payload)?;
    if crc32fast::hash(&payload) != expected_checksum {
        return Err(invalid("world payload checksum mismatch"));
    }
    Ok((version, payload))
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

fn point_in_bounds((x, y): (i32, i32), size: i32) -> bool {
    x >= 0 && y >= 0 && x < size && y < size
}

fn finite_between(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

fn invalid(message: impl Into<String>) -> io::Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_FILE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn full_world_roundtrip_preserves_every_persistent_byte() {
        let path = test_path("roundtrip");
        let mut world = representative_world();
        let first_clan = world.clans[0].id;
        let second_clan = world.clans[1].id;
        world.diplomacy.adjust(first_clan, second_clan, 0.42);
        world
            .diplomacy
            .record_trade(first_clan, second_clan, 7, 3, world.tick);
        world.champion = Some(Brain::random(&mut world.rng));

        world.save_file(&path).unwrap();
        let loaded = World::load_file(&path).unwrap();

        assert_eq!(persistent_bytes(&world), persistent_bytes(&loaded));
        assert!(loaded.reach.is_empty());
        assert!(loaded.occupied.is_empty());
        cleanup(&path);
    }

    #[test]
    fn loaded_world_continues_identically_with_exact_rng_state() {
        let path = test_path("continuation");
        let mut original = representative_world();
        original.save_file(&path).unwrap();
        let mut loaded = World::load_file(&path).unwrap();

        for _ in 0..1000 {
            original.step();
            loaded.step();
        }

        assert_eq!(persistent_bytes(&original), persistent_bytes(&loaded));
        cleanup(&path);
    }

    #[test]
    fn active_care_and_trade_operations_continue_identically() {
        let path = test_path("active-operations");
        let mut original = representative_world();
        let first_clan = original.clans[0].id;
        let second_clan = original.clans[1].id;
        let mut first_members = original
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| (entity.clan == first_clan).then_some(index));
        let rescuer = first_members.next().unwrap();
        let patient = first_members.next().unwrap();
        let courier = original
            .entities
            .iter()
            .position(|entity| entity.clan == second_clan && !entity.is_leader)
            .unwrap();
        original.entities[patient].health = 0.0;
        original.entities[patient].incapacitated_until = original.tick + 120;
        original.entities[patient].carried_by = Some(original.entities[rescuer].id);
        original.entities[rescuer].rescue_target = Some(original.entities[patient].id);
        original.entities[rescuer].goal = crate::entity::Goal::Rescuing;
        original.entities[courier].trade_target_clan = first_clan;
        original.entities[courier].trade_food = 1;
        original.entities[courier].goal = crate::entity::Goal::Starving;
        original.grid.traffic[3] = 77;
        original.grid.depletion[4] = 91;
        original.disaster_level = 0.35;

        original.save_file(&path).unwrap();
        let mut loaded = World::load_file(&path).unwrap();
        for _ in 0..250 {
            original.step();
            loaded.step();
        }

        assert_eq!(persistent_bytes(&original), persistent_bytes(&loaded));
        cleanup(&path);
    }

    #[test]
    fn scratch_buffers_do_not_change_saved_bytes() {
        let mut world = representative_world();
        let baseline = persistent_bytes(&world);
        world.reach = vec![i32::MAX; 17];
        world.occupied = vec![u16::MAX; 23];
        assert_eq!(baseline, persistent_bytes(&world));
    }

    #[test]
    fn checksum_corruption_and_unknown_versions_are_rejected() {
        let path = test_path("corrupt");
        representative_world().save_file(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x80;
        std::fs::write(&path, &bytes).unwrap();
        let error = World::load_file(&path).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("checksum"));

        representative_world().save_file(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8..10].copy_from_slice(&4u16.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        let error = World::load_file(&path).err().unwrap();
        assert!(error.to_string().contains("unsupported world version"));
        cleanup(&path);
    }

    #[test]
    fn malformed_references_and_grid_layers_are_rejected() {
        let path = test_path("validation");
        let world = representative_world();
        let mut snapshot = WorldSnapshotV1::capture(&world);
        snapshot.next_entity_id = 1;
        write_envelope(&path, VERSION_V1, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("next entity id"));

        let mut snapshot = WorldSnapshotV1::capture(&world);
        snapshot.grid.owner.pop();
        write_envelope(&path, VERSION_V1, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("grid layer length"));
        cleanup(&path);
    }

    #[test]
    fn valid_stale_caches_and_runtime_normalized_params_can_be_saved() {
        let path = test_path("stale-caches");
        let mut snapshot = WorldSnapshotV1::capture(&representative_world());
        let historical_clan = snapshot.next_clan_id + 4;
        snapshot.next_clan_id = historical_clan + 1;
        snapshot.clans[0].trade_partner = Some(historical_clan);
        snapshot
            .diplomacy
            .adjust(snapshot.clans[0].id, historical_clan, 0.2);
        let historical_entity = snapshot.next_entity_id + 4;
        snapshot.next_entity_id = historical_entity + 1;
        snapshot.clans[0].trade_route_threat = Some(historical_entity);
        snapshot.clans[0].recruit_target = Some(historical_entity);
        snapshot.entities[0].rescue_target = Some(historical_entity);
        snapshot.entities[1].carried_by = Some(historical_entity);
        snapshot.params.max_speed = snapshot.params.min_speed * 0.5;
        snapshot.params.hunger_max = snapshot.params.hunger_min * 0.5;
        snapshot.params.water_level = 0.6;
        snapshot.params.mountain_level = 0.5;

        write_envelope(&path, VERSION_V1, &encode_snapshot(&snapshot).unwrap()).unwrap();
        World::load_file(&path).unwrap();
        cleanup(&path);
    }

    #[test]
    fn v1_world_migrates_to_empty_enabled_settlement_state() {
        let path = test_path("v1-migration");
        let original = representative_world();
        let snapshot = WorldSnapshotV1::capture(&original);
        write_envelope(&path, VERSION_V1, &encode_snapshot(&snapshot).unwrap()).unwrap();

        let loaded = World::load_file(&path).unwrap();

        assert!(loaded.community_settlement);
        assert!(loaded.buildings.is_empty());
        assert!(loaded.settlements.is_empty());
        assert!(loaded.building_cells.iter().all(|&id| id == 0));
        assert_eq!(loaded.next_building_id, 1);
        assert!(loaded.community_military);
        assert!(!loaded.ore_deposits.is_empty());
        cleanup(&path);
    }

    #[test]
    fn v2_world_migrates_to_enabled_military_with_reachable_deposits() {
        let path = test_path("v2-migration");
        let original = representative_world();
        let snapshot = WorldSnapshotV2::capture(&original);
        write_envelope(&path, VERSION_V2, &encode_snapshot(&snapshot).unwrap()).unwrap();

        let loaded = World::load_file(&path).unwrap();

        assert!(loaded.community_military);
        assert!(!loaded.ore_deposits.is_empty());
        for clan in &loaded.clans {
            let home = clan.stockpile.unwrap();
            assert!(loaded.ore_deposits.iter().any(|deposit| {
                !deposit.is_depleted()
                    && (deposit.x - home.0).abs().max((deposit.y - home.1).abs()) <= 10
                    && !loaded.is_foreign_tile(deposit.x, deposit.y, clan.id)
            }));
        }
        assert!(loaded.ore_cargo.is_empty());
        assert!(loaded.equipment.is_empty());
        assert!(loaded.militaries.is_empty());
        assert!(loaded.next_ore_deposit_id > 1);
        cleanup(&path);
    }

    #[test]
    fn v3_roundtrip_preserves_active_cargo_project_and_equipment() {
        let path = test_path("v3-military");
        let mut world = representative_world();
        let clan_id = world.clans[0].id;
        let entity_id = world
            .entities
            .iter()
            .find(|entity| entity.clan == clan_id)
            .unwrap()
            .id;
        crate::military::add_entity_ore(&mut world.ore_cargo, entity_id, 3);
        crate::military::assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: entity_id,
                kind: crate::military::EquipmentKind::Spear,
            },
        );
        let mut military = crate::military::ClanMilitary::new(clan_id);
        military.ore_stockpile = 9;
        military.miner_entity_id = Some(entity_id);
        military.project = Some(crate::military::EquipmentProject {
            recipient_entity_id: entity_id,
            kind: crate::military::EquipmentKind::Spear,
            work: 7,
        });
        military.stats.ore_delivered = 12;
        military.stats.equipped_member_ticks = 40;
        world.militaries = vec![military];

        world.save_file(&path).unwrap();
        let loaded = World::load_file(&path).unwrap();

        assert_eq!(persistent_bytes(&world), persistent_bytes(&loaded));
        cleanup(&path);
    }

    #[test]
    fn malformed_v3_military_references_are_rejected() {
        let path = test_path("v3-invalid-military");
        let world = representative_world();
        let mut snapshot = WorldSnapshotV3::capture(&world);
        snapshot.ore_cargo = vec![crate::military::EntityOreCargo {
            entity_id: snapshot.base.base.next_entity_id + 10,
            ore: 1,
        }];
        write_envelope(&path, VERSION_V3, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("invalid entity ore cargo"));
        cleanup(&path);
    }

    #[test]
    fn v2_roundtrip_preserves_active_construction_and_research() {
        let path = test_path("v2-settlement");
        let mut world = representative_world();
        world.buildings.clear();
        world.building_cells.fill(0);
        world.settlements.clear();
        world.next_building_id = 1;
        let clan_id = world.clans[0].id;
        let stockpile = world.clans[0].stockpile.unwrap();
        let cell = world.grid.idx(stockpile.0, stockpile.1);
        let mut building = crate::settlement::Building::new(
            crate::settlement::BuildingId(world.next_building_id),
            clan_id,
            stockpile.0,
            stockpile.1,
            crate::settlement::BuildingKind::Workshop,
        );
        building.add_construction(17);
        world.next_building_id += 1;
        world.building_cells[cell] = building.id.0;
        world.settlements.push(crate::settlement::ClanSettlement {
            clan_id,
            tech: crate::settlement::TechState {
                level: 1,
                research: 23,
            },
            build_target: Some(building.id),
            stats: crate::settlement::SettlementStats {
                construction_work: 17,
                research_ticks: 23,
                ..crate::settlement::SettlementStats::default()
            },
        });
        world.settlements.sort_by_key(|state| state.clan_id);
        world.buildings.push(building);

        world.save_file(&path).unwrap();
        let loaded = World::load_file(&path).unwrap();

        assert_eq!(persistent_bytes(&world), persistent_bytes(&loaded));
        cleanup(&path);
    }

    #[test]
    fn malformed_v2_settlement_references_are_rejected() {
        let path = test_path("v2-invalid-settlement");
        let world = representative_world();
        let mut snapshot = WorldSnapshotV2::capture(&world);
        snapshot.building_cells.pop();
        write_envelope(&path, VERSION_V2, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("building layer length"));

        let mut snapshot = WorldSnapshotV2::capture(&world);
        snapshot
            .settlements
            .push(crate::settlement::ClanSettlement {
                clan_id: world.clans[0].id,
                tech: crate::settlement::TechState {
                    level: MAX_TECH_LEVEL + 1,
                    research: 0,
                },
                ..crate::settlement::ClanSettlement::default()
            });
        write_envelope(&path, VERSION_V2, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("invalid clan settlement"));

        let mut snapshot = WorldSnapshotV2::capture(&world);
        snapshot.buildings.clear();
        snapshot.building_cells.fill(0);
        snapshot.settlements.clear();
        snapshot.next_building_id = 2;
        let clan_id = world.clans[0].id;
        let (x, y) = world.clans[0].stockpile.unwrap();
        let orphan = crate::settlement::Building::new(
            crate::settlement::BuildingId(1),
            clan_id,
            x,
            y,
            crate::settlement::BuildingKind::House,
        );
        snapshot.building_cells[world.grid.idx(x, y)] = orphan.id.0;
        snapshot.buildings.push(orphan);
        write_envelope(&path, VERSION_V2, &encode_snapshot(&snapshot).unwrap()).unwrap();
        assert!(World::load_file(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("incomplete building has no active clan project"));
        cleanup(&path);
    }

    #[test]
    fn saving_twice_replaces_the_previous_snapshot() {
        let path = test_path("replace");
        let mut world = representative_world();
        world.save_file(&path).unwrap();
        world.step();
        world.save_file(&path).unwrap();
        let loaded = World::load_file(&path).unwrap();
        assert_eq!(world.tick, loaded.tick);
        cleanup(&path);
    }

    fn representative_world() -> World {
        let mut world = World::new(40, 0x51A7_EF11);
        world.populate(8, 18, 3);
        world.maintain_pop = 24;
        world.maintain_clans = 3;
        for _ in 0..367 {
            world.step();
        }
        world
    }

    fn persistent_bytes(world: &World) -> Vec<u8> {
        encode_snapshot(&WorldSnapshotV3::capture(world)).unwrap()
    }

    fn test_path(label: &str) -> PathBuf {
        let sequence = NEXT_TEST_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "life-{label}-{}-{sequence}.lifeworld",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(temporary_path(path));
    }
}
