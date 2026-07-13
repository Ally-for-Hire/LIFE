//! World: the grid plus everything living on it, and the per-tick `step`.
//!
//! Tick layout:
//!   trees → clan_think (decide goals + cache targets) → entity behaviour
//!   → recruitment → combat → detach dead → maintain → prune territory
//!
//! Territory model (owner grid): clans claim only passable land, growth stays
//! connected, claiming is rate-limited and done by WORKERS toward a frontier
//! the leader's goal points at. Cut-off territory is pruned to "useless"
//! (unowned). Outsiders route around foreign territory unless there's no other
//! way to what they need, and any non-member standing on a clan's land is
//! hunted and killed.

use crate::brain::{Brain, N_EXPERTS, N_MODES, N_OUT};
use crate::clan::{hue_color, Clan, ClanMode};
use crate::diplomacy::DiplomacyLedger;
use crate::entity::{Entity, Goal};
use crate::grid::{terrain, Grid, NO_OWNER};
use crate::military::{
    add_entity_ore, assign_equipment, equipment_for, ore_cargo_for, remove_entity_equipment,
    remove_entity_ore_cargo, take_entity_ore, ClanMilitary, EntityEquipment, EntityOreCargo,
    EquipmentKind, OreDeposit, OreDepositId, MAX_CARRIED_ORE,
};
use crate::rng::Rng;
use crate::settlement::{
    active_building_counts, building_counts, development_score, Building, BuildingId, BuildingKind,
    ClanSettlement, TechState, MAX_TECH_LEVEL,
};
use std::collections::HashMap;

mod persistence;

// Default values (also slider starting points).
pub const D_STARVE_TICKS: i32 = 1400;
pub const D_STARVE_DAMAGE: f32 = 0.05;
pub const D_HEAL_RATE: f32 = 0.008;
pub const D_BASE_HEALTH: f32 = 10.0;
pub const D_LEADER_HEALTH: f32 = 24.0;
pub const D_MIN_SPEED: f32 = 0.25;
pub const D_MAX_SPEED: f32 = 0.5;
pub const D_VISION_RADIUS: i32 = 15;
pub const D_LEADER_CHANCE: f32 = 0.02;
pub const D_HUNGER_MIN: f32 = 0.16;
pub const D_HUNGER_MAX: f32 = 0.42;
pub const D_PELLET_ENERGY: i32 = 10;
pub const D_TREE_INTERVAL: i32 = 110;
pub const D_TREE_PER_CYCLE: i32 = 6;
pub const D_TREE_RADIUS: i32 = 7;
pub const D_MAX_PELLET_FRACTION: f32 = 0.09;
pub const D_CARRY_LIMIT: i32 = 10;
pub const D_ATTACK_DAMAGE: f32 = 0.45;
pub const D_ATTACK_COOLDOWN: i32 = 20;
pub const D_CLAN_GRACE_TICKS: i32 = 1800;
pub const D_WAR_THRESHOLD: f32 = 1.05;
pub const D_RECRUIT_RADIUS: i32 = 3;
pub const D_CLAIM_INTERVAL: i32 = 14;
pub const D_MEMBERS_PER_CLAIM: i32 = 2;
// farms: owned, fertile land grows food — this is what makes territory the economy.
pub const D_FARM_INTERVAL: i32 = 16;
pub const D_FARM_YIELD: f32 = 0.16;
pub const D_HOME_RANGE: i32 = 24;
pub const D_EXPAND_CLAIM_RADIUS: i32 = 1;
// seasons: a slow global yield cycle. Lean seasons cut food production, turning
// abundance into scarcity and giving leaders a recurring "situation" (famine vs
// plenty) to handle — and a reason to fight when the harvest fails.
pub const D_SEASON_LENGTH: i32 = 3000;
pub const D_SEASON_AMP: f32 = 0.55;
pub const D_WATER_LEVEL: f32 = 0.32;
pub const D_MOUNTAIN_LEVEL: f32 = 0.80;
pub const D_BIRTH_CHANCE: f32 = 0.025;
pub const D_BIRTH_INTERVAL: i32 = 180;
pub const D_BIRTH_FOOD_COST: i32 = 4;

const CLAN_THINK_INTERVAL: i32 = 120;
const TARGET_REFRESH_INTERVAL: i32 = 15;
const INITIAL_CLAIM_RADIUS: i32 = 3;
const TERRITORY_PRUNE_INTERVAL: i32 = 200;
const EXPAND_REACH: i32 = 30;
const EMERGENCY_HUNGER: f32 = 0.78;
const WORK_ASSIGNMENT_TICKS: i32 = CLAN_THINK_INTERVAL * 2;
const FOREST_WOOD_CAP: u8 = 6;
const WOOD_REGROW_INTERVAL: i32 = 360;
const WOOD_REGROW_CHANCE: f32 = 0.08;
const SPRING_WOOD_REGROW_MULTIPLIER: f32 = 1.5;
const AUTUMN_WOOD_REGROW_MULTIPLIER: f32 = 0.5;
const ROAD_BUILD_INTERVAL: i32 = 60;
const ROAD_WOOD_COST: i32 = 2;
const ROAD_MIN_TRAFFIC: u16 = 3;
const RESERVE_FOOD_PER_MEMBER: i32 = 4;
const STOCKPILE_FOOD_PER_MEMBER: i32 = 4;
const AUTUMN_STOCKPILE_FOOD_PER_MEMBER: i32 = 3;
const RESCUE_WINDOW_TICKS: i32 = 240;
const RESCUE_RADIUS: i32 = 12;
const RESCUE_REVIVE_HEALTH: f32 = 0.60;
const TRADE_REFRESH_INTERVAL: i32 = CLAN_THINK_INTERVAL;
const TRADE_RANGE: i32 = 60;
const TRADE_PACT_TICKS: i32 = CLAN_THINK_INTERVAL * 6;
const TRADE_PACT_MIN_MATERIAL: f32 = 9.0;
const TRADE_ALLY_TRUST: f32 = 0.15;
const TRADE_FOOD_FLOOR_PER_MEMBER: i32 = 8;
const TRADE_WOOD_FLOOR: i32 = 6;
const TRADE_FOOD_LOAD: i32 = 2;
const TRADE_WOOD_LOAD: i32 = 1;
const SETTLEMENT_PLAN_INTERVAL: i32 = 120;
const RESEARCH_INTERVAL: i32 = 10;
const PASSIVE_RESEARCH_INTERVAL: i32 = 30;
const MAX_ENTITIES_PER_CELL: u16 = 3;
const HOUSE_MEMBER_CAPACITY: usize = 2;
const GRANARY_RESERVE_CAPACITY: i32 = 6;
const SETTLEMENT_WOOD_MARGIN: i32 = 4;
const WALL_DEFENSE_RADIUS: i32 = 4;
const WALL_DAMAGE_REDUCTION: f32 = 0.25;
const HOUSE_HEAL_BONUS: f32 = 0.02;
const ORE_DEPOSIT_AMOUNT: u16 = 48;
const ORE_GLOBAL_SPACING: usize = 97;
const MILITARY_ORE_TARGET_PER_MEMBER: i32 = 2;
const MILITARY_WOOD_MARGIN: i32 = 8;
const MILITARY_PLAN_INTERVAL: i32 = 30;
/// Above this hunger, an outsider will steal from foreign farmland to survive —
/// which makes them a trespasser and triggers the owner's defense. Below it,
/// a clan's crops feed only its own people (despotic exclusion).
const EMERGENCY_STEAL: f32 = 0.9;

/// A named quarter of the existing global yield cycle. `Off` is the neutral
/// state when either seasonal parameter disables the cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeasonPhase {
    Off,
    Spring,
    Summer,
    Autumn,
    Winter,
}

/// Read-only seasonal state derived entirely from persisted tick/parameter
/// values. It deliberately owns no simulation state, so save compatibility and
/// exact post-load continuation do not depend on a new serialized field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonState {
    pub phase: SeasonPhase,
    /// Progress through the full cycle in `[0, 1)`.
    pub cycle_progress: f32,
    /// Progress through the current named quarter in `[0, 1)`.
    pub phase_progress: f32,
    /// Existing sine-wave food multiplier.
    pub yield_factor: f32,
    /// Cosine trend: positive means improving, negative means worsening.
    pub trend: f32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Params {
    // food / trees
    pub tree_interval: i32,
    pub tree_per_cycle: i32,
    pub tree_radius: i32,
    pub pellet_energy: i32,
    pub max_pellet_fraction: f32,
    // hunger / health
    pub starve_ticks: i32,
    pub starve_damage: f32,
    pub heal_rate: f32,
    pub base_health: f32,
    pub leader_health: f32,
    pub hunger_min: f32,
    pub hunger_max: f32,
    // movement / perception
    pub min_speed: f32,
    pub max_speed: f32,
    pub vision_radius: i32,
    pub leader_chance: f32,
    // clans / combat
    pub carry_limit: i32,
    pub attack_damage: f32,
    pub attack_cooldown: i32,
    pub clan_grace_ticks: i32,
    pub war_threshold: f32,
    pub recruit_radius: i32,
    pub claim_interval: i32,
    pub members_per_claim: i32,
    // farming / settlement
    pub farm_interval: i32, // ticks between farm growth passes on owned land
    pub farm_yield: f32,    // per owned fertile tile, pellet-spawn chance per pass (×fertility)
    pub home_range: i32,    // how far workers roam from the stockpile while working
    pub expand_claim_radius: i32, // radius of a single worker land claim while expanding
    pub season_length: i32, // ticks per full season cycle (0 = seasons off)
    pub season_amp: f32,    // yield swing amplitude (0..1); 0 = no seasonal variation
    pub soil_depletion_rate: f32, // how fast harvesting exhausts a tile (0 = off)
    pub disaster_rate: f32, // chance/intensity of regional blights (0 = off)
    pub community_logistics: bool, // shared reserves, wood labor, and roads
    pub community_care: bool, // incapacitation, evacuation, and recovery
    pub community_trade: bool, // relationship memory and physical surplus exchange
    // population growth
    pub birth_chance: f32,    // chance per pair of NPCs per reproduction check
    pub birth_interval: i32,  // ticks between reproduction checks
    pub birth_food_cost: i32, // food a clan spends per birth
    // terrain
    pub terrain_on: bool,
    pub water_level: f32,
    pub mountain_level: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            tree_interval: D_TREE_INTERVAL,
            tree_per_cycle: D_TREE_PER_CYCLE,
            tree_radius: D_TREE_RADIUS,
            pellet_energy: D_PELLET_ENERGY,
            max_pellet_fraction: D_MAX_PELLET_FRACTION,
            starve_ticks: D_STARVE_TICKS,
            starve_damage: D_STARVE_DAMAGE,
            heal_rate: D_HEAL_RATE,
            base_health: D_BASE_HEALTH,
            leader_health: D_LEADER_HEALTH,
            hunger_min: D_HUNGER_MIN,
            hunger_max: D_HUNGER_MAX,
            min_speed: D_MIN_SPEED,
            max_speed: D_MAX_SPEED,
            vision_radius: D_VISION_RADIUS,
            leader_chance: D_LEADER_CHANCE,
            carry_limit: D_CARRY_LIMIT,
            attack_damage: D_ATTACK_DAMAGE,
            attack_cooldown: D_ATTACK_COOLDOWN,
            clan_grace_ticks: D_CLAN_GRACE_TICKS,
            war_threshold: D_WAR_THRESHOLD,
            recruit_radius: D_RECRUIT_RADIUS,
            claim_interval: D_CLAIM_INTERVAL,
            members_per_claim: D_MEMBERS_PER_CLAIM,
            farm_interval: D_FARM_INTERVAL,
            farm_yield: D_FARM_YIELD,
            home_range: D_HOME_RANGE,
            expand_claim_radius: D_EXPAND_CLAIM_RADIUS,
            season_length: D_SEASON_LENGTH,
            season_amp: D_SEASON_AMP,
            soil_depletion_rate: 0.0, // off by default; the curriculum ramps it in
            disaster_rate: 0.0,       // off by default; the curriculum ramps it in
            community_logistics: true,
            community_care: true,
            community_trade: true,
            birth_chance: D_BIRTH_CHANCE,
            birth_interval: D_BIRTH_INTERVAL,
            birth_food_cost: D_BIRTH_FOOD_COST,
            terrain_on: true,
            water_level: D_WATER_LEVEL,
            mountain_level: D_MOUNTAIN_LEVEL,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Tree {
    pub x: i32,
    pub y: i32,
    pub last_spawn: i32,
    pub destroyed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GroundLoot {
    pub x: i32,
    pub y: i32,
    pub food: i32,
    pub wood: i32,
    pub ore: u16,
}

impl GroundLoot {
    fn is_empty(&self) -> bool {
        self.food <= 0 && self.wood <= 0 && self.ore == 0
    }
}

/// Row-major cells in the in-bounds 3x3 footprint centered on a building
/// anchor. Placement, persistence, occupancy, and rendering share this geometry.
pub(crate) fn building_footprint_cells(
    size: i32,
    x: i32,
    y: i32,
) -> impl Iterator<Item = (i32, i32)> {
    let min_x = (x - 1).max(0);
    let max_x = (x + 1).min(size - 1);
    let min_y = (y - 1).max(0);
    let max_y = (y + 1).min(size - 1);
    (min_y..=max_y).flat_map(move |cell_y| (min_x..=max_x).map(move |cell_x| (cell_x, cell_y)))
}

pub struct World {
    pub grid: Grid,
    pub tick: i32,
    pub entities: Vec<Entity>,
    pub trees: Vec<Tree>,
    pub clans: Vec<Clan>,
    pub rng: Rng,
    pub params: Params,
    pub diplomacy: DiplomacyLedger,
    pub buildings: Vec<Building>,
    pub building_cells: Vec<u32>,
    pub settlements: Vec<ClanSettlement>,
    pub community_settlement: bool,
    pub ore_deposits: Vec<OreDeposit>,
    pub ore_cargo: Vec<EntityOreCargo>,
    pub militaries: Vec<ClanMilitary>,
    pub equipment: Vec<EntityEquipment>,
    pub ground_loot: Vec<GroundLoot>,
    pub community_military: bool,
    next_entity_id: u32,
    next_clan_id: i32,
    next_building_id: u32,
    next_ore_deposit_id: u32,
    pellet_total: usize,
    pub deaths_starved: u64,
    pub deaths_combat: u64,
    pub births: u64,
    pub maintain_pop: i32,
    /// Target number of clans to keep alive — when conflict thins the field
    /// below this, a new village forms from masterless refugees (or fresh land).
    /// Keeps the world a living patchwork of villages instead of consolidating.
    pub maintain_clans: i32,
    /// Best brain from the background arena trainer, if any. New villages
    /// occasionally inherit from it, so offline evolution flows into the live
    /// world automatically (the app keeps this in sync with the trainer).
    pub champion: Option<Brain>,
    /// Decaying [0,1] "turbulence" signal — spikes when a regional disaster hits,
    /// fed to leaders so they can learn to keep reserves against shocks.
    disaster_level: f32,
    reach: Vec<i32>,    // reusable flood-fill buffer for territory pruning
    occupied: Vec<u16>, // entities per cell, capped at three units per tile
}

impl World {
    pub fn new(size: i32, seed: u64) -> Self {
        World {
            grid: Grid::new(size),
            tick: 0,
            entities: Vec::new(),
            trees: Vec::new(),
            clans: Vec::new(),
            rng: Rng::new(seed),
            params: Params::default(),
            diplomacy: DiplomacyLedger::new(),
            buildings: Vec::new(),
            building_cells: vec![0; (size * size) as usize],
            settlements: Vec::new(),
            community_settlement: true,
            ore_deposits: Vec::new(),
            ore_cargo: Vec::new(),
            militaries: Vec::new(),
            equipment: Vec::new(),
            ground_loot: Vec::new(),
            community_military: true,
            next_entity_id: 1,
            next_clan_id: 1,
            next_building_id: 1,
            next_ore_deposit_id: 1,
            pellet_total: 0,
            deaths_starved: 0,
            deaths_combat: 0,
            births: 0,
            maintain_pop: 0,
            maintain_clans: 0,
            champion: None,
            disaster_level: 0.0,
            reach: Vec::new(),
            occupied: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.tick = 0;
        self.entities.clear();
        self.trees.clear();
        self.clans.clear();
        for p in self.grid.pellet.iter_mut() {
            *p = 0;
        }
        for d in self.grid.depletion.iter_mut() {
            *d = 0;
        }
        for r in self.grid.road.iter_mut() {
            *r = 0;
        }
        for w in self.grid.wood.iter_mut() {
            *w = 0;
        }
        for t in self.grid.traffic.iter_mut() {
            *t = 0;
        }
        for o in self.grid.owner.iter_mut() {
            *o = NO_OWNER;
        }
        for t in self.grid.terrain.iter_mut() {
            *t = terrain::PLAINS;
        }
        self.pellet_total = 0;
        self.deaths_starved = 0;
        self.deaths_combat = 0;
        self.births = 0;
        self.diplomacy = DiplomacyLedger::new();
        self.buildings.clear();
        self.building_cells.fill(0);
        self.settlements.clear();
        self.ore_deposits.clear();
        self.ore_cargo.clear();
        self.militaries.clear();
        self.equipment.clear();
        self.ground_loot.clear();
        self.disaster_level = 0.0;
        self.next_clan_id = 1;
        self.next_building_id = 1;
        self.next_ore_deposit_id = 1;
    }

    fn reserve_building_footprint(&mut self, x: i32, y: i32, id: BuildingId) {
        for (cell_x, cell_y) in building_footprint_cells(self.grid.size, x, y) {
            let cell = self.grid.idx(cell_x, cell_y);
            if self.building_cells[cell] == 0 || id.0 < self.building_cells[cell] {
                self.building_cells[cell] = id.0;
            }
        }
    }

    fn clear_building_footprint(&mut self, x: i32, y: i32, id: BuildingId) {
        for (cell_x, cell_y) in building_footprint_cells(self.grid.size, x, y) {
            let cell = self.grid.idx(cell_x, cell_y);
            if self.building_cells[cell] == id.0 {
                self.building_cells[cell] = 0;
            }
        }
    }

    fn rebuild_building_footprints(&mut self) {
        self.building_cells.fill(0);
        let active: Vec<_> = self
            .buildings
            .iter()
            .filter(|building| !building.is_destroyed())
            .map(|building| (building.x, building.y, building.id))
            .collect();
        for (x, y, id) in active {
            self.reserve_building_footprint(x, y, id);
        }
    }

    /// V1-V3 predate the live three-unit admission rule. Preserve those saves
    /// by relocating only overflow entities to the nearest deterministic free
    /// passable cell; V4 validates the invariant instead of migrating it.
    fn migrate_legacy_entity_cell_capacity(&mut self) -> bool {
        let cell_count = self.grid.terrain.len();
        let mut counts = vec![0u16; cell_count];
        for entity_index in 0..self.entities.len() {
            if self.entities[entity_index].dead {
                continue;
            }
            let (x, y) = (self.entities[entity_index].x, self.entities[entity_index].y);
            let cell = self.grid.idx(x, y);
            if counts[cell] < MAX_ENTITIES_PER_CELL {
                counts[cell] += 1;
                continue;
            }

            let replacement = (0..cell_count)
                .filter(|&candidate| {
                    counts[candidate] < MAX_ENTITIES_PER_CELL
                        && terrain_move_cost(self.grid.terrain[candidate]).is_finite()
                })
                .min_by_key(|&candidate| {
                    let candidate_x = candidate as i32 % self.grid.size;
                    let candidate_y = candidate as i32 / self.grid.size;
                    (
                        (candidate_x - x).abs().max((candidate_y - y).abs()),
                        candidate,
                    )
                });
            let Some(replacement) = replacement else {
                return false;
            };
            self.entities[entity_index].x = replacement as i32 % self.grid.size;
            self.entities[entity_index].y = replacement as i32 / self.grid.size;
            counts[replacement] += 1;
        }
        true
    }

    // --- terrain ---
    #[inline]
    fn is_passable(&self, x: i32, y: i32) -> bool {
        let t = self.grid.terrain[self.grid.idx(x, y)];
        t != terrain::WATER && t != terrain::MOUNTAIN
    }

    #[inline]
    fn is_foreign_tile(&self, x: i32, y: i32, own_clan: i32) -> bool {
        let o = self.grid.owner[self.grid.idx(x, y)];
        o != NO_OWNER && o != own_clan && !self.are_allied(own_clan, o)
    }

    /// Fractal value-noise field in [0,1], deterministic via the world rng.
    fn value_noise(&mut self, base_freq: f32, octaves: u32) -> Vec<f32> {
        let size = self.grid.size as usize;
        let mut field = vec![0f32; size * size];
        let mut amp = 1.0f32;
        let mut freq = base_freq;
        let mut max_amp = 0.0f32;
        for _ in 0..octaves {
            let cells = ((self.grid.size as f32 * freq).ceil() as i32).max(2);
            let step = self.grid.size as f32 / cells as f32;
            let latw = (cells + 2) as usize;
            let mut lat = vec![0f32; latw * latw];
            for v in lat.iter_mut() {
                *v = self.rng.f32();
            }
            for y in 0..size {
                for x in 0..size {
                    let fx = x as f32 / step;
                    let fy = y as f32 / step;
                    let x0 = fx.floor() as usize;
                    let y0 = fy.floor() as usize;
                    let tx = fx - x0 as f32;
                    let ty = fy - y0 as f32;
                    let sx = tx * tx * (3.0 - 2.0 * tx);
                    let sy = ty * ty * (3.0 - 2.0 * ty);
                    let v00 = lat[y0 * latw + x0];
                    let v10 = lat[y0 * latw + x0 + 1];
                    let v01 = lat[(y0 + 1) * latw + x0];
                    let v11 = lat[(y0 + 1) * latw + x0 + 1];
                    let top = v00 + (v10 - v00) * sx;
                    let bot = v01 + (v11 - v01) * sx;
                    field[y * size + x] += (top + (bot - top) * sy) * amp;
                }
            }
            max_amp += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        for v in field.iter_mut() {
            *v /= max_amp;
        }
        field
    }

    fn generate_terrain(&mut self) {
        let n = (self.grid.size * self.grid.size) as usize;
        if !self.params.terrain_on {
            for i in 0..n {
                self.grid.terrain[i] = terrain::PLAINS;
                self.grid.fertility[i] = 180;
                self.grid.wood[i] = 0;
            }
            return;
        }
        let elev = self.value_noise(0.012, 5);
        let moist = self.value_noise(0.02, 4);
        // A separate, lower-frequency fertility field so good farmland is rare
        // and clumped into a few fertile basins — not uniform plains. Scarce,
        // patchy richness is the precondition for villages settling on the best
        // land and clashing over it (Resource Dispersion / economic defendability).
        let fert = self.value_noise(0.03, 3);
        let water = self.params.water_level;
        let mountain = self.params.mountain_level;
        for i in 0..n {
            let e = elev[i];
            let m = moist[i];
            let (t, base) = if e < water {
                (terrain::WATER, 0u16)
            } else if e < water + 0.04 {
                (terrain::SAND, 55)
            } else if e > mountain {
                (terrain::MOUNTAIN, 0)
            } else if e > mountain - 0.14 {
                (terrain::HILL, 120)
            } else if m > 0.55 {
                (terrain::FOREST, 235)
            } else {
                (terrain::PLAINS, 205)
            };
            // Skew the multiplier low (powf > 1) so most land is mediocre and
            // only a few patches are prime — those become the contested valleys.
            let mult = 0.16 + 0.84 * fert[i].clamp(0.0, 1.0).powf(1.6);
            let f = if base == 0 {
                0
            } else {
                ((base as f32 * mult) as u16).clamp(8, 255) as u8
            };
            self.grid.terrain[i] = t;
            self.grid.fertility[i] = f;
            self.grid.wood[i] = if t == terrain::FOREST {
                FOREST_WOOD_CAP
            } else {
                0
            };
        }
    }

    /// Deterministic finite mineral deposits. Their placement consumes no RNG,
    /// so military treatment/control arms begin from the same world stream.
    fn generate_ore_deposits(&mut self) {
        self.ore_deposits.clear();
        self.next_ore_deposit_id = 1;
        for index in 0..self.grid.terrain.len() {
            let terrain = self.grid.terrain[index];
            if terrain == terrain::WATER
                || terrain == terrain::MOUNTAIN
                || self.grid.wood[index] > 0
            {
                continue;
            }
            let hash = index
                .wrapping_mul(0x9E37_79B1usize)
                .wrapping_add(self.grid.fertility[index] as usize * 131);
            if hash % ORE_GLOBAL_SPACING != 0 && terrain != terrain::HILL {
                continue;
            }
            if terrain == terrain::HILL && hash % 31 != 0 {
                continue;
            }
            let x = (index as i32) % self.grid.size;
            let y = (index as i32) / self.grid.size;
            self.push_ore_deposit(x, y, ORE_DEPOSIT_AMOUNT);
        }
    }

    fn push_ore_deposit(&mut self, x: i32, y: i32, amount: u16) {
        if self
            .ore_deposits
            .iter()
            .any(|deposit| deposit.x == x && deposit.y == y)
        {
            return;
        }
        let id = OreDepositId(self.next_ore_deposit_id);
        self.next_ore_deposit_id = self.next_ore_deposit_id.saturating_add(1);
        self.ore_deposits.push(OreDeposit::new(id, x, y, amount));
    }

    fn initialize_military_resources(&mut self) {
        self.generate_ore_deposits();
        let settlements: Vec<(i32, i32, i32)> = self
            .clans
            .iter()
            .filter_map(|clan| clan.stockpile.map(|(x, y)| (clan.id, x, y)))
            .collect();
        for (clan_id, x, y) in settlements {
            self.ensure_ore_near(clan_id, x, y);
        }
    }

    /// Every settlement gets one reachable bootstrap deposit. This prevents a
    /// zero pipeline from meaning unlucky geography rather than policy failure.
    fn ensure_ore_near(&mut self, clan_id: i32, x: i32, y: i32) {
        let existing = self.ore_deposits.iter().any(|deposit| {
            !deposit.is_depleted()
                && (deposit.x - x).abs().max((deposit.y - y).abs()) <= 10
                && !self.is_foreign_tile(deposit.x, deposit.y, clan_id)
        });
        if existing {
            return;
        }
        let mut candidates = Vec::new();
        for yy in (y - 8).max(0)..=(y + 8).min(self.grid.size - 1) {
            for xx in (x - 8).max(0)..=(x + 8).min(self.grid.size - 1) {
                if (xx, yy) == (x, y) || !self.is_passable(xx, yy) {
                    continue;
                }
                let index = self.grid.idx(xx, yy);
                if self.grid.wood[index] > 0 || self.building_cells[index] != 0 {
                    continue;
                }
                if self.is_foreign_tile(xx, yy, clan_id) {
                    continue;
                }
                let distance = (xx - x).abs().max((yy - y).abs());
                candidates.push((distance, index, xx, yy));
            }
        }
        candidates.sort_unstable();
        if let Some((_, _, dx, dy)) = candidates.first().copied() {
            self.push_ore_deposit(dx, dy, ORE_DEPOSIT_AMOUNT);
        }
    }

    // --- population ---
    fn roll_hunger_threshold(&mut self) -> f32 {
        let lo = self.params.hunger_min;
        let hi = self.params.hunger_max.max(lo);
        lo + self.rng.f32() * (hi - lo)
    }

    fn fullness_ticks(&self) -> i32 {
        let energy = self.params.pellet_energy.clamp(1, 100);
        ((energy - 1) * self.params.starve_ticks.max(1) / 20)
            .clamp(0, self.params.starve_ticks.max(1) * 3)
    }

    fn feed_entity(&mut self, e: &mut Entity) {
        e.ticks_since_food = -self.fullness_ticks();
        e.hunger_threshold = self.roll_hunger_threshold();
        e.health = (e.health + self.params.heal_rate * 6.0).min(e.max_health);
    }

    fn eat_carried(&mut self, e: &mut Entity) -> bool {
        if e.food <= 0 {
            return false;
        }
        e.food -= 1;
        self.feed_entity(e);
        e.goal = Goal::Eating;
        true
    }

    fn eat_from_stockpile(&mut self, e: &mut Entity, cidx: usize) -> bool {
        let Some((sx, sy)) = self.clans[cidx].stockpile else {
            return false;
        };
        if e.x != sx || e.y != sy {
            return false;
        }
        if self.clans[cidx].food > 0 {
            self.clans[cidx].food -= 1;
        } else if self.params.community_logistics && self.clans[cidx].reserve_food > 0 {
            self.clans[cidx].reserve_food -= 1;
            self.clans[cidx].stats.reserve_released += 1;
        } else {
            return false;
        }
        self.feed_entity(e);
        e.goal = Goal::Eating;
        true
    }

    /// Deliver food to the shared stockpile. Ordinary working food is filled
    /// first; only the surplus above that floor enters the protected reserve.
    fn deposit_food(&mut self, cidx: usize, amount: i32) {
        if amount <= 0 {
            return;
        }
        self.clans[cidx].stats.food_delivered += amount as u32;
        if !self.params.community_logistics {
            self.clans[cidx].food += amount;
            return;
        }
        let pop = self.clan_roster_size(cidx);
        // Autumn harvests prepare for the falling half of the cycle: once a
        // smaller working buffer is ready, deliveries fill protected storage.
        // After the existing reserve cap is full, normal stockpile growth
        // resumes, so prosperous clans can still fund civic work.
        let ordinary_per_member = if self.season_phase() == SeasonPhase::Autumn {
            AUTUMN_STOCKPILE_FOOD_PER_MEMBER
        } else {
            STOCKPILE_FOOD_PER_MEMBER
        };
        let ordinary_floor = pop * ordinary_per_member;
        let base_reserve_cap = pop * RESERVE_FOOD_PER_MEMBER;
        let reserve_cap = self.reserve_capacity(cidx);
        let reserve_before = self.clans[cidx].reserve_food;
        let ordinary_needed = (ordinary_floor - self.clans[cidx].food).max(0);
        let to_ordinary = amount.min(ordinary_needed);
        self.clans[cidx].food += to_ordinary;

        let remaining = amount - to_ordinary;
        let reserve_room = (reserve_cap - self.clans[cidx].reserve_food).max(0);
        let to_reserve = remaining.min(reserve_room);
        self.clans[cidx].reserve_food += to_reserve;
        self.clans[cidx].stats.reserve_deposited += to_reserve as u32;
        self.clans[cidx].food += remaining - to_reserve;
        if self.community_settlement {
            let granary_storage = (self.clans[cidx].reserve_food - base_reserve_cap).max(0)
                - (reserve_before - base_reserve_cap).max(0);
            if granary_storage > 0 {
                let state = self.ensure_settlement_index(self.clans[cidx].id);
                self.settlements[state].stats.granary_food_stored += granary_storage as u32;
            }
        }
    }

    fn consume_pellet_at(&mut self, e: &mut Entity, carry: bool) -> bool {
        let i = self.grid.idx(e.x, e.y);
        if self.grid.pellet[i] == 0 {
            return false;
        }
        // Despotic exclusion: a clan's farmland feeds only its own members. An
        // outsider can't harvest it unless desperate — and stealing makes them a
        // hunted trespasser. This is what gives owning land real value.
        let owner = self.grid.owner[i];
        if owner != NO_OWNER && owner != e.clan {
            let hunger = e.hunger(self.params.starve_ticks.max(1));
            if hunger < EMERGENCY_STEAL {
                return false;
            }
        }
        self.grid.pellet[i] = 0;
        self.pellet_total = self.pellet_total.saturating_sub(1);
        // Harvesting exhausts the soil (recovers over time in grow_farms).
        if self.params.soil_depletion_rate > 0.0 {
            let bump = (self.params.soil_depletion_rate * 70.0) as u8;
            self.grid.depletion[i] = self.grid.depletion[i].saturating_add(bump);
        }
        e.last_food = Some((e.x, e.y));
        if carry {
            e.food += 1;
        } else {
            self.feed_entity(e);
            e.goal = Goal::Eating;
        }
        true
    }

    fn survival_food_radius(&self, e: &Entity) -> i32 {
        let hunger = e.hunger(self.params.starve_ticks.max(1));
        let vision = self.params.vision_radius.max(1);
        if hunger >= 0.92 {
            self.grid.size
        } else if hunger >= EMERGENCY_HUNGER {
            (vision * 5).min(self.grid.size)
        } else {
            (vision * 2).min(self.grid.size)
        }
    }

    fn make_entity(&mut self, x: i32, y: i32, is_leader: bool) -> Entity {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        let lo = self.params.min_speed;
        let hi = self.params.max_speed.max(lo);
        let speed = lo + self.rng.f32() * (hi - lo);
        let max_health = if is_leader {
            self.params.leader_health
        } else {
            self.params.base_health
        };
        let threshold = self.roll_hunger_threshold();
        Entity {
            id,
            x,
            y,
            speed,
            move_budget: 0.0,
            health: max_health,
            max_health,
            is_leader,
            food: 0,
            wood: 0,
            ticks_since_food: 0,
            hunger_threshold: threshold,
            goal: Goal::Wander,
            clan: -1,
            work_role: ClanMode::Gather,
            work_until: 0,
            last_food: None,
            attack_cooldown: 0,
            incapacitated_until: 0,
            downed_by_clan: -1,
            downed_by_entity: None,
            rescue_target: None,
            carried_by: None,
            trade_target_clan: -1,
            trade_returning: false,
            trade_food: 0,
            trade_wood: 0,
            dead: false,
        }
    }

    pub fn spawn_entity(&mut self, x: i32, y: i32, is_leader: bool) {
        if self.cell_at_capacity(x, y) {
            return;
        }
        let e = self.make_entity(x, y, is_leader);
        self.entities.push(e);
    }

    fn random_cell(&mut self) -> (i32, i32) {
        let s = self.grid.size;
        (self.rng.below(s), self.rng.below(s))
    }

    /// Random unowned, passable cell — keeps spawns off water/mountain and out
    /// of existing clan territory (so they don't spawn straight into a kill).
    fn random_land_cell(&mut self) -> Option<(i32, i32)> {
        for _ in 0..50 {
            let (x, y) = self.random_cell();
            let i = self.grid.idx(x, y);
            let t = self.grid.terrain[i];
            if t != terrain::WATER
                && t != terrain::MOUNTAIN
                && self.grid.owner[i] == NO_OWNER
                && !self.cell_at_capacity(x, y)
            {
                return Some((x, y));
            }
        }
        for _ in 0..50 {
            let (x, y) = self.random_cell();
            if self.is_passable(x, y) && !self.cell_at_capacity(x, y) {
                return Some((x, y));
            }
        }
        for require_unowned in [true, false] {
            for cell in 0..self.grid.terrain.len() {
                let x = cell as i32 % self.grid.size;
                let y = cell as i32 / self.grid.size;
                if self.is_passable(x, y)
                    && (!require_unowned || self.grid.owner[cell] == NO_OWNER)
                    && !self.cell_at_capacity(x, y)
                {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// A passable, unowned cell biased toward high fertility — picks the most
    /// fertile of several random samples. Used to seed trees (and clans) onto
    /// the good land that's worth settling and fighting for.
    fn random_fertile_land_cell(&mut self) -> Option<(i32, i32)> {
        let mut best = self.random_land_cell()?;
        let mut best_f = self.grid.fertility[self.grid.idx(best.0, best.1)];
        for _ in 0..6 {
            let Some((x, y)) = self.random_land_cell() else {
                break;
            };
            let f = self.grid.fertility[self.grid.idx(x, y)];
            if f > best_f {
                best_f = f;
                best = (x, y);
            }
        }
        Some(best)
    }

    fn cell_at_capacity(&self, x: i32, y: i32) -> bool {
        self.entities
            .iter()
            .filter(|e| !e.dead && e.x == x && e.y == y)
            .count()
            >= MAX_ENTITIES_PER_CELL as usize
    }

    fn nearby_spawn_cell(&mut self, cx: i32, cy: i32, r: i32) -> Option<(i32, i32)> {
        let radius = r.max(1);
        for _ in 0..80 {
            let x = self.grid.clamp(cx + self.rng.range(-radius, radius + 1));
            let y = self.grid.clamp(cy + self.rng.range(-radius, radius + 1));
            if self.is_passable(x, y) && !self.cell_at_capacity(x, y) {
                return Some((x, y));
            }
        }
        for rr in radius + 1..=(radius + 8).min(self.grid.size) {
            for yy in (cy - rr).max(0)..=(cy + rr).min(self.grid.size - 1) {
                for xx in (cx - rr).max(0)..=(cx + rr).min(self.grid.size - 1) {
                    if self.is_passable(xx, yy) && !self.cell_at_capacity(xx, yy) {
                        return Some((xx, yy));
                    }
                }
            }
        }
        self.random_land_cell()
    }

    fn spawn_clan(&mut self) -> bool {
        let brain = self.breed_brain();
        self.spawn_clan_with(brain) != NO_OWNER
    }

    /// In-vivo evolution: a new leader's brain is bred from the clans that are
    /// currently *thriving* (big, landed, winning), via tournament selection +
    /// crossover + mutation — with an occasional fresh random "immigrant" for
    /// diversity. The live world is thus its own training ground: strategies
    /// that survive and dominate spread to the next generation of villages,
    /// while losers' genes die out. No manual training step required.
    fn breed_brain(&mut self) -> Brain {
        // Champion transfer: a slice of new villages inherit the arena trainer's
        // best brain (lightly mutated), so offline evolution shapes the live
        // world without any manual seeding.
        if let Some(champ) = self.champion.clone() {
            if self.rng.chance(0.3) {
                let mut child = champ;
                child.mutate(&mut self.rng, 0.06, 0.2);
                return child;
            }
        }
        // immigrant: inject fresh genes sometimes so the gene pool can't stagnate
        if self.rng.chance(0.15) || self.clans.iter().all(|c| c.disbanded) {
            return Brain::random(&mut self.rng);
        }
        let mut pool: Vec<(usize, f32)> = Vec::new();
        for (i, c) in self.clans.iter().enumerate() {
            if c.disbanded {
                continue;
            }
            let pop = self.clan_population(c.id) as f32;
            if pop < 1.0 {
                continue;
            }
            // success = people fed on held, productive land, plus wins
            let fit = pop
                + c.fertile_capacity * 0.6
                + c.territory as f32 * 0.05
                + c.stats.kills as f32 * 0.5
                + if self.params.community_logistics {
                    c.reserve_food.max(0) as f32 * 0.12
                        + (c.stats.road_steps as f32).sqrt() * 0.12
                        + (c.stats.food_delivered as f32).sqrt() * 0.08
                        + c.stats.reserve_released as f32 * 0.15
                } else {
                    0.0
                }
                + if self.params.community_trade {
                    (c.stats
                        .trade_food_sent
                        .saturating_add(c.stats.trade_wood_sent) as f32)
                        .sqrt()
                        * 0.10
                } else {
                    0.0
                };
            pool.push((i, fit));
        }
        if pool.is_empty() {
            return Brain::random(&mut self.rng);
        }
        let pick = |pool: &[(usize, f32)], rng: &mut Rng| -> usize {
            let k = 3.min(pool.len());
            let mut best = 0usize;
            let mut bf = f32::MIN;
            for _ in 0..k {
                let j = rng.below(pool.len() as i32) as usize;
                if pool[j].1 > bf {
                    bf = pool[j].1;
                    best = j;
                }
            }
            pool[best].0
        };
        let ai = pick(&pool, &mut self.rng);
        let bi = pick(&pool, &mut self.rng);
        let pa = self.clans[ai].brain.clone();
        let pb = self.clans[bi].brain.clone();
        let mut child = Brain::crossover(&pa, &pb, &mut self.rng);
        child.mutate(&mut self.rng, 0.12, 0.3);
        child
    }

    pub fn spawn_clan_with(&mut self, brain: Brain) -> i32 {
        let Some((x, y)) = self.random_land_cell() else {
            return NO_OWNER;
        };
        let mut leader = self.make_entity(x, y, true);
        leader.clan = -1;
        let lid = leader.id;
        self.entities.push(leader);

        let id = self.create_clan(lid, x, y, brain);
        let li = self.entities.len() - 1;
        self.entities[li].clan = id;
        let idx = self.clan_index(id).unwrap();

        for _ in 0..3 {
            let Some((fx, fy)) = self.nearby_spawn_cell(x, y, 3) else {
                break;
            };
            let mut f = self.make_entity(fx, fy, false);
            f.clan = id;
            let fid = f.id;
            self.entities.push(f);
            self.clans[idx].members.push(fid);
        }
        id
    }

    fn create_clan(&mut self, leader_id: u32, x: i32, y: i32, brain: Brain) -> i32 {
        let id = self.next_clan_id;
        self.next_clan_id += 1;
        let color = hue_color(id as f32 * 67.0);
        let mut clan = Clan::new(id, leader_id, color, brain, self.tick);
        clan.stockpile = Some((x, y));
        clan.food = 14;
        self.clans.push(clan);
        let idx = self.clans.len() - 1;
        self.claim_area(idx, x, y, INITIAL_CLAIM_RADIUS);
        self.ensure_ore_near(id, x, y);
        id
    }

    pub fn populate(&mut self, neutrals: i32, trees: i32, clans: i32) {
        self.clear();
        self.generate_terrain();
        self.generate_ore_deposits();
        for _ in 0..trees {
            let Some((x, y)) = self.random_fertile_land_cell() else {
                break;
            };
            let last = -self.rng.below(self.params.tree_interval.max(1));
            self.trees.push(Tree {
                x,
                y,
                last_spawn: last,
                destroyed: false,
            });
        }
        for _ in 0..clans {
            // initial clans get diverse random brains — seeding diversity matters
            // enormously; in-vivo breeding only applies to later replacements.
            let brain = Brain::random(&mut self.rng);
            self.spawn_clan_with(brain);
        }
        for _ in 0..neutrals {
            let Some((x, y)) = self.random_land_cell() else {
                break;
            };
            self.spawn_entity(x, y, false);
        }
    }

    pub fn setup_arena(&mut self, brains: &[Brain], trees: i32, neutrals: i32) -> Vec<i32> {
        self.clear();
        self.generate_terrain();
        self.generate_ore_deposits();
        for _ in 0..trees {
            let Some((x, y)) = self.random_fertile_land_cell() else {
                break;
            };
            let last = -self.rng.below(self.params.tree_interval.max(1));
            self.trees.push(Tree {
                x,
                y,
                last_spawn: last,
                destroyed: false,
            });
        }
        let mut ids = Vec::with_capacity(brains.len());
        for b in brains {
            ids.push(self.spawn_clan_with(b.clone()));
        }
        for _ in 0..neutrals {
            let Some((x, y)) = self.random_land_cell() else {
                break;
            };
            self.spawn_entity(x, y, false);
        }
        ids
    }

    pub fn seed_clan(&mut self, brain: Brain) -> i32 {
        let id = self.spawn_clan_with(brain);
        if let Some(idx) = self.clan_index(id) {
            let (lx, ly) = self
                .entity_pos(self.clans[idx].leader_id)
                .unwrap_or((self.grid.size / 2, self.grid.size / 2));
            for _ in 0..6 {
                let Some((fx, fy)) = self.nearby_spawn_cell(lx, ly, 4) else {
                    break;
                };
                let mut f = self.make_entity(fx, fy, false);
                f.clan = id;
                let fid = f.id;
                self.entities.push(f);
                self.clans[idx].members.push(fid);
            }
            self.clans[idx].food += 30;
        }
        id
    }

    // --- territory ---
    fn has_owned_neighbor(&self, id: i32, x: i32, y: i32) -> bool {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = x + dx;
                let ny = y + dy;
                if self.grid.in_bounds(nx, ny) && self.grid.owner[self.grid.idx(nx, ny)] == id {
                    return true;
                }
            }
        }
        false
    }

    /// Claim passable tiles in a disc. The founding claim (territory == 0) seeds
    /// a contiguous blob; afterwards growth only claims tiles adjacent to land
    /// the clan already owns, so claims can never be disconnected.
    fn claim_area(&mut self, clan_idx: usize, cx: i32, cy: i32, r: i32) {
        let id = self.clans[clan_idx].id;
        let founding = self.clans[clan_idx].territory == 0;
        let r2 = r * r;
        let mut added = 0u32;
        let mut added_fert = 0.0f32;
        for yy in (cy - r).max(0)..=(cy + r).min(self.grid.size - 1) {
            for xx in (cx - r).max(0)..=(cx + r).min(self.grid.size - 1) {
                let dx = xx - cx;
                let dy = yy - cy;
                if dx * dx + dy * dy > r2 || !self.is_passable(xx, yy) {
                    continue;
                }
                let i = self.grid.idx(xx, yy);
                if self.grid.owner[i] == NO_OWNER
                    && (founding || self.has_owned_neighbor(id, xx, yy))
                {
                    self.grid.owner[i] = id;
                    added += 1;
                    added_fert += self.grid.fertility[i] as f32 / 255.0;
                }
            }
        }
        self.clans[clan_idx].territory += added;
        self.clans[clan_idx].fertile_capacity += added_fert;
    }

    /// Flood-fill each clan's territory from its stockpile; tiles that can't be
    /// reached through owned land (cut off by enemies, water, or disconnection)
    /// revert to unowned — "useless". Then recount each clan's territory.
    fn prune_territory(&mut self) {
        let size = self.grid.size;
        let n = (size * size) as usize;
        if self.reach.len() != n {
            self.reach = vec![NO_OWNER; n];
        } else {
            for v in self.reach.iter_mut() {
                *v = NO_OWNER;
            }
        }
        let mut queue: Vec<usize> = Vec::new();
        for c in &self.clans {
            if c.disbanded {
                continue;
            }
            if let Some((sx, sy)) = c.stockpile {
                let i = (sy * size + sx) as usize;
                self.grid.owner[i] = c.id; // keep the base owned
                self.reach[i] = c.id;
                queue.push(i);
            }
        }
        let mut head = 0;
        while head < queue.len() {
            let i = queue[head];
            head += 1;
            let id = self.reach[i];
            let x = (i as i32) % size;
            let y = (i as i32) / size;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let nx = x + dx;
                let ny = y + dy;
                if nx < 0 || ny < 0 || nx >= size || ny >= size {
                    continue;
                }
                let ni = (ny * size + nx) as usize;
                if self.grid.owner[ni] == id && self.reach[ni] == NO_OWNER {
                    self.reach[ni] = id;
                    queue.push(ni);
                }
            }
        }
        for i in 0..n {
            let o = self.grid.owner[i];
            if o != NO_OWNER && self.reach[i] != o {
                self.grid.owner[i] = NO_OWNER;
            }
        }
        let mut counts: HashMap<i32, u32> = HashMap::new();
        let mut fert: HashMap<i32, f32> = HashMap::new();
        let mut depl: HashMap<i32, f32> = HashMap::new();
        for i in 0..n {
            let o = self.grid.owner[i];
            if o != NO_OWNER {
                *counts.entry(o).or_insert(0) += 1;
                *fert.entry(o).or_insert(0.0) += self.grid.fertility[i] as f32 / 255.0;
                *depl.entry(o).or_insert(0.0) += self.grid.depletion[i] as f32 / 255.0;
            }
        }
        for c in self.clans.iter_mut() {
            let tiles = counts.get(&c.id).copied().unwrap_or(0);
            c.territory = tiles;
            c.fertile_capacity = fert.get(&c.id).copied().unwrap_or(0.0);
            c.soil_depletion = if tiles > 0 {
                depl.get(&c.id).copied().unwrap_or(0.0) / tiles as f32
            } else {
                0.0
            };
        }
    }

    // --- lookups ---
    fn clan_index(&self, id: i32) -> Option<usize> {
        if id < 0 {
            return None;
        }
        self.clans.iter().position(|c| c.id == id && !c.disbanded)
    }

    fn clan_aggr(&self, id: i32) -> f32 {
        self.clan_index(id)
            .map(|i| self.clans[i].aggression)
            .unwrap_or(0.0)
    }

    fn entity_index(&self, id: u32) -> Option<usize> {
        self.entities.iter().position(|e| e.id == id)
    }

    fn entity_pos(&self, id: u32) -> Option<(i32, i32)> {
        self.entities
            .iter()
            .find(|e| e.id == id && e.is_active())
            .map(|e| (e.x, e.y))
    }

    pub fn entity_by_id(&self, id: u32) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn clan_by_id(&self, id: i32) -> Option<&Clan> {
        self.clans.iter().find(|c| c.id == id && !c.disbanded)
    }

    pub fn clan_population(&self, id: i32) -> usize {
        self.entities
            .iter()
            .filter(|e| e.clan == id && e.is_active())
            .count()
    }

    fn clan_roster_size(&self, clan_idx: usize) -> i32 {
        (1 + self.clans[clan_idx].members.len()) as i32
    }

    /// Population a clan's land can support — keyed to *productive* (fertile)
    /// territory, not raw tile count (Resource Dispersion Hypothesis). A fertile
    /// valley feeds a real village; scrubland supports only a few, which pushes
    /// the clan to expand or fight toward better land.
    fn clan_member_cap(&self, clan_idx: usize) -> usize {
        let cap = self.clans[clan_idx].fertile_capacity.max(0.5);
        let land_cap = (cap * self.params.members_per_claim.max(1) as f32)
            .round()
            .max(3.0) as usize;
        let houses = if self.community_settlement {
            active_building_counts(&self.buildings, self.clans[clan_idx].id).houses as usize
        } else {
            0
        };
        land_cap + houses * HOUSE_MEMBER_CAPACITY
    }

    fn clan_can_add_member(&self, clan_idx: usize) -> bool {
        (self.clan_roster_size(clan_idx) as usize) < self.clan_member_cap(clan_idx)
    }

    fn settlement_index(&self, clan_id: i32) -> Option<usize> {
        self.settlements
            .binary_search_by_key(&clan_id, |state| state.clan_id)
            .ok()
    }

    fn ensure_settlement_index(&mut self, clan_id: i32) -> usize {
        match self
            .settlements
            .binary_search_by_key(&clan_id, |state| state.clan_id)
        {
            Ok(index) => index,
            Err(index) => {
                self.settlements.insert(
                    index,
                    ClanSettlement {
                        clan_id,
                        ..ClanSettlement::default()
                    },
                );
                index
            }
        }
    }

    fn settlement_tech(&self, clan_id: i32) -> TechState {
        self.settlement_index(clan_id)
            .map_or(TechState::default(), |index| self.settlements[index].tech)
    }

    fn settlement_signals(&self, clan_idx: usize) -> (f32, f32) {
        if !self.community_settlement {
            return (0.0, 0.0);
        }
        let clan_id = self.clans[clan_idx].id;
        let pop = self.clan_roster_size(clan_idx).max(1) as f32;
        let development =
            (development_score(&self.buildings, clan_id) as f32 / (pop * 2.0)).clamp(0.0, 1.0);
        let technology = self.settlement_tech(clan_id).level as f32 / MAX_TECH_LEVEL.max(1) as f32;
        (development, technology)
    }

    fn reserve_capacity(&self, clan_idx: usize) -> i32 {
        let pop = self.clan_roster_size(clan_idx);
        let granaries = if self.community_settlement {
            active_building_counts(&self.buildings, self.clans[clan_idx].id).granaries as i32
        } else {
            0
        };
        pop * RESERVE_FOOD_PER_MEMBER + granaries * GRANARY_RESERVE_CAPACITY
    }

    /// A live treatment toggle must not keep food that exists only because of
    /// granary capacity. Structural settlement state remains available if the
    /// treatment is re-enabled, but excess protected food is removed immediately.
    fn enforce_settlement_ablation_limits(&mut self) {
        if self.community_settlement {
            return;
        }
        let caps: Vec<i32> = (0..self.clans.len())
            .map(|idx| self.clan_roster_size(idx) * RESERVE_FOOD_PER_MEMBER)
            .collect();
        for (clan, cap) in self.clans.iter_mut().zip(caps) {
            clan.reserve_food = clan.reserve_food.min(cap);
        }
    }

    fn military_index(&self, clan_id: i32) -> Option<usize> {
        self.militaries
            .binary_search_by_key(&clan_id, |state| state.clan_id)
            .ok()
    }

    fn ensure_military_index(&mut self, clan_id: i32) -> usize {
        match self
            .militaries
            .binary_search_by_key(&clan_id, |state| state.clan_id)
        {
            Ok(index) => index,
            Err(index) => {
                self.militaries.insert(index, ClanMilitary::new(clan_id));
                index
            }
        }
    }

    fn military_safety_ready(&self, clan_idx: usize) -> bool {
        let pop = self.clan_roster_size(clan_idx);
        pop >= 4
            && self.clans[clan_idx].food >= pop * STOCKPILE_FOOD_PER_MEMBER
            && self.clans[clan_idx].reserve_food >= pop * RESERVE_FOOD_PER_MEMBER
    }

    fn active_workshop(&self, clan_id: i32) -> Option<(i32, i32)> {
        self.buildings
            .iter()
            .filter(|building| {
                building.clan_id == clan_id
                    && building.kind == BuildingKind::Workshop
                    && building.is_active()
            })
            .min_by_key(|building| building.id)
            .map(Building::position)
    }

    fn military_signals(&self, clan_idx: usize) -> (f32, f32) {
        if !self.community_military {
            return (0.0, 0.0);
        }
        let clan_id = self.clans[clan_idx].id;
        let active_ids: Vec<u32> = self
            .entities
            .iter()
            .filter(|entity| entity.clan == clan_id && entity.is_active())
            .map(|entity| entity.id)
            .collect();
        let bonus: u32 = active_ids
            .iter()
            .filter_map(|&id| equipment_for(&self.equipment, id))
            .map(|loadout| loadout.strength_milli().saturating_sub(1000) as u32)
            .sum();
        let strength = (bonus as f32 / (active_ids.len().max(1) as f32 * 700.0)).min(1.0);
        let stored = self
            .military_index(clan_id)
            .map_or(0, |index| self.militaries[index].ore_stockpile.max(0));
        let reachable: i32 = self
            .ore_deposits
            .iter()
            .filter(|deposit| {
                !deposit.is_depleted()
                    && self.clans[clan_idx].stockpile.is_some_and(|(x, y)| {
                        (deposit.x - x).abs().max((deposit.y - y).abs())
                            <= self.params.home_range.max(1)
                    })
                    && !self.is_foreign_tile(deposit.x, deposit.y, clan_id)
            })
            .map(|deposit| deposit.remaining as i32)
            .sum();
        let ore = ((stored + reachable) as f32
            / (active_ids.len().max(1) as f32 * ORE_DEPOSIT_AMOUNT as f32))
            .min(1.0);
        (strength, ore)
    }

    fn nearest_ore_deposit(&self, entity: &Entity, radius: i32) -> Option<usize> {
        self.ore_deposits
            .iter()
            .enumerate()
            .filter(|(_, deposit)| {
                !deposit.is_depleted()
                    && (deposit.x - entity.x)
                        .abs()
                        .max((deposit.y - entity.y).abs())
                        <= radius
                    && !self.is_foreign_tile(deposit.x, deposit.y, entity.clan)
            })
            .min_by_key(|(_, deposit)| {
                (
                    (deposit.x - entity.x)
                        .abs()
                        .max((deposit.y - entity.y).abs()),
                    deposit.id,
                )
            })
            .map(|(index, _)| index)
    }

    fn desired_equipment(&self, entity_id: u32, tech: u8) -> Option<EquipmentKind> {
        let loadout = equipment_for(&self.equipment, entity_id);
        if loadout.and_then(|gear| gear.weapon).is_none() {
            return Some(EquipmentKind::Spear);
        }
        if tech >= 1 && loadout.and_then(|gear| gear.weapon) == Some(EquipmentKind::Spear) {
            return Some(EquipmentKind::Sword);
        }
        if tech >= 2 && loadout.and_then(|gear| gear.armor).is_none() {
            return Some(EquipmentKind::Armor);
        }
        None
    }

    // --- spatial queries (bounded) ---
    /// Nearest *harvestable* pellet for a forager of `clan`. Prefers the clan's
    /// own farmland, then unclaimed wild food; foreign farmland is only returned
    /// when `allow_foreign` is set (an emergency steal). For neutrals (clan = -1)
    /// "own" and "unowned" coincide, so they harvest only the commons.
    fn nearest_pellet_harvestable(
        &self,
        x: i32,
        y: i32,
        radius: i32,
        clan: i32,
        allow_foreign: bool,
    ) -> Option<(i32, i32)> {
        let g = &self.grid;
        let x0 = (x - radius).max(0);
        let x1 = (x + radius).min(g.size - 1);
        let y0 = (y - radius).max(0);
        let y1 = (y + radius).min(g.size - 1);
        let mut best_own: Option<(i32, i32)> = None;
        let mut best_own_d = i32::MAX;
        let mut best_free: Option<(i32, i32)> = None;
        let mut best_free_d = i32::MAX;
        let mut best_foreign: Option<(i32, i32)> = None;
        let mut best_foreign_d = i32::MAX;
        for yy in y0..=y1 {
            let row = (yy * g.size) as usize;
            for xx in x0..=x1 {
                let i = row + xx as usize;
                if g.pellet[i] == 0 {
                    continue;
                }
                let dx = xx - x;
                let dy = yy - y;
                let d = dx * dx + dy * dy;
                let o = g.owner[i];
                if o == clan {
                    if d < best_own_d {
                        best_own_d = d;
                        best_own = Some((xx, yy));
                    }
                } else if o == NO_OWNER {
                    if d < best_free_d {
                        best_free_d = d;
                        best_free = Some((xx, yy));
                    }
                } else if allow_foreign && d < best_foreign_d {
                    best_foreign_d = d;
                    best_foreign = Some((xx, yy));
                }
            }
        }
        best_own
            .or(best_free)
            .or(if allow_foreign { best_foreign } else { None })
    }

    fn nearest_tree(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let mut best = None;
        let mut best_d = i64::MAX;
        for t in &self.trees {
            if t.destroyed {
                continue;
            }
            let dx = (t.x - x) as i64;
            let dy = (t.y - y) as i64;
            let d = dx * dx + dy * dy;
            if d < best_d {
                best_d = d;
                best = Some((t.x, t.y));
            }
        }
        best
    }

    fn nearest_wood(&self, x: i32, y: i32, radius: i32, clan: i32) -> Option<(i32, i32)> {
        let g = &self.grid;
        let mut best = None;
        let mut best_d = i32::MAX;
        for yy in (y - radius).max(0)..=(y + radius).min(g.size - 1) {
            for xx in (x - radius).max(0)..=(x + radius).min(g.size - 1) {
                let i = g.idx(xx, yy);
                if g.wood[i] == 0 {
                    continue;
                }
                let owner = g.owner[i];
                if owner != NO_OWNER && owner != clan {
                    continue;
                }
                let dx = xx - x;
                let dy = yy - y;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best = Some((xx, yy));
                }
            }
        }
        best
    }

    /// Local public-good signals for the unchanged reserved brain inputs.
    fn logistics_signals(&self, clan_idx: usize) -> (f32, f32) {
        if !self.params.community_logistics {
            return (0.0, 0.0);
        }
        let Some((sx, sy)) = self.clans[clan_idx].stockpile else {
            return (0.0, 0.0);
        };
        let id = self.clans[clan_idx].id;
        let r = self.params.home_range.max(1);
        let mut owned = 0u32;
        let mut roads = 0u32;
        let mut wood = 0u32;
        for y in (sy - r).max(0)..=(sy + r).min(self.grid.size - 1) {
            for x in (sx - r).max(0)..=(sx + r).min(self.grid.size - 1) {
                let i = self.grid.idx(x, y);
                let owner = self.grid.owner[i];
                if owner == id {
                    owned += 1;
                    roads += u32::from(self.grid.road[i] > 0);
                }
                if owner == NO_OWNER || owner == id {
                    wood += self.grid.wood[i] as u32;
                }
            }
        }
        let road_access = roads as f32 / owned.max(1) as f32;
        let pop = self.clan_roster_size(clan_idx).max(1) as f32;
        let available_wood = (wood as f32 / (pop * FOREST_WOOD_CAP as f32 * 3.0)).min(1.0);
        (road_access.min(1.0), available_wood)
    }

    pub fn entity_near(&self, x: i32, y: i32, radius: i32) -> Option<u32> {
        let mut best = None;
        let mut best_d = (radius * radius) + 1;
        for e in &self.entities {
            let dx = e.x - x;
            let dy = e.y - y;
            let d = dx * dx + dy * dy;
            if d <= best_d {
                best_d = d;
                best = Some(e.id);
            }
        }
        best
    }

    // --- terrain-aware movement ---
    fn move_cost_to(&self, x: i32, y: i32) -> f32 {
        if !self.grid.in_bounds(x, y) {
            return f32::INFINITY;
        }
        let i = self.grid.idx(x, y);
        let mut c = terrain_move_cost(self.grid.terrain[i]);
        if self.params.community_logistics && self.grid.road[i] > 0 {
            c *= 0.5; // roads retain dependable travel through winter
        } else {
            c *= self.seasonal_offroad_multiplier();
        }
        c
    }

    fn rebuild_occupancy(&mut self, entities: &[Entity]) {
        let n = (self.grid.size * self.grid.size) as usize;
        if self.occupied.len() != n {
            self.occupied = vec![0u16; n];
        } else {
            for v in self.occupied.iter_mut() {
                *v = 0;
            }
        }
        for e in entities {
            if !e.dead {
                let i = self.grid.idx(e.x, e.y);
                self.occupied[i] = self.occupied[i].saturating_add(1);
            }
        }
    }

    /// A cell can be entered if it is passable terrain and remains below capacity.
    fn can_enter(&self, x: i32, y: i32) -> bool {
        self.move_cost_to(x, y).is_finite()
            && self.occupied[self.grid.idx(x, y)] < MAX_ENTITIES_PER_CELL
    }

    fn try_step(&mut self, e: &mut Entity, nx: i32, ny: i32) -> bool {
        let c = self.move_cost_to(nx, ny);
        if !c.is_finite() || e.move_budget < c {
            return false; // impassable, or can't afford yet — wait, keep budget
        }
        let ni = self.grid.idx(nx, ny);
        if self.occupied[ni] >= MAX_ENTITIES_PER_CELL {
            return false; // the destination cell is full
        }
        let oi = self.grid.idx(e.x, e.y);
        self.occupied[oi] = self.occupied[oi].saturating_sub(1);
        self.occupied[ni] += 1;
        self.grid.traffic[ni] = self.grid.traffic[ni].saturating_add(1);
        if self.params.community_logistics && self.grid.road[ni] > 0 && e.clan >= 0 {
            let saved =
                road_cost_saved_milli(self.grid.terrain[ni], self.seasonal_offroad_multiplier());
            if let Some(cidx) = self.clan_index(e.clan) {
                self.clans[cidx].stats.road_steps += 1;
                self.clans[cidx].stats.road_cost_saved_milli += saved;
            }
        }
        e.move_budget -= c;
        e.x = nx;
        e.y = ny;
        true
    }

    /// Greedy step toward a target. Sidesteps impassable terrain and occupied
    /// tiles, and when `avoid_foreign` is set prefers steps that stay out of
    /// other clans' territory — only crossing it if there's no other way.
    fn move_toward(&mut self, e: &mut Entity, tx: i32, ty: i32, avoid_foreign: bool) {
        if e.x == tx && e.y == ty {
            return;
        }

        let mut best_clear: Option<(i32, i32, f32)> = None;
        let mut best_any: Option<(i32, i32, f32)> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = self.grid.clamp(e.x + dx);
                let ny = self.grid.clamp(e.y + dy);
                if (nx, ny) == (e.x, e.y) || !self.can_enter(nx, ny) {
                    continue;
                }
                let ddx = (tx - nx) as f32;
                let ddy = (ty - ny) as f32;
                let score = ddx * ddx + ddy * ddy + self.move_cost_to(nx, ny) * 0.25;
                if best_any.map_or(true, |(_, _, s)| score < s) {
                    best_any = Some((nx, ny, score));
                }
                if !avoid_foreign || !self.is_foreign_tile(nx, ny, e.clan) {
                    if best_clear.map_or(true, |(_, _, s)| score < s) {
                        best_clear = Some((nx, ny, score));
                    }
                }
            }
        }

        if let Some((nx, ny, _)) = best_clear.or(best_any) {
            self.try_step(e, nx, ny);
        }
    }

    fn random_walk(&mut self, e: &mut Entity, avoid_foreign: bool) {
        let dx = self.rng.step();
        let dy = self.rng.step();
        if dx == 0 && dy == 0 {
            return;
        }
        let nx = self.grid.clamp(e.x + dx);
        let ny = self.grid.clamp(e.y + dy);
        if (nx, ny) == (e.x, e.y) || !self.can_enter(nx, ny) {
            return;
        }
        if avoid_foreign && self.is_foreign_tile(nx, ny, e.clan) {
            return; // don't idle into someone else's land
        }
        self.try_step(e, nx, ny);
    }

    // --- per-tick ---
    pub fn step(&mut self) {
        self.tick += 1;
        self.enforce_settlement_ablation_limits();
        // Farms (owned land) get first call on the food budget; wild trees fill
        // what's left — so cultivated territory out-produces the wilderness.
        self.grow_farms();
        self.update_trees();
        self.regrow_wood();
        self.maybe_disaster();
        self.refresh_diplomacy();
        self.clan_think();
        self.plan_settlement_projects();
        self.advance_workshop_research();
        self.plan_military_work();
        self.prepare_rescues();

        let mut entities = std::mem::take(&mut self.entities);
        self.rebuild_occupancy(&entities);
        for e in entities.iter_mut() {
            self.update_entity(e);
        }
        self.entities = entities;
        self.advance_rescues();
        self.build_roads();
        self.record_military_readiness();

        self.resolve_recruitment();
        self.resolve_combat();
        self.resolve_raiding();
        self.detach_dead();
        self.entities.retain(|e| !e.dead);
        self.clans.retain(|c| !c.disbanded);
        self.record_clan_stats();
        self.maintain();

        if self.params.birth_interval > 0 && self.tick % self.params.birth_interval == 0 {
            self.reproduce();
        }
        if self.tick % TERRITORY_PRUNE_INTERVAL == 0 {
            self.prune_territory();
        }
    }

    fn record_military_readiness(&mut self) {
        if !self.community_military {
            return;
        }
        let active_clans: HashMap<u32, i32> = self
            .entities
            .iter()
            .filter(|entity| entity.is_active() && entity.clan >= 0)
            .map(|entity| (entity.id, entity.clan))
            .collect();
        let mut counts: HashMap<i32, u64> = HashMap::new();
        for loadout in &self.equipment {
            if loadout.weapon.is_none() && loadout.armor.is_none() {
                continue;
            }
            if let Some(&clan_id) = active_clans.get(&loadout.entity_id) {
                *counts.entry(clan_id).or_default() += 1;
            }
        }
        for (clan_id, count) in counts {
            let state = self.ensure_military_index(clan_id);
            self.militaries[state].record_equipped_member_ticks(count);
        }
    }

    fn record_clan_stats(&mut self) {
        if self.clans.is_empty() {
            return;
        }
        let starve_ticks = self.params.starve_ticks.max(1) as f32;
        let mut idx_by_id: HashMap<i32, usize> = HashMap::new();
        for (i, c) in self.clans.iter().enumerate() {
            if !c.disbanded {
                idx_by_id.insert(c.id, i);
            }
        }
        let mut pop = vec![0u32; self.clans.len()];
        let mut hunger_sum = vec![0f32; self.clans.len()];
        let mut starving = vec![0u32; self.clans.len()];
        let mut on_terr = vec![0u32; self.clans.len()];
        let mut roles = vec![[0u32; N_MODES]; self.clans.len()];
        for e in &self.entities {
            if !e.is_active() || e.clan < 0 {
                continue;
            }
            if let Some(&idx) = idx_by_id.get(&e.clan) {
                let h = (e.ticks_since_food as f32 / starve_ticks).clamp(0.0, 2.0);
                pop[idx] += 1;
                hunger_sum[idx] += h;
                if h >= 1.0 {
                    starving[idx] += 1;
                }
                if self.grid.owner[self.grid.idx(e.x, e.y)] == e.clan {
                    on_terr[idx] += 1;
                }
                roles[idx][e.work_role.index()] += 1;
            }
        }
        for i in 0..self.clans.len() {
            if self.clans[i].disbanded {
                continue;
            }
            let p = pop[i];
            self.clans[i].stats.alive_ticks += 1;
            self.clans[i].stats.pop_tick_sum += p as u64;
            self.clans[i].stats.hunger_tick_sum += hunger_sum[i];
            self.clans[i].stats.starving_ticks += starving[i];
            let reserve = if self.params.community_logistics {
                self.clans[i].reserve_food.max(0)
            } else {
                0
            };
            self.clans[i].stats.food_tick_sum += (self.clans[i].food.max(0) + reserve) as u64;
            self.clans[i].stats.on_terr_tick_sum += on_terr[i] as u64;
            for role in 0..N_MODES {
                self.clans[i].stats.role_tick_sum[role] += roles[i][role] as u64;
            }
            self.clans[i].stats.peak_pop = self.clans[i].stats.peak_pop.max(p);
        }
    }

    /// Population growth: per reproduction check, each pair of NPCs has a
    /// variable chance to produce a child when food is available. Clans pay
    /// from their stockpile; neutrals breed only on a clear map-food surplus.
    fn reproduce(&mut self) {
        let chance = self.params.birth_chance;
        if chance <= 0.0 {
            return;
        }
        let cost = self.params.birth_food_cost.max(0);

        for ci in 0..self.clans.len() {
            if self.clans[ci].disbanded {
                continue;
            }
            if !self.clan_can_add_member(ci) {
                continue;
            }
            let id = self.clans[ci].id;
            let starve_ticks = self.params.starve_ticks.max(1);
            let mut pop = 0i32;
            let mut hunger_sum = 0.0f32;
            let mut has_starving_member = false;
            for e in &self.entities {
                if e.clan != id || !e.is_active() {
                    continue;
                }
                let hunger = e.hunger(starve_ticks).clamp(0.0, 2.0);
                pop += 1;
                hunger_sum += hunger;
                has_starving_member |= hunger >= 0.85;
            }
            if pop < 2 {
                continue;
            }
            let avg_hunger = hunger_sum / pop as f32;
            if avg_hunger > 0.32 || has_starving_member {
                continue;
            }
            let reserve_floor = (pop * 3).max(cost + 2);
            if self.clans[ci].food <= reserve_floor {
                continue;
            }
            let pairs = pop / 2;
            let max_births = ((pop + 9) / 10).clamp(1, 4);
            let mut births_this_check = 0;
            let reserve_pressure = ((self.clans[ci].food - reserve_floor) as f32
                / (pop * 8).max(1) as f32)
                .clamp(0.0, 1.0);
            // Prosperity never boosts the configured rate, while the falling
            // and lean quarters prevent a famine-driven baby boom.
            let birth_chance = chance * reserve_pressure * self.seasonal_birth_multiplier();
            let base = match self.clans[ci].stockpile {
                Some(p) => Some(p),
                None => self.entity_pos(self.clans[ci].leader_id),
            };
            let (sx, sy) = match base {
                Some(p) => p,
                None => continue,
            };
            for _ in 0..pairs {
                if births_this_check >= max_births
                    || !self.clan_can_add_member(ci)
                    || self.clans[ci].food < reserve_floor + cost
                {
                    break;
                }
                if self.rng.chance(birth_chance) {
                    let Some((bx, by)) = self.nearby_spawn_cell(sx, sy, 2) else {
                        break;
                    };
                    self.clans[ci].food -= cost;
                    let mut baby = self.make_entity(bx, by, false);
                    baby.clan = id;
                    let bid = baby.id;
                    self.entities.push(baby);
                    self.clans[ci].members.push(bid);
                    self.births += 1;
                    births_this_check += 1;
                }
            }
        }

        let neutrals: Vec<(i32, i32)> = self
            .entities
            .iter()
            .filter(|e| e.clan < 0 && e.is_active())
            .map(|e| (e.x, e.y))
            .collect();
        if self.pellet_total > neutrals.len() * 6 && neutrals.len() >= 2 {
            let pairs = neutrals.len() / 2;
            let mut births_this_check = 0;
            let max_births = ((neutrals.len() as i32 + 19) / 20).clamp(1, 2);
            for _ in 0..pairs {
                if births_this_check >= max_births {
                    break;
                }
                if self.rng.chance(chance * 0.04) {
                    let k = self.rng.below(neutrals.len() as i32) as usize;
                    let (nx, ny) = neutrals[k];
                    if let Some((bx, by)) = self.nearby_spawn_cell(nx, ny, 2) {
                        self.spawn_entity(bx, by, false);
                        self.births += 1;
                        births_this_check += 1;
                    }
                }
            }
        }
    }

    fn maintain(&mut self) {
        if self.tick % 20 != 0 {
            return;
        }
        // Clan floor: when war thins the field, a new village coalesces from
        // masterless refugees (or, failing enough of them, fresh settlers).
        if self.maintain_clans > 0 {
            let mut tries = 0;
            while (self.clan_count() as i32) < self.maintain_clans && tries < 2 {
                self.form_refugee_clan();
                tries += 1;
            }
        }
        if self.maintain_pop > 0 {
            while (self.entities.len() as i32) < self.maintain_pop {
                let spawned = if let Some(ci) = self
                    .clans
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !c.disbanded)
                    .filter(|(i, _)| self.clan_can_add_member(*i))
                    .min_by_key(|(_, c)| self.clan_population(c.id))
                    .map(|(i, _)| i)
                {
                    let id = self.clans[ci].id;
                    let base = self.clans[ci]
                        .stockpile
                        .or_else(|| self.entity_pos(self.clans[ci].leader_id))
                        .or_else(|| self.random_land_cell());
                    let Some((x, y)) = base.and_then(|(sx, sy)| self.nearby_spawn_cell(sx, sy, 4))
                    else {
                        break;
                    };
                    let mut e = self.make_entity(x, y, false);
                    e.clan = id;
                    let eid = e.id;
                    self.entities.push(e);
                    self.clans[ci].members.push(eid);
                    self.clans[ci].food = self.clans[ci].food.max(4);
                    true
                } else {
                    self.spawn_clan()
                };
                if !spawned {
                    break;
                }
            }
        }
    }

    /// Form a new village. Preferred path: a band of masterless refugees on
    /// unclaimed land elects a leader and settles — so defeated peoples and the
    /// wandering poor coalesce back into villages instead of roaming forever.
    /// Falls back to fresh settlers when there aren't enough refugees.
    fn form_refugee_clan(&mut self) {
        let neutrals: Vec<usize> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.clan < 0 && e.is_active())
            .map(|(i, _)| i)
            .collect();
        if neutrals.len() < 4 {
            self.spawn_clan(); // not enough refugees — seed fresh settlers
            return;
        }
        // Pick a seed refugee on the most fertile, unclaimed spot among samples.
        let n = neutrals.len() as i32;
        let mut seed = neutrals[self.rng.below(n) as usize];
        let mut seed_f = -1i32;
        for _ in 0..6 {
            let k = neutrals[self.rng.below(n) as usize];
            let (x, y) = (self.entities[k].x, self.entities[k].y);
            let i = self.grid.idx(x, y);
            if self.grid.owner[i] != NO_OWNER {
                continue;
            }
            let f = self.grid.fertility[i] as i32;
            if f > seed_f {
                seed_f = f;
                seed = k;
            }
        }
        let (lx, ly) = (self.entities[seed].x, self.entities[seed].y);
        if self.grid.owner[self.grid.idx(lx, ly)] != NO_OWNER {
            return; // chosen spot got claimed; try again next tick
        }
        let leader_id = self.entities[seed].id;
        let brain = self.breed_brain();
        let id = self.create_clan(leader_id, lx, ly, brain);
        self.entities[seed].clan = id;
        self.entities[seed].is_leader = true;
        self.entities[seed].max_health = self.params.leader_health;
        self.entities[seed].health = self.params.leader_health;
        self.entities[seed].last_food = None;
        let idx = self.clan_index(id).unwrap();
        // The nearest few refugees join the new village.
        let mut others: Vec<usize> = neutrals.into_iter().filter(|&i| i != seed).collect();
        others.sort_by_key(|&i| {
            let dx = (self.entities[i].x - lx) as i64;
            let dy = (self.entities[i].y - ly) as i64;
            dx * dx + dy * dy
        });
        for &i in others.iter().take(4) {
            let d = (self.entities[i].x - lx)
                .abs()
                .max((self.entities[i].y - ly).abs());
            if d <= 24 {
                self.entities[i].clan = id;
                self.entities[i].last_food = None;
                let mid = self.entities[i].id;
                self.clans[idx].members.push(mid);
            }
        }
    }

    fn refresh_diplomacy(&mut self) {
        if self.tick % TRADE_REFRESH_INTERVAL != 0 {
            return;
        }
        let live_ids: Vec<i32> = self
            .clans
            .iter()
            .filter(|clan| !clan.disbanded)
            .map(|clan| clan.id)
            .collect();
        self.diplomacy.prune(&live_ids);
        self.diplomacy.decay(self.tick, 0.995, 0.85);
        if !self.params.community_trade {
            for clan in &mut self.clans {
                clan.trade_partner = None;
                clan.trade_route_threat = None;
            }
            return;
        }

        let homes: Vec<(usize, i32, i32, i32)> = self
            .clans
            .iter()
            .enumerate()
            .filter_map(|(index, clan)| {
                (!clan.disbanded)
                    .then_some(clan.stockpile.map(|home| (index, clan.id, home.0, home.1)))
                    .flatten()
            })
            .collect();
        let mut partners = vec![None; self.clans.len()];
        for &(index, clan_id, x, y) in &homes {
            let mut best = None;
            for &(_, other_id, ox, oy) in &homes {
                if clan_id == other_id {
                    continue;
                }
                let distance = (x - ox).abs().max((y - oy).abs());
                if distance > TRADE_RANGE {
                    continue;
                }
                let trust = self
                    .diplomacy
                    .lookup(clan_id, other_id)
                    .map_or(0.0, |relation| relation.trust);
                if trust < -0.25 {
                    continue;
                }
                let key = (distance, other_id);
                if best.map_or(true, |current| key < current) {
                    best = Some(key);
                }
            }
            partners[index] = best.map(|(_, id)| id);
        }

        for (index, partner) in partners.into_iter().enumerate() {
            self.clans[index].trade_partner = partner;
            let Some(partner_id) = partner else {
                self.clans[index].trade_route_threat = None;
                continue;
            };
            let Some(home) = self.clans[index].stockpile else {
                continue;
            };
            let Some(partner_home) = self.clan_by_id(partner_id).and_then(|clan| clan.stockpile)
            else {
                continue;
            };
            let clan_id = self.clans[index].id;
            let threat = self
                .entities
                .iter()
                .filter(|entity| self.is_trade_route_hostile(clan_id, partner_id, entity))
                .min_by_key(|entity| {
                    (
                        route_distance_sq((entity.x, entity.y), home, partner_home),
                        entity.id,
                    )
                })
                .filter(|entity| route_distance_sq((entity.x, entity.y), home, partner_home) <= 36)
                .map(|entity| entity.id);
            self.clans[index].trade_route_threat = threat;
        }
    }

    fn are_allied(&self, first: i32, second: i32) -> bool {
        if !self.params.community_trade {
            return first == second && first >= 0;
        }
        if first < 0 || second < 0 || first == second {
            return first == second && first >= 0;
        }
        self.diplomacy
            .lookup(first, second)
            .is_some_and(|relation| {
                relation.trust >= TRADE_ALLY_TRUST || relation.pact_active(self.tick)
            })
    }

    fn has_trade_passage(&self, entity: &Entity, host_clan: i32) -> bool {
        self.params.community_trade
            && entity.is_active()
            && entity.trade_target_clan == host_clan
            && (entity.trade_food > 0 || entity.trade_wood > 0 || entity.trade_returning)
    }

    fn is_trade_route_hostile(&self, clan_id: i32, partner_id: i32, entity: &Entity) -> bool {
        if !entity.is_active()
            || entity.clan < 0
            || entity.clan == clan_id
            || entity.clan == partner_id
            || self.are_allied(clan_id, entity.clan)
        {
            return false;
        }
        let trust = self
            .diplomacy
            .lookup(clan_id, entity.clan)
            .map_or(0.0, |relation| relation.trust);
        entity.work_role == ClanMode::Attack
            || trust < 0.0
            || self.clan_aggr(clan_id) + self.clan_aggr(entity.clan) >= self.params.war_threshold
    }

    fn trade_signals(&self, clan_idx: usize) -> (f32, f32, f32) {
        if !self.params.community_trade {
            return (0.0, 0.0, 0.0);
        }
        let clan_id = self.clans[clan_idx].id;
        let relation = self.clans[clan_idx]
            .trade_partner
            .and_then(|partner| self.diplomacy.lookup(clan_id, partner))
            .map_or(0.0, |entry| (entry.trust + 1.0) * 0.5);
        let mut partners = 0usize;
        let mut recent_volume = 0.0;
        for entry in self.diplomacy.relationships() {
            if entry.clan_low != clan_id && entry.clan_high != clan_id {
                continue;
            }
            if entry.trust >= TRADE_ALLY_TRUST || entry.pact_active(self.tick) {
                partners += 1;
            }
            recent_volume += entry.recent_food_delivered + entry.recent_wood_delivered;
        }
        (
            relation.clamp(0.0, 1.0),
            (partners as f32 / 3.0).min(1.0),
            recent_volume / (recent_volume + 8.0),
        )
    }

    fn clan_think(&mut self) {
        let refresh = self.tick % TARGET_REFRESH_INTERVAL == 0;
        let decide = self.tick % CLAN_THINK_INTERVAL == 0;
        if !refresh && !decide || self.clans.is_empty() {
            return;
        }

        let starve_ticks = self.params.starve_ticks.max(1) as f32;
        let vision = self.params.vision_radius;
        let grace = self.tick < self.params.clan_grace_ticks;

        let mut idx_by_id: HashMap<i32, usize> = HashMap::new();
        for (i, c) in self.clans.iter().enumerate() {
            if !c.disbanded {
                idx_by_id.insert(c.id, i);
            }
        }
        let n = self.clans.len();
        let mut leader_pos: Vec<Option<(i32, i32)>> = vec![None; n];
        let mut pop = vec![0u32; n];
        let mut hunger_sum = vec![0f32; n];
        let mut health_sum = vec![0f32; n];
        let mut health_count = vec![0u32; n];
        let mut clan_members: Vec<(i32, i32, i32, i32, bool, bool)> = Vec::new();
        let mut neutrals: Vec<(u32, i32, i32, bool)> = Vec::new();

        for e in &self.entities {
            if e.dead {
                continue;
            }
            if e.clan >= 0 {
                if let Some(&idx) = idx_by_id.get(&e.clan) {
                    health_sum[idx] += (e.health / e.max_health.max(f32::EPSILON)).clamp(0.0, 1.0);
                    health_count[idx] += 1;
                    if !e.is_active() {
                        continue;
                    }
                    pop[idx] += 1;
                    hunger_sum[idx] += (e.ticks_since_food as f32 / starve_ticks).clamp(0.0, 2.0);
                    if e.is_leader {
                        leader_pos[idx] = Some((e.x, e.y));
                    }
                }
                clan_members.push((
                    e.x,
                    e.y,
                    e.clan,
                    e.trade_target_clan,
                    e.trade_food > 0 || e.trade_wood > 0 || e.trade_returning,
                    e.goal == Goal::Hiding,
                ));
            } else {
                neutrals.push((e.id, e.x, e.y, e.goal == Goal::Hiding));
            }
        }
        let max_pop = pop.iter().copied().max().unwrap_or(1).max(1) as f32;

        struct Dec {
            idx: usize,
            enemy_pos: Option<(i32, i32)>,
            recruit_target: Option<u32>,
            neutral_pos: Option<(i32, i32)>,
            trespasser_pos: Option<(i32, i32)>,
            expand_target: Option<(i32, i32)>,
            pop: u32,
            decided: Option<(
                ClanMode,
                f32,
                [f32; N_OUT],
                [f32; N_EXPERTS],
                [bool; N_MODES],
            )>,
        }
        let mut decs: Vec<Dec> = Vec::with_capacity(n);

        for idx in 0..n {
            if self.clans[idx].disbanded {
                continue;
            }
            let lp = match leader_pos[idx] {
                Some(p) => p,
                None => continue,
            };
            let id = self.clans[idx].id;

            let mut enemy = None;
            let mut ed = i64::MAX;
            let mut enemies_seen = 0;
            let mut tres = None;
            let mut td = i64::MAX;
            for &(ex, ey, ec, trade_target, active_trade, hidden) in &clan_members {
                if ec == id
                    || self.are_allied(id, ec)
                    || self.params.community_trade && active_trade && trade_target == id
                {
                    continue;
                }
                let dx = (ex - lp.0) as i64;
                let dy = (ey - lp.1) as i64;
                let d = dx * dx + dy * dy;
                let target_vision = detection_radius(vision, hidden);
                let target_vision2 = (target_vision * target_vision) as i64;
                if d > target_vision2 {
                    continue;
                }
                enemies_seen += 1;
                if d < ed {
                    ed = d;
                    enemy = Some((ex, ey));
                }
                if self.grid.owner[self.grid.idx(ex, ey)] == id && d < td {
                    td = d;
                    tres = Some((ex, ey));
                }
            }
            let mut enemy_stockpile = None;
            let mut sd = i64::MAX;
            for c in &self.clans {
                if c.disbanded || c.id == id || c.food <= 0 || self.are_allied(id, c.id) {
                    continue;
                }
                if let Some((sx, sy)) = c.stockpile {
                    let dx = (sx - lp.0) as i64;
                    let dy = (sy - lp.1) as i64;
                    let d = dx * dx + dy * dy;
                    if d < sd {
                        sd = d;
                        enemy_stockpile = Some((sx, sy));
                    }
                }
            }
            let mut ntarget = None;
            let mut npos = None;
            let mut nd = i64::MAX;
            let mut neutrals_seen = 0;
            for &(neutral_id, nx, ny, hidden) in &neutrals {
                let dx = (nx - lp.0) as i64;
                let dy = (ny - lp.1) as i64;
                let d = dx * dx + dy * dy;
                let target_vision = detection_radius(vision, hidden);
                if d > (target_vision * target_vision) as i64 {
                    continue;
                }
                neutrals_seen += 1;
                if self.grid.owner[self.grid.idx(nx, ny)] == id && d < td {
                    td = d;
                    tres = Some((nx, ny));
                }
                if d < nd {
                    nd = d;
                    ntarget = Some(neutral_id);
                    npos = Some((nx, ny));
                }
            }

            // The clan's frontier — the next, best (fertile, close) tile to claim.
            // Computed every refresh so Expand always has a fresh target and the
            // decision below can tell whether the clan is boxed in.
            let frontier = self.find_frontier(idx, lp);
            let neighbor2 = (EXPAND_REACH as i64 * 2) * (EXPAND_REACH as i64 * 2);
            let enemy_near = ed.min(sd) <= neighbor2;

            let decided = if decide {
                let size = pop[idx] as f32;
                let reserve = if self.params.community_logistics {
                    self.clans[idx].reserve_food
                } else {
                    0
                };
                let food = (self.clans[idx].food + reserve) as f32;
                let terr = self.clans[idx].territory as f32;
                let avg_hunger = if pop[idx] > 0 {
                    hunger_sum[idx] / pop[idx] as f32
                } else {
                    0.0
                };
                let avg_health = if health_count[idx] > 0 {
                    health_sum[idx] / health_count[idx] as f32
                } else {
                    0.0
                };
                // how full is the clan relative to the population its land supports
                let cap = self.clan_member_cap(idx) as f32;
                let crowd = size / cap.max(1.0);
                let headroom = ((cap - size) / cap.max(1.0)).clamp(0.0, 1.0);
                let frontier_exists = frontier.is_some();
                let food_signal = match self.nearest_tree(lp.0, lp.1) {
                    Some((tx, ty)) => {
                        let dist = (((tx - lp.0).pow(2) + (ty - lp.1).pow(2)) as f32).sqrt();
                        (1.0 - dist / (self.grid.size as f32 * 0.55)).clamp(0.0, 1.0)
                    }
                    None => 0.0,
                };
                // global food "climate": how full the world's food budget is.
                // Low = famine — a situation a sub-mind can learn to handle.
                let cells = (self.grid.size as i64 * self.grid.size as i64) as f32;
                let max_pellets =
                    (cells * self.params.max_pellet_fraction.clamp(0.0, 1.0)).max(1.0);
                let world_food = (self.pellet_total as f32 / max_pellets).min(1.0);
                let (road_access, available_wood) = self.logistics_signals(idx);
                let (trade_relation, trade_partners, trade_volume) = self.trade_signals(idx);
                let (settlement_development, technology) = self.settlement_signals(idx);
                let (military_strength, ore_access) = self.military_signals(idx);
                let season_yield = self.season_factor();

                // The situation vector the master + sub-minds read. Normalised,
                // information-rich, and free of behavioural prescriptions — what
                // to DO with it is entirely up to the evolved networks.
                let inputs: [f32; crate::brain::N_IN] = [
                    (size / 25.0).min(1.0),                                  // 0 population
                    (food / (size * 4.0).max(1.0)).min(1.0),                 // 1 stored food / head
                    avg_hunger.min(1.0),                                     // 2 hunger
                    crowd.min(1.0), // 3 crowding vs land cap
                    headroom,       // 4 room to grow
                    food_signal,    // 5 nearest wild food
                    (enemies_seen as f32 / (size * 2.0).max(1.0)).min(1.0), // 6 enemies in sight
                    (neutrals_seen as f32 / (size * 2.0).max(1.0)).min(1.0), // 7 recruits in sight
                    size / (size + max_pop), // 8 relative power
                    self.clans[idx].aggression, // 9 own aggression (feedback)
                    if grace { 1.0 } else { 0.0 }, // 10 peace grace active
                    (terr / 250.0).min(1.0), // 11 territory held
                    if frontier_exists { 1.0 } else { 0.0 }, // 12 can expand?
                    if enemy_near { 1.0 } else { 0.0 }, // 13 enemy nearby (threat)
                    world_food,     // 14 food climate (famine sense)
                    (season_yield - 1.0) / self.params.season_amp.max(0.01), // 15 seasonal yield [-1,1]
                    // --- RESERVED future-feature inputs (read 0.0 until the world
                    //     wires them up; the brain's size never changes) ---
                    road_access,            // 16 roads / logistics access near home
                    settlement_development, // 17 buildings / settlement development level
                    technology,             // 18 tech / research level
                    military_strength,      // 19 military strength / equipment level
                    if self.params.community_logistics {
                        (self.clans[idx].wood as f32 / (size * 3.0).max(1.0)).min(1.0)
                    } else {
                        0.0
                    }, // 20 stored wood / head
                    available_wood,         // 21 reachable forest wood
                    trade_relation,         // 22 relation with nearest trade partner
                    trade_partners,         // 23 allies / active trade partners
                    trade_volume,           // 24 recent delivered trade volume
                    0.0, // 25 reserved; live trend would destabilize the tracked champion
                    self.clans[idx].soil_depletion.min(1.0), // 26 soil depletion of owned tiles
                    self.disaster_level, // 27 active disaster/event severity
                    avg_health, // 28 average member health
                    0.0, // 29 water / coast / river access of territory
                    ore_access, // 30 stored / reachable mineral access
                    1.0, // 31 bias
                ];

                // The master routes; the sub-minds propose. We take the blended
                // action vector and pick the highest-utility *physically feasible*
                // mode (an action only needs a target to exist — no strategy gates).
                // Individual hunger foraging is handled per-entity, so a clan never
                // starves just because its leader chose a bold project.
                let (out, gate_w) = self.clans[idx].brain.evaluate(&inputs);
                let mut order: Vec<usize> = (0..N_MODES).collect();
                order.sort_by(|&a, &b| out[b].partial_cmp(&out[a]).unwrap());
                let feasible = [
                    ntarget.is_some() && self.clan_can_add_member(idx),
                    frontier_exists,
                    true,
                    !grace && enemy.is_some(),
                    true,
                    true,
                ];
                let mut chosen = ClanMode::Gather;
                for &oi in &order {
                    let m = ClanMode::from_index(oi);
                    if feasible[oi] {
                        chosen = m;
                        break;
                    }
                }
                // Aggression is its own output the master controls directly.
                let aggression = out[N_MODES].clamp(0.0, 1.0);
                if chosen == ClanMode::Attack {
                    // march on the richest target: a hoard if one is in reach.
                    enemy = enemy_stockpile.or(enemy);
                }
                Some((chosen, aggression, out, gate_w, feasible))
            } else {
                None
            };
            // Whatever the mode, a worker always needs the freshest frontier tile.
            let expand_target = frontier;

            decs.push(Dec {
                idx,
                enemy_pos: enemy,
                recruit_target: ntarget,
                neutral_pos: npos,
                trespasser_pos: tres,
                expand_target,
                pop: pop[idx],
                decided,
            });
        }

        let mut workforce_updates = Vec::new();
        for d in decs {
            {
                let c = &mut self.clans[d.idx];
                c.enemy_pos = d.enemy_pos;
                c.recruit_target = d.recruit_target;
                c.neutral_pos = d.neutral_pos;
                c.trespasser_pos = d.trespasser_pos;
                c.expand_target = d.expand_target;
                c.stats.peak_pop = c.stats.peak_pop.max(d.pop);
                if let Some((mode, aggr, out, gate_w, feasible)) = d.decided {
                    c.mode = mode;
                    c.aggression = aggr;
                    c.brain.last_out = out;
                    c.brain.last_gate = gate_w;
                    workforce_updates.push((d.idx, mode, out, feasible));
                }
            }
        }
        for (idx, mode, out, feasible) in workforce_updates {
            self.rebalance_workforce(idx, mode, &out, &feasible);
        }
    }

    /// Convert the blended utilities into deterministic integer jobs while
    /// preserving existing assignments whenever their quota still has room.
    fn rebalance_workforce(
        &mut self,
        clan_idx: usize,
        headline: ClanMode,
        utilities: &[f32; N_OUT],
        feasible: &[bool; N_MODES],
    ) {
        let clan_id = self.clans[clan_idx].id;
        let leader_id = self.clans[clan_idx].leader_id;
        let mut followers: Vec<usize> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.clan == clan_id && e.is_active() && e.id != leader_id)
            .map(|(i, _)| i)
            .collect();
        followers.sort_by_key(|&i| self.entities[i].id);

        let mut target = [0u16; N_MODES];
        let follower_count = followers.len();
        if follower_count > 0 {
            let gather_core = ((follower_count + 3) / 4).max(1).min(follower_count);
            target[ClanMode::Gather.index()] = gather_core as u16;
            let defend_core = usize::from(follower_count > gather_core);
            target[ClanMode::Defend.index()] = defend_core as u16;
            let remaining_slots = follower_count - gather_core - defend_core;
            let worker_modes = [
                ClanMode::Expand,
                ClanMode::Gather,
                ClanMode::Attack,
                ClanMode::Defend,
            ];
            let weight_sum: f32 = worker_modes
                .iter()
                .filter(|m| feasible[m.index()])
                .map(|m| utilities[m.index()].max(0.0))
                .sum();
            if remaining_slots > 0 && weight_sum > 0.0 {
                let mut used = 0usize;
                let mut fractions = Vec::new();
                for mode in worker_modes {
                    if !feasible[mode.index()] {
                        continue;
                    }
                    let exact =
                        remaining_slots as f32 * utilities[mode.index()].max(0.0) / weight_sum;
                    let whole = exact.floor() as usize;
                    target[mode.index()] += whole as u16;
                    used += whole;
                    fractions.push((exact - whole as f32, mode.index()));
                }
                fractions.sort_by(|a, b| {
                    b.0.partial_cmp(&a.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.1.cmp(&b.1))
                });
                for &(_, role) in fractions.iter().take(remaining_slots.saturating_sub(used)) {
                    target[role] += 1;
                }
            } else {
                target[ClanMode::Gather.index()] += remaining_slots as u16;
            }
        }

        let leader_idx = self
            .entities
            .iter()
            .position(|e| e.id == leader_id && e.clan == clan_id && e.is_active());
        if leader_idx.is_some() {
            target[headline.index()] += 1;
        }

        let mut remaining = target;
        let mut assignments: Vec<(usize, ClanMode)> = Vec::with_capacity(followers.len() + 1);
        if let Some(i) = leader_idx {
            assignments.push((i, headline));
            remaining[headline.index()] = remaining[headline.index()].saturating_sub(1);
        }

        for locked_pass in [true, false] {
            for &i in &followers {
                if assignments.iter().any(|(assigned, _)| *assigned == i) {
                    continue;
                }
                let locked = self.entities[i].work_until > self.tick;
                if locked != locked_pass {
                    continue;
                }
                let role = self.entities[i].work_role;
                if matches!(role, ClanMode::Recruit | ClanMode::Scout)
                    || !feasible[role.index()]
                    || remaining[role.index()] == 0
                {
                    continue;
                }
                assignments.push((i, role));
                remaining[role.index()] -= 1;
            }
        }

        let mut role_order = [
            ClanMode::Expand,
            ClanMode::Gather,
            ClanMode::Attack,
            ClanMode::Defend,
        ];
        role_order.sort_by(|a, b| {
            utilities[b.index()]
                .partial_cmp(&utilities[a.index()])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.index().cmp(&b.index()))
        });
        for i in followers {
            if assignments.iter().any(|(assigned, _)| *assigned == i) {
                continue;
            }
            let role = role_order
                .iter()
                .copied()
                .find(|m| remaining[m.index()] > 0)
                .unwrap_or(ClanMode::Gather);
            remaining[role.index()] = remaining[role.index()].saturating_sub(1);
            assignments.push((i, role));
        }

        let mut actual = [0u16; N_MODES];
        for (i, role) in assignments {
            actual[role.index()] += 1;
            if self.entities[i].work_role != role {
                self.entities[i].work_role = role;
                self.entities[i].work_until = self.tick + WORK_ASSIGNMENT_TICKS;
            }
        }
        self.clans[clan_idx].workforce = actual;
    }

    /// Best unowned passable tile on the clan's frontier — the next claim.
    /// Scored to prefer fertile farmland (worth owning) that's close to the
    /// stockpile, so villages grow toward the best land instead of sprawling
    /// randomly. Returns the frontier tile nearest the leader among the top
    /// candidates so workers actually reach it.
    fn find_frontier(&self, clan_idx: usize, leader: (i32, i32)) -> Option<(i32, i32)> {
        let id = self.clans[clan_idx].id;
        let (sx, sy) = self.clans[clan_idx].stockpile?;
        let mut best = None;
        let mut best_score = f32::MIN;
        let x0 = (sx - EXPAND_REACH).max(0);
        let x1 = (sx + EXPAND_REACH).min(self.grid.size - 1);
        let y0 = (sy - EXPAND_REACH).max(0);
        let y1 = (sy + EXPAND_REACH).min(self.grid.size - 1);
        let reach = (EXPAND_REACH * EXPAND_REACH) as f32;
        for yy in y0..=y1 {
            for xx in x0..=x1 {
                let i = self.grid.idx(xx, yy);
                if self.grid.owner[i] != NO_OWNER || !self.is_passable(xx, yy) {
                    continue;
                }
                if !self.has_owned_neighbor(id, xx, yy) {
                    continue;
                }
                let fert = self.grid.fertility[i] as f32 / 255.0;
                // distance from the stockpile (compactness) and from the leader
                let ds = ((xx - sx).pow(2) + (yy - sy).pow(2)) as f32;
                let dl = ((xx - leader.0).pow(2) + (yy - leader.1).pow(2)) as f32;
                // prefer fertile land, close to home, reachable by the leader
                let score = fert * 2.0 - (ds / reach) * 0.9 - (dl / reach) * 0.6;
                if score > best_score {
                    best_score = score;
                    best = Some((xx, yy));
                }
            }
        }
        best
    }

    fn drop_entity_resources(&mut self, entity_index: usize) {
        let (x, y, food, wood, entity_id) = {
            let entity = &mut self.entities[entity_index];
            let values = (
                entity.x,
                entity.y,
                entity.food.saturating_add(entity.trade_food).max(0),
                entity.wood.saturating_add(entity.trade_wood).max(0),
                entity.id,
            );
            entity.food = 0;
            entity.wood = 0;
            entity.trade_food = 0;
            entity.trade_wood = 0;
            entity.trade_target_clan = -1;
            entity.trade_returning = false;
            values
        };
        let ore = remove_entity_ore_cargo(&mut self.ore_cargo, entity_id);
        self.add_ground_loot(x, y, food, wood, ore);
    }

    fn drop_detached_entity_resources(&mut self, entity: &mut Entity) {
        let food = entity.food.saturating_add(entity.trade_food).max(0);
        let wood = entity.wood.saturating_add(entity.trade_wood).max(0);
        let ore = remove_entity_ore_cargo(&mut self.ore_cargo, entity.id);
        entity.food = 0;
        entity.wood = 0;
        entity.trade_food = 0;
        entity.trade_wood = 0;
        entity.trade_target_clan = -1;
        entity.trade_returning = false;
        self.add_ground_loot(entity.x, entity.y, food, wood, ore);
    }

    fn add_ground_loot(&mut self, x: i32, y: i32, food: i32, wood: i32, ore: u16) {
        if food <= 0 && wood <= 0 && ore == 0 {
            return;
        }
        let key = (y, x);
        match self
            .ground_loot
            .binary_search_by_key(&key, |pile| (pile.y, pile.x))
        {
            Ok(index) => {
                let pile = &mut self.ground_loot[index];
                pile.food = pile.food.saturating_add(food.max(0));
                pile.wood = pile.wood.saturating_add(wood.max(0));
                pile.ore = pile.ore.saturating_add(ore);
            }
            Err(index) => self.ground_loot.insert(
                index,
                GroundLoot {
                    x,
                    y,
                    food: food.max(0),
                    wood: wood.max(0),
                    ore,
                },
            ),
        }
    }

    fn collect_ground_loot(&mut self, entity: &mut Entity) {
        let key = (entity.y, entity.x);
        let Ok(index) = self
            .ground_loot
            .binary_search_by_key(&key, |pile| (pile.y, pile.x))
        else {
            return;
        };
        let pile = &mut self.ground_loot[index];
        entity.food = entity.food.saturating_add(pile.food.max(0));
        entity.wood = entity.wood.saturating_add(pile.wood.max(0));
        pile.food = 0;
        pile.wood = 0;
        let accepted = add_entity_ore(&mut self.ore_cargo, entity.id, pile.ore);
        pile.ore -= accepted;
        if pile.is_empty() {
            self.ground_loot.remove(index);
        }
    }

    fn update_entity(&mut self, e: &mut Entity) {
        e.ticks_since_food += self.seasonal_hunger_increment(e.id);
        e.attack_cooldown = (e.attack_cooldown - 1).max(0);
        if e.incapacitated_until > 0 {
            e.goal = Goal::Incapacitated;
            return;
        }
        self.collect_ground_loot(e);
        let starve_ticks = self.params.starve_ticks.max(1);
        let hunger = e.ticks_since_food as f32 / starve_ticks as f32;

        e.move_budget += e.speed;
        let hungry = hunger >= e.hunger_threshold;

        if hungry && self.eat_carried(e) {
            self.random_walk(e, true);
            return;
        }

        if hunger < 1.0 && e.health < e.max_health {
            e.health = (e.health + self.params.heal_rate).min(e.max_health);
        }

        // neutral
        if e.clan < 0 {
            if hungry {
                if hunger >= 1.0 {
                    e.health -= self.params.starve_damage;
                    e.goal = Goal::Starving;
                    if e.health <= 0.0 {
                        self.drop_detached_entity_resources(e);
                        e.dead = true;
                        self.deaths_starved += 1;
                        return;
                    }
                }
                self.forage(e);
            } else {
                e.goal = Goal::Wander;
                self.random_walk(e, true);
            }
            return;
        }

        let cidx = match self.clan_index(e.clan) {
            Some(i) => i,
            None => {
                e.clan = -1;
                self.random_walk(e, true);
                return;
            }
        };

        if !hungry {
            self.apply_shelter_healing(e, cidx);
        }

        if hungry {
            if self.eat_from_stockpile(e, cidx) {
                return;
            }
            let urgent = hunger >= EMERGENCY_HUNGER;
            let local_radius = if urgent {
                self.survival_food_radius(e)
            } else {
                self.params.vision_radius
            };
            let allow_steal = hunger >= EMERGENCY_STEAL;
            if let Some((px, py)) =
                self.nearest_pellet_harvestable(e.x, e.y, local_radius, e.clan, allow_steal)
            {
                e.last_food = Some((px, py));
                e.goal = Goal::SeekFood;
                self.move_toward(e, px, py, !allow_steal);
                self.consume_pellet_at(e, false);
                return;
            }
            let reserve_available =
                self.params.community_logistics && self.clans[cidx].reserve_food > 0;
            if self.clans[cidx].food > 0 || reserve_available {
                if let Some((sx, sy)) = self.clans[cidx].stockpile {
                    let d = (e.x - sx).abs().max((e.y - sy).abs());
                    if !urgent || d <= self.params.vision_radius * 2 {
                        e.goal = Goal::SeekFood;
                        self.move_toward(e, sx, sy, true);
                        self.eat_from_stockpile(e, cidx);
                        return;
                    }
                }
            }
            if hunger >= 1.0 {
                e.health -= self.params.starve_damage;
                e.goal = Goal::Starving;
                if e.health <= 0.0 {
                    self.drop_detached_entity_resources(e);
                    e.dead = true;
                    self.deaths_starved += 1;
                    return;
                }
            }
            self.forage(e); // neutral-style survival fallback (avoids enemy land)
            return;
        }

        if e.rescue_target.is_some() {
            e.goal = Goal::Rescuing;
            return;
        }
        if e.trade_target_clan >= 0 && self.handle_trade(e, cidx) {
            return;
        }

        // territory defense: hunt down a trespasser on our land (any mode)
        if let Some((tx, ty)) = self.clans[cidx].trespasser_pos {
            let d = (e.x - tx).abs().max((e.y - ty).abs());
            if d <= self.params.vision_radius {
                e.goal = Goal::Fighting;
                self.move_toward(e, tx, ty, false);
                return;
            }
        }

        match e.work_role {
            ClanMode::Attack => {
                // Only healthy members march as the war party; the rest stay and
                // work the land, so a clan at war is still a living village, not
                // an empty camp. (Any member still defends home against a
                // trespasser via the check above.)
                let warrior = e.is_leader || e.health >= e.max_health * 0.5;
                if warrior {
                    if let Some((tx, ty)) = self.clans[cidx].enemy_pos {
                        e.goal = Goal::Fighting;
                        self.move_toward(e, tx, ty, false);
                        return;
                    }
                }
                self.gather(e, cidx);
            }
            ClanMode::Recruit => {
                if e.is_leader {
                    if let Some((tx, ty)) = self.clans[cidx].neutral_pos {
                        e.goal = Goal::Recruiting;
                        self.move_toward(e, tx, ty, true);
                        return;
                    }
                }
                self.gather(e, cidx);
            }
            ClanMode::Defend => self.defend(e, cidx),
            ClanMode::Expand => {
                if self.handle_construction(e, cidx) {
                    return;
                }
                if self.handle_military_production(e, cidx) {
                    return;
                }
                if let Some((tx, ty)) = self.clans[cidx].expand_target {
                    e.goal = Goal::Claiming;
                    self.move_toward(e, tx, ty, true);
                    let d = (e.x - tx).abs().max((e.y - ty).abs());
                    if d <= 1 {
                        let interval = self.params.claim_interval.max(1);
                        if self.tick - self.clans[cidx].last_claim_tick >= interval {
                            let r = self.params.expand_claim_radius.max(1);
                            self.claim_area(cidx, tx, ty, r);
                            self.clans[cidx].last_claim_tick = self.tick;
                            self.clans[cidx].expand_target = None;
                        }
                    }
                    return;
                }
                self.gather(e, cidx);
            }
            ClanMode::Scout => {
                if e.is_leader {
                    if !self.handle_research(e, cidx) {
                        e.goal = Goal::Wander;
                        self.random_walk(e, true);
                    }
                } else {
                    self.gather(e, cidx);
                }
            }
            ClanMode::Gather => {
                if self.handle_trade(e, cidx) {
                    return;
                }
                if self.should_mine_ore(e, cidx) {
                    self.mine_ore(e, cidx);
                } else if self.should_gather_wood(e, cidx) {
                    self.gather_wood(e, cidx);
                } else {
                    self.gather(e, cidx);
                }
            }
        }
    }

    fn handle_trade(&mut self, entity: &mut Entity, clan_idx: usize) -> bool {
        if !self.params.community_trade {
            if entity.trade_target_clan >= 0 {
                self.cancel_trade(entity, clan_idx);
            }
            return false;
        }
        if entity.trade_target_clan >= 0 {
            if entity.trade_returning {
                let Some(home) = self.clans[clan_idx].stockpile else {
                    self.cancel_trade(entity, clan_idx);
                    return false;
                };
                entity.goal = Goal::Trading;
                self.move_toward(entity, home.0, home.1, false);
                if (entity.x - home.0).abs().max((entity.y - home.1).abs()) <= 1 {
                    entity.trade_target_clan = -1;
                    entity.trade_returning = false;
                }
                return true;
            }
            let Some(partner_idx) = self.clan_index(entity.trade_target_clan) else {
                self.cancel_trade(entity, clan_idx);
                return false;
            };
            let Some(target) = self.clans[partner_idx].stockpile else {
                self.cancel_trade(entity, clan_idx);
                return false;
            };
            entity.goal = Goal::Trading;
            self.move_toward(entity, target.0, target.1, false);
            let distance = (entity.x - target.0).abs().max((entity.y - target.1).abs());
            if distance > 1 {
                return true;
            }

            let donor_id = self.clans[clan_idx].id;
            let partner_id = self.clans[partner_idx].id;
            let food = entity.trade_food.max(0);
            let wood = entity.trade_wood.max(0);
            self.clans[partner_idx].food += food;
            self.clans[partner_idx].wood += wood;
            self.clans[clan_idx].stats.trade_food_sent += food as u32;
            self.clans[clan_idx].stats.trade_wood_sent += wood as u32;
            self.clans[clan_idx].stats.trade_deliveries += 1;
            self.clans[partner_idx].stats.trade_food_received += food as u32;
            self.clans[partner_idx].stats.trade_wood_received += wood as u32;
            if self.community_settlement
                && active_building_counts(&self.buildings, donor_id).markets > 0
            {
                let state = self.ensure_settlement_index(donor_id);
                self.settlements[state].stats.market_material_delivered += (food + wood) as u32;
            }
            self.diplomacy
                .record_trade(donor_id, partner_id, food as u32, wood as u32, self.tick);
            self.diplomacy
                .adjust(donor_id, partner_id, 0.02 * (food + wood) as f32);
            let repeated_aid =
                self.diplomacy
                    .lookup(donor_id, partner_id)
                    .is_some_and(|relation| {
                        relation.recent_food_delivered + relation.recent_wood_delivered
                            >= TRADE_PACT_MIN_MATERIAL
                    });
            if repeated_aid {
                self.diplomacy
                    .set_pact(donor_id, partner_id, self.tick + TRADE_PACT_TICKS);
            }
            entity.trade_returning = true;
            entity.trade_food = 0;
            entity.trade_wood = 0;
            return true;
        }

        if entity.is_leader {
            return false;
        }
        let Some(partner_id) = self.clans[clan_idx].trade_partner else {
            return false;
        };
        let Some(partner_idx) = self.clan_index(partner_id) else {
            return false;
        };
        let Some(home) = self.clans[clan_idx].stockpile else {
            return false;
        };
        if (entity.x, entity.y) != home {
            return false;
        }
        let pop = self.clan_roster_size(clan_idx).max(1);
        let partner_pop = self.clan_roster_size(partner_idx).max(1);
        let food_surplus = (self.clans[clan_idx].food - pop * TRADE_FOOD_FLOOR_PER_MEMBER).max(0);
        let wood_surplus = (self.clans[clan_idx].wood - TRADE_WOOD_FLOOR).max(0);
        let food_gap = self.clans[clan_idx].food / pop - self.clans[partner_idx].food / partner_pop;
        let wood_gap = self.clans[clan_idx].wood - self.clans[partner_idx].wood;
        let market_capacity = if self.community_settlement {
            active_building_counts(&self.buildings, self.clans[clan_idx].id).markets as i32
        } else {
            0
        };
        let food = if food_gap > 1 {
            food_surplus
                .min(TRADE_FOOD_LOAD + market_capacity)
                .min(food_gap)
        } else {
            0
        };
        let wood = if wood_gap > 1 {
            wood_surplus
                .min(TRADE_WOOD_LOAD + market_capacity)
                .min(wood_gap)
        } else {
            0
        };
        if food == 0 && wood == 0 {
            return false;
        }
        self.clans[clan_idx].food -= food;
        self.clans[clan_idx].wood -= wood;
        entity.trade_target_clan = partner_id;
        entity.trade_returning = false;
        entity.trade_food = food;
        entity.trade_wood = wood;
        entity.goal = Goal::Trading;
        true
    }

    fn cancel_trade(&mut self, entity: &mut Entity, clan_idx: usize) {
        self.clans[clan_idx].food += entity.trade_food.max(0);
        self.clans[clan_idx].wood += entity.trade_wood.max(0);
        entity.trade_target_clan = -1;
        entity.trade_returning = false;
        entity.trade_food = 0;
        entity.trade_wood = 0;
    }

    fn apply_shelter_healing(&mut self, entity: &mut Entity, clan_idx: usize) {
        if !self.community_settlement || entity.health >= entity.max_health {
            return;
        }
        let clan_id = self.clans[clan_idx].id;
        let sheltered = self.buildings.iter().any(|building| {
            building.clan_id == clan_id
                && building.kind == BuildingKind::House
                && building.is_active()
                && (entity.x - building.x)
                    .abs()
                    .max((entity.y - building.y).abs())
                    <= 6
        });
        if !sheltered {
            return;
        }
        let before = entity.health;
        entity.health = (entity.health + HOUSE_HEAL_BONUS).min(entity.max_health);
        let healed = ((entity.health - before) * 1000.0).round().max(0.0) as u64;
        if healed > 0 {
            let state = self.ensure_settlement_index(clan_id);
            self.settlements[state].stats.shelter_healing_milli += healed;
        }
    }

    fn should_mine_ore(&self, entity: &Entity, clan_idx: usize) -> bool {
        if !self.community_military
            || self.active_workshop(self.clans[clan_idx].id).is_none()
            || entity.hunger(self.params.starve_ticks.max(1)) >= EMERGENCY_HUNGER
            || entity.food > 0
            || entity.wood > 0
            || entity.trade_target_clan >= 0
        {
            return false;
        }
        if ore_cargo_for(&self.ore_cargo, entity.id).is_some() {
            return true;
        }
        if !self.military_safety_ready(clan_idx) {
            return false;
        }
        let chosen = self
            .military_index(entity.clan)
            .and_then(|index| self.militaries[index].miner_entity_id);
        if chosen != Some(entity.id) {
            return false;
        }
        let stored = self
            .military_index(entity.clan)
            .map_or(0, |index| self.militaries[index].ore_stockpile.max(0));
        stored < self.clan_roster_size(clan_idx) * MILITARY_ORE_TARGET_PER_MEMBER
            && self
                .nearest_ore_deposit(entity, self.params.home_range.max(1))
                .is_some()
    }

    fn mine_ore(&mut self, entity: &mut Entity, clan_idx: usize) {
        let carried = ore_cargo_for(&self.ore_cargo, entity.id).map_or(0, |cargo| cargo.ore);
        let safety_ready = self.military_safety_ready(clan_idx)
            && entity.hunger(self.params.starve_ticks.max(1)) < EMERGENCY_HUNGER;
        let deposit = self.nearest_ore_deposit(entity, self.params.home_range.max(1));
        if carried >= MAX_CARRIED_ORE || (carried > 0 && (deposit.is_none() || !safety_ready)) {
            if let Some((sx, sy)) = self.clans[clan_idx].stockpile {
                entity.goal = Goal::HaulingOre;
                self.move_toward(entity, sx, sy, true);
                if (entity.x, entity.y) == (sx, sy) {
                    let delivered = take_entity_ore(&mut self.ore_cargo, entity.id, u16::MAX);
                    let state = self.ensure_military_index(entity.clan);
                    self.militaries[state].deliver_ore(delivered as i32);
                }
                return;
            }
        }
        if !safety_ready {
            let state = self.ensure_military_index(entity.clan);
            self.militaries[state].record_unsafe_work_tick();
            self.gather(entity, clan_idx);
            return;
        }
        let Some(deposit_idx) = deposit else {
            self.gather(entity, clan_idx);
            return;
        };
        let (x, y) = self.ore_deposits[deposit_idx].position();
        entity.goal = Goal::MiningOre;
        self.move_toward(entity, x, y, true);
        if (entity.x, entity.y) != (x, y) {
            return;
        }
        let extracted = self.ore_deposits[deposit_idx].extract(1);
        if extracted == 0 {
            return;
        }
        let accepted = add_entity_ore(&mut self.ore_cargo, entity.id, extracted);
        let state = self.ensure_military_index(entity.clan);
        self.militaries[state].record_extraction(accepted);
    }

    fn handle_military_production(&mut self, entity: &mut Entity, clan_idx: usize) -> bool {
        if !self.community_military
            || !self.military_safety_ready(clan_idx)
            || entity.hunger(self.params.starve_ticks.max(1)) >= EMERGENCY_HUNGER
        {
            return false;
        }
        let clan_id = self.clans[clan_idx].id;
        let Some(workshop) = self.active_workshop(clan_id) else {
            return false;
        };
        let tech = self.settlement_tech(clan_id).level;
        let state_idx = self.ensure_military_index(clan_id);
        let active_recipient = self.militaries[state_idx]
            .project
            .map(|project| project.recipient_entity_id);
        if active_recipient != Some(entity.id) {
            return false;
        }

        entity.goal = Goal::ForgingEquipment;
        self.move_toward(entity, workshop.0, workshop.1, true);
        if (entity.x - workshop.0)
            .abs()
            .max((entity.y - workshop.1).abs())
            > 1
        {
            return true;
        }

        let work = if tech >= 2 { 2 } else { 1 };
        if !self.military_safety_ready(clan_idx)
            || entity.hunger(self.params.starve_ticks.max(1)) >= EMERGENCY_HUNGER
        {
            self.militaries[state_idx].record_unsafe_work_tick();
            return false;
        }
        let advance = self.militaries[state_idx].add_production_work(work);
        if let Some(produced) = advance.completed {
            let replaced = assign_equipment(&mut self.equipment, produced);
            self.militaries[state_idx].record_equipped();
            if replaced.is_some() {
                self.militaries[state_idx].record_equipment_lost(1);
            }
        }
        true
    }

    fn should_gather_wood(&self, e: &Entity, cidx: usize) -> bool {
        if !self.params.community_logistics {
            return false;
        }
        if e.wood > 0 {
            return true;
        }
        let pop = self.clan_roster_size(cidx);
        let food_safe = self.clans[cidx].food >= pop * STOCKPILE_FOOD_PER_MEMBER;
        let reserve_ready = self.clans[cidx].reserve_food >= pop * RESERVE_FOOD_PER_MEMBER;
        let road_workers = self.clans[cidx].workforce[ClanMode::Expand.index()] as i32;
        let road_target = ROAD_WOOD_COST * (road_workers.max(1) + 2);
        let settlement_target = if self.community_settlement {
            BuildingKind::Market.cost().wood + SETTLEMENT_WOOD_MARGIN
        } else {
            0
        };
        let wood_target = road_target.max(settlement_target);
        food_safe && reserve_ready && road_workers > 0 && self.clans[cidx].wood < wood_target
    }

    fn gather_wood(&mut self, e: &mut Entity, cidx: usize) {
        let limit = self.params.carry_limit.max(1);
        let stockpile = self.clans[cidx].stockpile;
        if let Some((sx, sy)) = stockpile {
            let nearby_wood = self.nearest_wood(e.x, e.y, self.params.vision_radius, e.clan);
            if e.wood >= limit || (e.wood > 0 && nearby_wood.is_none()) {
                e.goal = Goal::HaulingWood;
                self.move_toward(e, sx, sy, true);
                if e.x == sx && e.y == sy {
                    let delivered = e.wood;
                    self.clans[cidx].wood += delivered;
                    self.clans[cidx].stats.wood_delivered += delivered as u32;
                    e.wood = 0;
                }
                return;
            }
            if (e.x - sx).abs().max((e.y - sy).abs()) > self.params.home_range.max(1) {
                e.goal = Goal::GatheringWood;
                self.move_toward(e, sx, sy, true);
                return;
            }
        }
        let radius = self.params.home_range.max(self.params.vision_radius).max(1);
        if let Some((wx, wy)) = self.nearest_wood(e.x, e.y, radius, e.clan) {
            e.goal = Goal::GatheringWood;
            self.move_toward(e, wx, wy, true);
            if e.x == wx && e.y == wy {
                let i = self.grid.idx(wx, wy);
                if self.grid.wood[i] > 0 {
                    self.grid.wood[i] -= 1;
                    e.wood += 1;
                }
            }
            return;
        }
        if e.wood > 0 {
            if let Some((sx, sy)) = stockpile {
                e.goal = Goal::HaulingWood;
                self.move_toward(e, sx, sy, true);
                if e.x == sx && e.y == sy {
                    let delivered = e.wood;
                    self.clans[cidx].wood += delivered;
                    self.clans[cidx].stats.wood_delivered += delivered as u32;
                    e.wood = 0;
                }
                return;
            }
        }
        self.gather(e, cidx);
    }

    fn forage(&mut self, e: &mut Entity) {
        if self.eat_carried(e) {
            self.random_walk(e, true);
            return;
        }
        let radius = self.survival_food_radius(e);
        let allow_steal = e.hunger(self.params.starve_ticks.max(1)) >= EMERGENCY_STEAL;
        if let Some((px, py)) =
            self.nearest_pellet_harvestable(e.x, e.y, radius, e.clan, allow_steal)
        {
            e.last_food = Some((px, py));
            e.goal = Goal::SeekFood;
            self.move_toward(e, px, py, !allow_steal);
            self.consume_pellet_at(e, false);
            return;
        }
        if let Some((fx, fy)) = e.last_food {
            if e.x != fx || e.y != fy {
                e.goal = Goal::SeekFood;
                self.move_toward(e, fx, fy, true);
                return;
            }
            e.last_food = None;
        }
        if let Some((tx, ty)) = self.nearest_tree(e.x, e.y) {
            e.goal = Goal::SeekFood;
            if (e.x - tx).abs().max((e.y - ty).abs()) <= self.params.tree_radius.max(1) {
                self.random_walk(e, true);
            } else {
                self.move_toward(e, tx, ty, true);
            }
            return;
        }
        if e.goal != Goal::Starving {
            e.goal = Goal::SeekFood;
        }
        self.random_walk(e, true);
    }

    /// Central-place gathering: a working (non-hungry) clan member harvests its
    /// village's land within a home range of the stockpile, hauls full loads
    /// home, and clusters near home when there's nothing to pick — it does NOT
    /// roam the whole map. (The hungry/survival path can still range far; this
    /// is the *working* behaviour that keeps villages compact.)
    fn gather(&mut self, e: &mut Entity, cidx: usize) {
        let limit = self.params.carry_limit.max(1);
        let stockpile = self.clans[cidx].stockpile;
        if e.food >= limit {
            if let Some((sx, sy)) = stockpile {
                e.goal = Goal::Hauling;
                self.move_toward(e, sx, sy, true);
                if e.x == sx && e.y == sy {
                    self.deposit_food(cidx, e.food);
                    e.food = 0;
                }
                return;
            }
        }
        let clan_id = self.clans[cidx].id;
        // Stay within the home range: if we've drifted out, head back before working.
        if let Some((sx, sy)) = stockpile {
            let hr = self.params.home_range.max(1);
            if (e.x - sx).abs().max((e.y - sy).abs()) > hr {
                e.goal = Goal::Gathering;
                self.move_toward(e, sx, sy, true);
                return;
            }
        }
        // Harvest the nearest pellet in sight on our own/unclaimed land (workers
        // never raid foreign farms — only the starving do, via the survival path).
        if let Some((px, py)) =
            self.nearest_pellet_harvestable(e.x, e.y, self.params.vision_radius, clan_id, false)
        {
            e.goal = Goal::Gathering;
            e.last_food = Some((px, py));
            self.move_toward(e, px, py, true);
            self.consume_pellet_at(e, true);
            return;
        }
        // Nothing in sight at home. If we own no land yet, fall back to wild
        // trees to bootstrap; otherwise drift around home waiting for crops —
        // staying put is the signal that pushes the leader to expand or raid.
        if self.clans[cidx].territory == 0 {
            if let Some((tx, ty)) = self.nearest_tree(e.x, e.y) {
                e.goal = Goal::Gathering;
                if (e.x - tx).abs().max((e.y - ty).abs()) <= self.params.tree_radius.max(1) {
                    self.random_walk(e, true);
                } else {
                    self.move_toward(e, tx, ty, true);
                }
                return;
            }
        }
        e.goal = Goal::Gathering;
        if let Some((sx, sy)) = stockpile {
            // loosely cluster near the stockpile rather than wandering off
            if (e.x - sx).abs().max((e.y - sy).abs()) > 4 {
                self.move_toward(e, sx, sy, true);
                return;
            }
        }
        self.random_walk(e, true);
    }

    fn defend(&mut self, e: &mut Entity, cidx: usize) {
        if self.params.community_trade {
            if let Some(threat_id) = self.clans[cidx].trade_route_threat {
                if let Some(threat) = self.entity_index(threat_id) {
                    let (tx, ty) = (self.entities[threat].x, self.entities[threat].y);
                    e.goal = Goal::GuardingTrade;
                    self.move_toward(e, tx, ty, false);
                    return;
                }
            }
        }
        if let Some((sx, sy)) = self.clans[cidx].stockpile {
            let d = (e.x - sx).abs().max((e.y - sy).abs());
            if d > 6 {
                e.goal = Goal::Defending;
                self.move_toward(e, sx, sy, true);
            } else if !e.is_leader
                && e.health < e.max_health * 0.6
                && e.food == 0
                && e.wood == 0
                && e.trade_target_clan < 0
                && e.rescue_target.is_none()
            {
                e.goal = Goal::Hiding;
            } else {
                e.goal = Goal::Defending;
                self.random_walk(e, true);
            }
        } else {
            self.gather(e, cidx);
        }
    }

    fn resolve_recruitment(&mut self) {
        let radius = self.params.recruit_radius;
        for ci in 0..self.clans.len() {
            if self.clans[ci].disbanded || self.clans[ci].workforce[ClanMode::Recruit.index()] == 0
            {
                continue;
            }
            if !self.clan_can_add_member(ci) {
                continue;
            }
            let target_id = match self.clans[ci].recruit_target {
                Some(t) => t,
                None => continue,
            };
            let Some(ti) = self.entity_index(target_id) else {
                continue;
            };
            let (tx, ty, available) = {
                let t = &self.entities[ti];
                (t.x, t.y, t.clan < 0 && t.is_active())
            };
            if !available {
                continue;
            }
            let id = self.clans[ci].id;
            let near_member = self.entities.iter().any(|e| {
                e.clan == id
                    && e.is_active()
                    && e.work_role == ClanMode::Recruit
                    && (e.x - tx).abs().max((e.y - ty).abs()) <= radius
            });
            if near_member {
                if !self.clan_can_add_member(ci) {
                    self.clans[ci].recruit_target = None;
                    continue;
                }
                self.entities[ti].clan = id;
                self.entities[ti].last_food = None;
                self.clans[ci].members.push(target_id);
                self.clans[ci].stats.recruits += 1;
                self.clans[ci].recruit_target = None;
            }
        }
    }

    fn resolve_raiding(&mut self) {
        if self.tick < self.params.clan_grace_ticks {
            return;
        }
        let starve_ticks = self.params.starve_ticks.max(1);
        let carry_limit = self.params.carry_limit.max(1);
        let war_threshold = self.params.war_threshold;
        let stockpiles: Vec<(usize, i32, i32, i32)> = self
            .clans
            .iter()
            .enumerate()
            .filter_map(|(idx, c)| {
                if c.disbanded || c.food <= 0 {
                    return None;
                }
                c.stockpile.map(|(x, y)| (idx, c.id, x, y))
            })
            .collect();
        if stockpiles.is_empty() {
            return;
        }

        let mut raids = Vec::new();
        for (ei, e) in self.entities.iter().enumerate() {
            if !e.is_active() || e.clan < 0 {
                continue;
            }
            let hunger = e.hunger(starve_ticks);
            for &(victim_idx, victim_id, sx, sy) in &stockpiles {
                if victim_id == e.clan
                    || self.are_allied(e.clan, victim_id)
                    || self.has_trade_passage(e, victim_id)
                    || e.x != sx
                    || e.y != sy
                {
                    continue;
                }
                let aggr = self.clan_aggr(e.clan) + self.clans[victim_idx].aggression;
                if hunger >= 0.70 || aggr >= war_threshold {
                    raids.push((ei, victim_idx, e.clan));
                }
                break;
            }
        }

        for (ei, victim_idx, raider_clan) in raids {
            if self.entities[ei].dead || self.clans[victim_idx].food <= 0 {
                continue;
            }
            self.clans[victim_idx].food -= 1;
            if self.entities[ei].food < carry_limit {
                self.entities[ei].food += 1;
            } else if let Some(ri) = self.clan_index(raider_clan) {
                self.clans[ri].food += 1;
            }
            let victim_id = self.clans[victim_idx].id;
            self.diplomacy.adjust(raider_clan, victim_id, -0.08);
        }
    }

    /// Expire untreated casualties and assign one nearby Gather/Defend worker to
    /// each remaining patient. Assignments happen before ordinary entity work so
    /// rescue is a real emergency override, not a second action in the same tick.
    fn prepare_rescues(&mut self) {
        let expired: Vec<usize> = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, entity)| {
                (!entity.dead
                    && entity.incapacitated_until > 0
                    && self.tick >= entity.incapacitated_until)
                    .then_some(i)
            })
            .collect();
        for i in expired {
            let victim_clan = self.entities[i].clan;
            let attacker_clan = self.entities[i].downed_by_clan;
            if let Some(rescuer_id) = self.entities[i].carried_by {
                if let Some(rescuer) = self.entity_index(rescuer_id) {
                    self.entities[rescuer].rescue_target = None;
                }
            }
            self.drop_entity_resources(i);
            self.entities[i].dead = true;
            self.deaths_combat += 1;
            if let Some(ci) = self.clan_index(victim_clan) {
                self.clans[ci].stats.bleedouts += 1;
            }
            if let Some(ci) = self.clan_index(attacker_clan) {
                self.clans[ci].stats.kills += 1;
            }
        }
        if !self.params.community_care {
            return;
        }

        for rescuer in 0..self.entities.len() {
            let Some(patient_id) = self.entities[rescuer].rescue_target else {
                continue;
            };
            let valid = self.entity_index(patient_id).is_some_and(|patient| {
                !self.entities[patient].dead
                    && self.entities[patient].incapacitated_until > self.tick
                    && self.entities[patient].clan == self.entities[rescuer].clan
                    && !self.entities[rescuer].dead
                    && self.entities[rescuer].incapacitated_until == 0
                    && matches!(
                        self.entities[rescuer].work_role,
                        ClanMode::Gather | ClanMode::Defend
                    )
            });
            if !valid {
                self.entities[rescuer].rescue_target = None;
            }
        }

        let mut patients: Vec<usize> = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, entity)| {
                (!entity.dead && entity.incapacitated_until > self.tick).then_some(i)
            })
            .collect();
        patients.sort_by_key(|&i| self.entities[i].id);
        let mut assigned: Vec<bool> = self
            .entities
            .iter()
            .map(|entity| entity.rescue_target.is_some())
            .collect();
        for patient in patients {
            let patient_id = self.entities[patient].id;
            if let Some(rescuer_id) = self.entities[patient].carried_by {
                if let Some(rescuer) = self.entity_index(rescuer_id) {
                    if !self.entities[rescuer].dead
                        && self.entities[rescuer].incapacitated_until == 0
                        && self.entities[rescuer].clan == self.entities[patient].clan
                    {
                        self.entities[rescuer].rescue_target = Some(patient_id);
                        assigned[rescuer] = true;
                        continue;
                    }
                }
                self.entities[patient].carried_by = None;
            }
            if self
                .entities
                .iter()
                .any(|entity| entity.rescue_target == Some(patient_id))
            {
                continue;
            }

            let casualty = &self.entities[patient];
            let clan = casualty.clan;
            let mut best = None;
            for (rescuer, entity) in self.entities.iter().enumerate() {
                if assigned[rescuer]
                    || entity.dead
                    || entity.incapacitated_until > 0
                    || entity.clan != clan
                    || !matches!(entity.work_role, ClanMode::Gather | ClanMode::Defend)
                    || entity.health < entity.max_health * 0.5
                {
                    continue;
                }
                let distance = (entity.x - casualty.x)
                    .abs()
                    .max((entity.y - casualty.y).abs());
                if distance > RESCUE_RADIUS {
                    continue;
                }
                let key = (distance, entity.id, rescuer);
                if best.map_or(true, |current| key < current) {
                    best = Some(key);
                }
            }
            if let Some((_, _, rescuer)) = best {
                assigned[rescuer] = true;
                self.entities[rescuer].rescue_target = Some(patient_id);
            }
        }
    }

    /// Move assigned rescuers toward their patients, then physically carry the
    /// patient one cell behind them until both reach the clan stockpile.
    fn advance_rescues(&mut self) {
        let pairs: Vec<(usize, usize)> = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(rescuer, entity)| {
                if entity.goal != Goal::Rescuing {
                    return None;
                }
                let patient = self.entity_index(entity.rescue_target?)?;
                Some((rescuer, patient))
            })
            .collect();
        if pairs.is_empty() {
            return;
        }
        let mut entities = std::mem::take(&mut self.entities);
        self.rebuild_occupancy(&entities);
        let mut completed = Vec::new();
        for (rescuer, patient) in pairs {
            if entities[patient].dead
                || entities[rescuer].dead
                || entities[patient].incapacitated_until <= self.tick
                || entities[rescuer].incapacitated_until > 0
            {
                continue;
            }
            let rescuer_id = entities[rescuer].id;
            let carrying = entities[patient].carried_by == Some(rescuer_id);
            let patient_pos = (entities[patient].x, entities[patient].y);
            let target = if carrying {
                self.clan_index(entities[patient].clan)
                    .and_then(|ci| self.clans[ci].stockpile)
                    .unwrap_or(patient_pos)
            } else {
                let dx = (patient_pos.0 - entities[rescuer].x).signum();
                let dy = (patient_pos.1 - entities[rescuer].y).signum();
                (patient_pos.0 - dx, patient_pos.1 - dy)
            };
            entities[rescuer].goal = Goal::Rescuing;
            let old_rescuer_pos = (entities[rescuer].x, entities[rescuer].y);
            self.move_toward(&mut entities[rescuer], target.0, target.1, true);
            let distance = (entities[rescuer].x - target.0)
                .abs()
                .max((entities[rescuer].y - target.1).abs());
            let patient_distance = (entities[rescuer].x - patient_pos.0)
                .abs()
                .max((entities[rescuer].y - patient_pos.1).abs());
            if carrying {
                if (entities[rescuer].x, entities[rescuer].y) != old_rescuer_pos {
                    let old_patient = self.grid.idx(entities[patient].x, entities[patient].y);
                    let next_patient = self.grid.idx(old_rescuer_pos.0, old_rescuer_pos.1);
                    self.occupied[old_patient] = self.occupied[old_patient].saturating_sub(1);
                    self.occupied[next_patient] = self.occupied[next_patient].saturating_add(1);
                    entities[patient].x = old_rescuer_pos.0;
                    entities[patient].y = old_rescuer_pos.1;
                }
                if distance <= 1 {
                    completed.push((patient, rescuer));
                }
            } else if patient_distance <= 1 {
                entities[patient].carried_by = Some(rescuer_id);
                if self.clan_index(entities[patient].clan).is_some_and(|ci| {
                    self.clans[ci].stockpile.is_some_and(|home| {
                        (entities[rescuer].x - home.0)
                            .abs()
                            .max((entities[rescuer].y - home.1).abs())
                            <= 1
                    })
                }) {
                    completed.push((patient, rescuer));
                }
            }
        }
        self.entities = entities;

        for (patient, rescuer) in completed {
            if self.entities[patient].dead || self.entities[patient].incapacitated_until == 0 {
                continue;
            }
            let clan = self.entities[patient].clan;
            let Some(ci) = self.clan_index(clan) else {
                continue;
            };
            self.entities[patient].health =
                (self.entities[patient].max_health * RESCUE_REVIVE_HEALTH).max(1.0);
            self.entities[patient].incapacitated_until = 0;
            self.entities[patient].downed_by_clan = -1;
            self.entities[patient].downed_by_entity = None;
            self.entities[patient].carried_by = None;
            self.entities[patient].attack_cooldown = self.params.attack_cooldown.max(1);
            self.entities[patient].goal = Goal::Defending;
            self.entities[rescuer].rescue_target = None;
            self.clans[ci].stats.rescues += 1;
        }
    }

    /// Combat: a clan member attacks an adjacent target if the target is on the
    /// member's own territory (trespasser — enemy OR neutral, always), or if the
    /// two clans are mutually aggressive enough to be at war (anywhere).
    fn resolve_combat(&mut self) {
        let dmg = self.params.attack_damage;
        let cd = self.params.attack_cooldown;
        let war = self.params.war_threshold;
        let grace = self.tick < self.params.clan_grace_ticks;
        let size = self.grid.size;
        let n = self.entities.len();
        if n == 0 {
            return;
        }

        let mut occ: HashMap<i32, Vec<usize>> = HashMap::new();
        for (i, e) in self.entities.iter().enumerate() {
            if e.dead {
                continue;
            }
            occ.entry(e.y * size + e.x).or_default().push(i);
        }

        let mut attacks: Vec<(usize, usize)> = Vec::new();
        for i in 0..n {
            let (ex, ey, ec, cool) = {
                let e = &self.entities[i];
                (e.x, e.y, e.clan, e.attack_cooldown)
            };
            if ec < 0
                || cool > 0
                || self.entities[i].dead
                || self.entities[i].incapacitated_until > 0
            {
                continue; // only clan members attack
            }
            let aggr_self = self.clan_aggr(ec);
            // A clan ordered to Attack strikes enemy clans wherever it meets
            // them (it's on campaign), not only on its own soil — this is what
            // lets a land-hungry clan press into a neighbour's territory.
            let on_campaign = !grace && self.entities[i].work_role == ClanMode::Attack;
            let route_threat =
                if self.params.community_trade && self.entities[i].work_role == ClanMode::Defend {
                    self.clan_index(ec)
                        .and_then(|clan_idx| self.clans[clan_idx].trade_route_threat)
                } else {
                    None
                };
            let mut found = None;
            'search: for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let x = ex + dx;
                    let y = ey + dy;
                    if x < 0 || y < 0 || x >= size || y >= size {
                        continue;
                    }
                    if let Some(list) = occ.get(&(y * size + x)) {
                        for &j in list {
                            if j == i {
                                continue;
                            }
                            let o = &self.entities[j];
                            if o.dead || o.incapacitated_until > 0 || o.clan == ec {
                                continue;
                            }
                            if o.clan >= 0
                                && (self.are_allied(ec, o.clan)
                                    || self.has_trade_passage(&self.entities[i], o.clan)
                                    || self.has_trade_passage(o, ec))
                            {
                                continue;
                            }
                            let on_my_land = self.grid.owner[self.grid.idx(o.x, o.y)] == ec;
                            let at_war =
                                !grace && o.clan >= 0 && aggr_self + self.clan_aggr(o.clan) >= war;
                            let raid = on_campaign && o.clan >= 0;
                            let route_defense = o.clan >= 0
                                && route_threat == Some(o.id)
                                && self.clan_index(ec).is_some_and(|clan_idx| {
                                    self.clans[clan_idx]
                                        .trade_partner
                                        .is_some_and(|partner_id| {
                                            self.is_trade_route_hostile(ec, partner_id, o)
                                        })
                                });
                            if on_my_land || at_war || raid || route_defense {
                                found = Some(j);
                                break 'search;
                            }
                        }
                    }
                }
            }
            if let Some(j) = found {
                attacks.push((i, j));
            }
        }

        for (i, j) in attacks {
            if self.entities[i].attack_cooldown > 0
                || self.entities[i].dead
                || self.entities[i].incapacitated_until > 0
                || self.entities[j].dead
                || self.entities[j].incapacitated_until > 0
            {
                continue;
            }
            self.entities[i].attack_cooldown = cd;
            let attacker_clan = self.entities[i].clan;
            let victim_clan = self.entities[j].clan;
            if attacker_clan >= 0 && victim_clan >= 0 {
                self.diplomacy.adjust(attacker_clan, victim_clan, -0.04);
            }
            let attacker_loadout = self
                .community_military
                .then(|| equipment_for(&self.equipment, self.entities[i].id).copied())
                .flatten();
            let defender_loadout = self
                .community_military
                .then(|| equipment_for(&self.equipment, self.entities[j].id).copied())
                .flatten();
            let weapon_bonus = attacker_loadout.map_or(0.0, |loadout| {
                dmg * loadout.attack_bonus_milli() as f32 / 1000.0
            });
            let mut applied_damage = dmg + weapon_bonus;
            let mut downstream_factor = 1.0;
            if self.community_settlement && victim_clan >= 0 {
                let protected = self.buildings.iter().any(|building| {
                    building.clan_id == victim_clan
                        && building.kind == BuildingKind::Wall
                        && building.is_active()
                        && (self.entities[j].x - building.x)
                            .abs()
                            .max((self.entities[j].y - building.y).abs())
                            <= WALL_DEFENSE_RADIUS
                });
                if protected {
                    let prevented = applied_damage * WALL_DAMAGE_REDUCTION;
                    applied_damage -= prevented;
                    downstream_factor *= 1.0 - WALL_DAMAGE_REDUCTION;
                    let state = self.ensure_settlement_index(victim_clan);
                    self.settlements[state].stats.wall_damage_prevented_milli +=
                        (prevented * 1000.0).round() as u64;
                }
            }
            if let Some(loadout) = defender_loadout {
                let before = applied_damage;
                applied_damage = loadout.protected_damage(applied_damage);
                let prevented = before - applied_damage;
                downstream_factor *= applied_damage / before.max(f32::EPSILON);
                if prevented > 0.0 && victim_clan >= 0 {
                    let state = self.ensure_military_index(victim_clan);
                    self.militaries[state].record_damage_prevented(prevented);
                }
            }
            if weapon_bonus > 0.0 && attacker_clan >= 0 {
                let state = self.ensure_military_index(attacker_clan);
                self.militaries[state].record_bonus_damage(weapon_bonus * downstream_factor);
            }
            self.entities[j].goal = Goal::Fighting;
            self.entities[j].health -= applied_damage;
            if self.entities[j].health <= 0.0 {
                if self.params.community_care && victim_clan >= 0 {
                    self.entities[j].health = 0.0;
                    self.entities[j].incapacitated_until = self.tick + RESCUE_WINDOW_TICKS;
                    self.entities[j].downed_by_clan = attacker_clan;
                    self.entities[j].downed_by_entity = Some(self.entities[i].id);
                    self.entities[j].goal = Goal::Incapacitated;
                    if let Some(ci) = self.clan_index(victim_clan) {
                        self.clans[ci].stats.incapacitations += 1;
                    }
                } else {
                    self.drop_entity_resources(j);
                    self.entities[j].dead = true;
                    self.deaths_combat += 1;
                    if let Some(ci) = self.clan_index(attacker_clan) {
                        self.clans[ci].stats.kills += 1;
                    }
                }
            }
        }
    }

    fn detach_dead(&mut self) {
        for i in 0..self.entities.len() {
            if !self.entities[i].dead {
                continue;
            }
            let cid = self.entities[i].clan;
            let dead_id = self.entities[i].id;
            remove_entity_ore_cargo(&mut self.ore_cargo, dead_id);
            if let Some(loadout) = remove_entity_equipment(&mut self.equipment, dead_id) {
                let lost = loadout.weapon.is_some() as u32 + loadout.armor.is_some() as u32;
                if cid >= 0 {
                    let state = self.ensure_military_index(cid);
                    self.militaries[state].record_equipment_lost(lost);
                }
            }
            if let Some(state) = self.military_index(cid) {
                if self.militaries[state]
                    .project
                    .is_some_and(|project| project.recipient_entity_id == dead_id)
                {
                    self.militaries[state].project = None;
                }
            }
            if cid < 0 {
                continue;
            }
            let ci = match self.clan_index(cid) {
                Some(x) => x,
                None => continue,
            };
            self.clans[ci].stats.losses += 1;
            if self.clans[ci].leader_id == dead_id {
                let members = self.clans[ci].members.clone();
                let mut successor = None;
                for mid in members {
                    if let Some(mi) = self.entity_index(mid) {
                        if self.entities[mi].is_active() {
                            successor = Some((mi, mid));
                            break;
                        }
                    }
                }
                if let Some((mi, mid)) = successor {
                    self.entities[mi].is_leader = true;
                    self.entities[mi].max_health = self.params.leader_health;
                    self.entities[mi].health = self.entities[mi]
                        .health
                        .max(self.params.leader_health * 0.75);
                    self.entities[mi].ticks_since_food = self.entities[mi].ticks_since_food.min(0);
                    self.clans[ci].leader_id = mid;
                    self.clans[ci].members.retain(|&m| m != mid);
                } else {
                    self.disband_clan(ci);
                }
            } else {
                self.clans[ci].members.retain(|&m| m != dead_id);
            }
            self.entities[i].clan = -1;
        }
    }

    fn disband_clan(&mut self, ci: usize) {
        let id = self.clans[ci].id;
        self.clans[ci].disbanded = true;
        for o in self.grid.owner.iter_mut() {
            if *o == id {
                *o = NO_OWNER;
            }
        }
        let members = std::mem::take(&mut self.clans[ci].members);
        for mid in members {
            remove_entity_ore_cargo(&mut self.ore_cargo, mid);
            remove_entity_equipment(&mut self.equipment, mid);
            if let Some(mi) = self.entity_index(mid) {
                self.entities[mi].food += self.entities[mi].trade_food;
                self.entities[mi].wood += self.entities[mi].trade_wood;
                self.entities[mi].trade_target_clan = -1;
                self.entities[mi].trade_returning = false;
                self.entities[mi].trade_food = 0;
                self.entities[mi].trade_wood = 0;
                self.entities[mi].clan = -1;
            }
        }
        let removed_buildings: Vec<_> = self
            .buildings
            .iter()
            .filter(|building| building.clan_id == id)
            .map(|building| (building.x, building.y, building.id))
            .collect();
        for (x, y, building_id) in removed_buildings {
            self.clear_building_footprint(x, y, building_id);
        }
        self.buildings.retain(|building| building.clan_id != id);
        self.rebuild_building_footprints();
        self.settlements.retain(|state| state.clan_id != id);
        self.militaries.retain(|state| state.clan_id != id);
    }

    fn regrow_wood(&mut self) {
        if self.tick % WOOD_REGROW_INTERVAL != 0 {
            return;
        }
        let regrow_chance = self.seasonal_wood_regrow_chance();
        for i in 0..self.grid.wood.len() {
            if self.grid.terrain[i] != terrain::FOREST {
                continue;
            }
            // Always consume exactly one roll per forest tile so paired on/off
            // worlds retain common random numbers despite different depletion.
            let regrows = self.rng.chance(regrow_chance);
            if self.params.community_logistics
                && self.building_cells[i] == 0
                && self.grid.wood[i] < FOREST_WOOD_CAP
                && regrows
            {
                self.grid.wood[i] += 1;
            }
        }
    }

    fn plan_settlement_projects(&mut self) {
        if self.tick % SETTLEMENT_PLAN_INTERVAL != 0 {
            return;
        }
        let destroyed: Vec<_> = self
            .buildings
            .iter()
            .filter(|building| building.is_destroyed())
            .map(|building| (building.x, building.y, building.id))
            .collect();
        if !destroyed.is_empty() {
            for (x, y, id) in destroyed {
                self.clear_building_footprint(x, y, id);
            }
            self.rebuild_building_footprints();
        }
        if !self.community_settlement {
            return;
        }
        let clan_ids: Vec<i32> = self
            .clans
            .iter()
            .filter(|clan| !clan.disbanded)
            .map(|clan| clan.id)
            .collect();
        for clan_id in clan_ids {
            let Some(clan_idx) = self.clan_index(clan_id) else {
                continue;
            };
            let state_idx = self.ensure_settlement_index(clan_id);
            let existing = self.settlements[state_idx].build_target.and_then(|id| {
                self.buildings
                    .iter()
                    .find(|building| building.id == id && !building.is_destroyed())
            });
            if existing.is_some_and(|building| !building.is_complete()) {
                continue;
            }
            self.settlements[state_idx].build_target = None;

            let pop = self.clan_roster_size(clan_idx);
            let food_safe = self.clans[clan_idx].food >= pop * STOCKPILE_FOOD_PER_MEMBER;
            let reserve_safe = self.clans[clan_idx].reserve_food >= pop * RESERVE_FOOD_PER_MEMBER;
            let labor_ready = self.clans[clan_idx].workforce[ClanMode::Expand.index()] > 0;
            let hunger_safe = !self.entities.iter().any(|entity| {
                entity.clan == clan_id
                    && entity.is_active()
                    && entity.hunger(self.params.starve_ticks.max(1)) >= 0.85
            });
            if pop < 4 || !food_safe || !reserve_safe || !labor_ready || !hunger_safe {
                continue;
            }
            let Some(kind) = self.next_building_kind(clan_idx) else {
                continue;
            };
            let cost = kind.cost();
            if self.clans[clan_idx].wood < cost.wood + SETTLEMENT_WOOD_MARGIN {
                continue;
            }
            let Some((x, y)) = self.settlement_site(clan_idx) else {
                continue;
            };
            self.clans[clan_idx].wood -= cost.wood;
            let id = BuildingId(self.next_building_id);
            self.next_building_id = self.next_building_id.saturating_add(1);
            self.reserve_building_footprint(x, y, id);
            self.buildings.push(Building::new(id, clan_id, x, y, kind));
            self.settlements[state_idx].build_target = Some(id);
        }
    }

    /// Completed workshops provide steady civic research; an attending Scout
    /// leader can still contribute an additional physical research tick.
    fn advance_workshop_research(&mut self) {
        if !self.community_settlement || self.tick % PASSIVE_RESEARCH_INTERVAL != 0 {
            return;
        }
        let clan_ids: Vec<i32> = self
            .buildings
            .iter()
            .filter(|building| building.kind == BuildingKind::Workshop && building.is_active())
            .map(|building| building.clan_id)
            .collect();
        for clan_id in clan_ids {
            let state_idx = self.ensure_settlement_index(clan_id);
            self.settlements[state_idx].stats.research_ticks += 1;
            let gained = self.settlements[state_idx].tech.add_research(1);
            self.settlements[state_idx].stats.tech_levels_gained += gained as u32;
        }
    }

    fn plan_military_work(&mut self) {
        if !self.community_military || self.tick % MILITARY_PLAN_INTERVAL != 0 {
            return;
        }
        let clan_ids: Vec<i32> = self
            .clans
            .iter()
            .filter(|clan| !clan.disbanded)
            .map(|clan| clan.id)
            .collect();
        for clan_id in clan_ids {
            let Some(clan_idx) = self.clan_index(clan_id) else {
                continue;
            };
            let state_idx = self.ensure_military_index(clan_id);
            let valid_miner = self.militaries[state_idx]
                .miner_entity_id
                .is_some_and(|id| {
                    self.entities.iter().any(|entity| {
                        entity.id == id
                            && entity.clan == clan_id
                            && entity.is_active()
                            && !entity.is_leader
                            && entity.work_role == ClanMode::Gather
                    })
                });
            if !valid_miner {
                self.militaries[state_idx].miner_entity_id = self
                    .entities
                    .iter()
                    .filter(|entity| {
                        entity.clan == clan_id
                            && entity.is_active()
                            && !entity.is_leader
                            && entity.work_role == ClanMode::Gather
                    })
                    .map(|entity| entity.id)
                    .min();
            }

            let project_valid = self.militaries[state_idx].project.is_some_and(|project| {
                self.entities.iter().any(|entity| {
                    entity.id == project.recipient_entity_id
                        && entity.clan == clan_id
                        && entity.is_active()
                        && entity.work_role == ClanMode::Expand
                })
            });
            if self.militaries[state_idx].project.is_some() && !project_valid {
                self.militaries[state_idx].project = None;
            }
            if self.militaries[state_idx].project.is_some()
                || !self.military_safety_ready(clan_idx)
                || self.active_workshop(clan_id).is_none()
                || self.settlements.iter().any(|state| {
                    state.clan_id == clan_id
                        && state.build_target.is_some_and(|id| {
                            self.buildings.iter().any(|building| {
                                building.id == id
                                    && !building.is_complete()
                                    && !building.is_destroyed()
                            })
                        })
                })
            {
                continue;
            }
            let tech = self.settlement_tech(clan_id).level;
            let candidate = self
                .entities
                .iter()
                .filter(|entity| {
                    entity.clan == clan_id
                        && entity.is_active()
                        && entity.work_role == ClanMode::Expand
                        && self.desired_equipment(entity.id, tech).is_some()
                })
                .min_by_key(|entity| entity.id)
                .map(|entity| entity.id);
            let Some(entity_id) = candidate else {
                continue;
            };
            let kind = self
                .desired_equipment(entity_id, tech)
                .expect("candidate needs equipment");
            let spendable_wood = (self.clans[clan_idx].wood - MILITARY_WOOD_MARGIN).max(0);
            if let Some(recipe) =
                self.militaries[state_idx].begin_project(entity_id, kind, tech, spendable_wood)
            {
                self.clans[clan_idx].wood -= recipe.wood;
            }
        }
    }

    fn next_building_kind(&self, clan_idx: usize) -> Option<BuildingKind> {
        let clan = &self.clans[clan_idx];
        let counts = building_counts(&self.buildings, clan.id);
        let tech = self.settlement_tech(clan.id);
        let candidate = if counts.granaries == 0 {
            BuildingKind::Granary
        } else if counts.workshops == 0 {
            BuildingKind::Workshop
        } else if counts.houses == 0 {
            BuildingKind::House
        } else if tech.level >= 1
            && counts.walls < 2
            && (clan.enemy_pos.is_some() || clan.trade_route_threat.is_some())
        {
            BuildingKind::Wall
        } else if tech.level >= 2 && counts.markets == 0 && clan.stats.trade_deliveries > 0 {
            BuildingKind::Market
        } else if counts.houses < (self.clan_roster_size(clan_idx) / 8 + 1) as u16 {
            BuildingKind::House
        } else if counts.granaries < (self.clan_roster_size(clan_idx) / 12 + 1) as u16 {
            BuildingKind::Granary
        } else {
            return None;
        };
        tech.can_build(candidate).then_some(candidate)
    }

    fn settlement_site(&self, clan_idx: usize) -> Option<(i32, i32)> {
        let clan = &self.clans[clan_idx];
        let home = clan.stockpile?;
        let radius = self.params.home_range.clamp(2, 12);
        let mut best = None;
        for y in (home.1 - radius).max(0)..=(home.1 + radius).min(self.grid.size - 1) {
            for x in (home.0 - radius).max(0)..=(home.0 + radius).min(self.grid.size - 1) {
                if x < 1
                    || y < 1
                    || x >= self.grid.size - 1
                    || y >= self.grid.size - 1
                    || self.grid.owner[self.grid.idx(x, y)] != clan.id
                {
                    continue;
                }
                let footprint_clear =
                    building_footprint_cells(self.grid.size, x, y).all(|(fx, fy)| {
                        let index = self.grid.idx(fx, fy);
                        (fx, fy) != home
                            && self.is_passable(fx, fy)
                            && self.building_cells[index] == 0
                            && self.grid.road[index] == 0
                            && self.grid.wood[index] == 0
                            && self.grid.pellet[index] == 0
                            && !self.buildings.iter().any(|building| {
                                !building.is_destroyed()
                                    && (building.x - fx).abs() <= 1
                                    && (building.y - fy).abs() <= 1
                            })
                    });
                if !footprint_clear {
                    continue;
                }
                let index = self.grid.idx(x, y);
                let distance = (x - home.0).abs().max((y - home.1).abs());
                let key = (distance, index);
                if best.is_none_or(|(current, _, _)| key < current) {
                    best = Some((key, x, y));
                }
            }
        }
        best.map(|(_, x, y)| (x, y))
    }

    fn handle_construction(&mut self, entity: &mut Entity, clan_idx: usize) -> bool {
        if !self.community_settlement {
            return false;
        }
        let clan_id = self.clans[clan_idx].id;
        let Some(state_idx) = self.settlement_index(clan_id) else {
            return false;
        };
        let Some(target_id) = self.settlements[state_idx].build_target else {
            return false;
        };
        let Some(building_idx) = self
            .buildings
            .iter()
            .position(|building| building.id == target_id)
        else {
            self.settlements[state_idx].build_target = None;
            return false;
        };
        if self.buildings[building_idx].is_destroyed() {
            let building = &self.buildings[building_idx];
            let (x, y, id) = (building.x, building.y, building.id);
            self.clear_building_footprint(x, y, id);
            self.rebuild_building_footprints();
            self.settlements[state_idx].build_target = None;
            return false;
        }
        if self.buildings[building_idx].is_complete() {
            self.settlements[state_idx].build_target = None;
            return false;
        }
        let target = self.buildings[building_idx].position();
        entity.goal = Goal::Constructing;
        self.move_toward(entity, target.0, target.1, true);
        if (entity.x - target.0).abs().max((entity.y - target.1).abs()) > 1 {
            return true;
        }
        let work = 1 + u16::from(self.settlements[state_idx].tech.level >= 2);
        let was_complete = self.buildings[building_idx].is_complete();
        let added = self.buildings[building_idx].add_construction(work);
        self.settlements[state_idx].stats.construction_work += added as u32;
        if !was_complete && self.buildings[building_idx].is_complete() {
            self.settlements[state_idx].stats.buildings_completed += 1;
            self.settlements[state_idx].build_target = None;
        }
        true
    }

    fn handle_research(&mut self, entity: &mut Entity, clan_idx: usize) -> bool {
        if !self.community_settlement {
            return false;
        }
        let clan_id = self.clans[clan_idx].id;
        let target = self
            .buildings
            .iter()
            .filter(|building| {
                building.clan_id == clan_id
                    && building.kind == BuildingKind::Workshop
                    && building.is_active()
            })
            .min_by_key(|building| {
                (
                    (entity.x - building.x)
                        .abs()
                        .max((entity.y - building.y).abs()),
                    building.id,
                )
            })
            .map(Building::position);
        let Some(target) = target else {
            return false;
        };
        entity.goal = Goal::Researching;
        self.move_toward(entity, target.0, target.1, true);
        if self.tick % RESEARCH_INTERVAL != 0
            || (entity.x - target.0).abs().max((entity.y - target.1).abs()) > 1
        {
            return true;
        }
        let state_idx = self.ensure_settlement_index(clan_id);
        self.settlements[state_idx].stats.research_ticks += 1;
        let gained = self.settlements[state_idx].tech.add_research(1);
        self.settlements[state_idx].stats.tech_levels_gained += gained as u32;
        true
    }

    /// Expand workers turn stored wood into roads on the clan's busiest owned
    /// cells. Traffic decays after each construction pass, keeping placement
    /// responsive to recent hauling and reinforcement routes.
    fn build_roads(&mut self) {
        if self.tick % ROAD_BUILD_INTERVAL != 0 {
            return;
        }
        if !self.params.community_logistics {
            for traffic in self.grid.traffic.iter_mut() {
                *traffic /= 2;
            }
            return;
        }
        let mut clan_by_id = HashMap::new();
        for (idx, clan) in self.clans.iter().enumerate() {
            if !clan.disbanded
                && clan.wood >= ROAD_WOOD_COST
                && clan.workforce[ClanMode::Expand.index()] > 0
                && !self.buildings.iter().any(|building| {
                    building.clan_id == clan.id
                        && !building.is_destroyed()
                        && !building.is_complete()
                })
                && self.entities.iter().any(|entity| {
                    entity.clan == clan.id
                        && entity.is_active()
                        && entity.work_role == ClanMode::Expand
                })
            {
                clan_by_id.insert(clan.id, idx);
            }
        }
        let mut best: Vec<Option<(u16, usize)>> = vec![None; self.clans.len()];
        for i in 0..self.grid.owner.len() {
            if self.grid.road[i] > 0
                || self.building_cells[i] != 0
                || self.grid.traffic[i] < ROAD_MIN_TRAFFIC
            {
                continue;
            }
            let Some(&ci) = clan_by_id.get(&self.grid.owner[i]) else {
                continue;
            };
            let traffic = self.grid.traffic[i];
            if best[ci].map_or(true, |(score, old_i)| {
                traffic > score || (traffic == score && i < old_i)
            }) {
                best[ci] = Some((traffic, i));
            }
        }
        for (ci, candidate) in best.into_iter().enumerate() {
            let Some((_, i)) = candidate else {
                continue;
            };
            if self.clans[ci].wood < ROAD_WOOD_COST
                || self.grid.road[i] > 0
                || self.building_cells[i] != 0
            {
                continue;
            }
            self.clans[ci].wood -= ROAD_WOOD_COST;
            self.clans[ci].stats.roads_built += 1;
            self.grid.road[i] = 1;
            let clan_id = self.clans[ci].id;
            let x = i as i32 % self.grid.size;
            let y = i as i32 / self.grid.size;
            if let Some(builder) = self
                .entities
                .iter_mut()
                .filter(|e| e.clan == clan_id && e.is_active() && e.work_role == ClanMode::Expand)
                .min_by_key(|e| (e.x - x).abs().max((e.y - y).abs()))
            {
                builder.goal = Goal::BuildingRoad;
            }
        }
        for traffic in self.grid.traffic.iter_mut() {
            *traffic /= 2;
        }
    }

    /// Regional disasters: occasionally a blight/drought clears food and exhausts
    /// soil across a random disc. Clans must keep reserves, spread their land, and
    /// recover. Deterministic (world rng), so under common-random-numbers training
    /// every brain faces the same shocks. `disaster_level` decays toward 0 and is
    /// fed to leaders as a "turbulence" sense.
    fn maybe_disaster(&mut self) {
        self.disaster_level *= 0.997;
        let rate = self.params.disaster_rate;
        if rate <= 0.0 || self.tick % 500 != 0 {
            return;
        }
        if !self.rng.chance(rate * 0.6) {
            return;
        }
        let size = self.grid.size;
        let cx = self.rng.below(size);
        let cy = self.rng.below(size);
        let r = (size / 9).max(4);
        let r2 = r * r;
        for yy in (cy - r).max(0)..=(cy + r).min(size - 1) {
            for xx in (cx - r).max(0)..=(cx + r).min(size - 1) {
                let dx = xx - cx;
                let dy = yy - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let i = self.grid.idx(xx, yy);
                if self.grid.pellet[i] > 0 {
                    self.grid.pellet[i] = 0;
                    self.pellet_total = self.pellet_total.saturating_sub(1);
                }
                self.grid.depletion[i] = self.grid.depletion[i].saturating_add(180);
            }
        }
        self.disaster_level = 1.0;
    }

    /// Seasonal yield multiplier in [1-amp, 1+amp]: a slow sine over the world
    /// tick. Lean seasons throttle both farms and wild food, so a clan that grew
    /// fat in summer faces a winter test — adapt, conserve, or raid.
    /// Named seasonal state for UI/diagnostics. It is derived from the existing
    /// persisted tick and parameters; no additional save state is required.
    pub fn season_state(&self) -> SeasonState {
        let len = self.params.season_length;
        if len <= 0 || self.params.season_amp <= 0.0 {
            return SeasonState {
                phase: SeasonPhase::Off,
                cycle_progress: 0.0,
                phase_progress: 0.0,
                yield_factor: 1.0,
                trend: 0.0,
            };
        }

        let cycle_progress = self.tick.rem_euclid(len) as f32 / len as f32;
        let quarter = cycle_progress * 4.0;
        let phase = match quarter.floor() as i32 {
            0 => SeasonPhase::Spring,
            1 => SeasonPhase::Summer,
            2 => SeasonPhase::Autumn,
            _ => SeasonPhase::Winter,
        };
        // Use the bounded remainder rather than the absolute tick so the wave
        // stays smooth after very long runs where f32 cannot represent every
        // integer tick exactly.
        let angle = cycle_progress * std::f32::consts::TAU;
        SeasonState {
            phase,
            cycle_progress,
            phase_progress: quarter.fract(),
            yield_factor: (1.0 + self.params.season_amp * angle.sin()).max(0.0),
            trend: angle.cos(),
        }
    }

    #[inline]
    pub fn season_phase(&self) -> SeasonPhase {
        let len = self.params.season_length;
        if len <= 0 || self.params.season_amp <= 0.0 {
            return SeasonPhase::Off;
        }
        match (i64::from(self.tick.rem_euclid(len)) * 4 / i64::from(len)) as i32 {
            0 => SeasonPhase::Spring,
            1 => SeasonPhase::Summer,
            2 => SeasonPhase::Autumn,
            _ => SeasonPhase::Winter,
        }
    }

    #[inline]
    fn seasonal_soil_recovery(&self) -> u8 {
        match self.season_phase() {
            SeasonPhase::Spring => 4,
            SeasonPhase::Summer | SeasonPhase::Off => 2,
            SeasonPhase::Autumn | SeasonPhase::Winter => 1,
        }
    }

    #[inline]
    fn seasonal_wood_regrow_chance(&self) -> f32 {
        let multiplier = match self.season_phase() {
            SeasonPhase::Spring => SPRING_WOOD_REGROW_MULTIPLIER,
            SeasonPhase::Summer | SeasonPhase::Off => 1.0,
            SeasonPhase::Autumn => AUTUMN_WOOD_REGROW_MULTIPLIER,
            SeasonPhase::Winter => 0.0,
        };
        (WOOD_REGROW_CHANCE * multiplier).clamp(0.0, 1.0)
    }

    #[inline]
    fn seasonal_offroad_multiplier(&self) -> f32 {
        if self.season_phase() == SeasonPhase::Winter {
            1.0 + 0.5 * self.params.season_amp.max(0.0)
        } else {
            1.0
        }
    }

    #[inline]
    fn seasonal_birth_multiplier(&self) -> f32 {
        let amp = self.params.season_amp.clamp(0.0, 1.0);
        match self.season_phase() {
            SeasonPhase::Autumn => 1.0 - 0.5 * amp,
            SeasonPhase::Winter => 1.0 - amp,
            SeasonPhase::Off | SeasonPhase::Spring | SeasonPhase::Summer => 1.0,
        }
    }

    /// Winter raises subsistence cost without adding direct cold damage. The
    /// extra hunger tick is staggered by entity id, deterministic, and modest:
    /// about +20% at the default amplitude and +33% in the harsh benchmark.
    #[inline]
    fn seasonal_hunger_increment(&self, entity_id: u32) -> i32 {
        if self.season_phase() != SeasonPhase::Winter {
            return 1;
        }
        let extra_rate = (self.params.season_amp.clamp(0.0, 1.0) * 0.35).min(0.5);
        if extra_rate <= 0.0 {
            return 1;
        }
        let interval = (1.0 / extra_rate).round().max(2.0) as i32;
        let stagger = i64::from(self.tick) + i64::from(entity_id);
        1 + i32::from(stagger.rem_euclid(i64::from(interval)) == 0)
    }

    pub fn season_factor(&self) -> f32 {
        self.season_state().yield_factor
    }

    /// Farms: every `farm_interval` ticks, owned, fertile, passable tiles grow
    /// food. This is what makes territory the economy — claimed fertile land
    /// feeds the village, so clans have a reason to settle on it, work it, and
    /// expand/fight for more. Wild trees (sparse) only bootstrap; farmland
    /// sustains. Yield scales with fertility, so the best land is worth taking.
    fn grow_farms(&mut self) {
        if self.params.farm_interval <= 0 || self.tick % self.params.farm_interval != 0 {
            return;
        }
        let yield_rate = self.params.farm_yield * self.season_factor();
        let cells = (self.grid.size as i64 * self.grid.size as i64) as f32;
        let max_pellets = (cells * self.params.max_pellet_fraction.clamp(0.0, 1.0)) as usize;
        let deplete_on = self.params.soil_depletion_rate > 0.0;
        let soil_recovery = self.seasonal_soil_recovery();
        // Nothing to do if farms can't grow and there's no soil to recover.
        if (yield_rate <= 0.0 || self.pellet_total >= max_pellets) && !deplete_on {
            return;
        }
        let energy = self.params.pellet_energy.clamp(1, 255) as u8;
        let n = (self.grid.size * self.grid.size) as usize;
        for i in 0..n {
            if self.grid.owner[i] == NO_OWNER {
                continue;
            }
            let t = self.grid.terrain[i];
            if t == terrain::WATER || t == terrain::MOUNTAIN {
                continue;
            }
            // Soil recovers steadily on owned land (only when the feature is on).
            if deplete_on && self.grid.depletion[i] > 0 {
                self.grid.depletion[i] = self.grid.depletion[i].saturating_sub(soil_recovery);
            }
            if self.grid.pellet[i] != 0 || self.pellet_total >= max_pellets || yield_rate <= 0.0 {
                continue;
            }
            let fert = self.grid.fertility[i] as f32 / 255.0;
            if fert <= 0.0 {
                continue;
            }
            // Exhausted soil yields less, forcing the clan to work fresh land.
            let soil = if deplete_on {
                1.0 - self.grid.depletion[i] as f32 / 255.0
            } else {
                1.0
            };
            let grows = self.rng.f32() < yield_rate * fert * soil;
            if self.building_cells[i] == 0 && grows {
                self.grid.pellet[i] = energy;
                self.pellet_total += 1;
            }
        }
    }

    fn update_trees(&mut self) {
        let interval = self.params.tree_interval.max(1);
        let per_cycle =
            ((self.params.tree_per_cycle.max(0) as f32) * self.season_factor()).round() as i32;
        let r = self.params.tree_radius.max(0);
        let r2 = r * r;
        let energy = self.params.pellet_energy.clamp(1, 255) as u8;
        let cells = (self.grid.size as i64 * self.grid.size as i64) as f32;
        let max_pellets = (cells * self.params.max_pellet_fraction.clamp(0.0, 1.0)) as usize;

        let mut trees = std::mem::take(&mut self.trees);
        for t in trees.iter_mut() {
            if t.destroyed || self.tick - t.last_spawn < interval {
                continue;
            }
            t.last_spawn = self.tick;
            let mut spawned = 0;
            let mut attempts = 0;
            let budget = per_cycle * 8 + 1;
            while spawned < per_cycle && attempts < budget {
                attempts += 1;
                if self.pellet_total >= max_pellets {
                    break;
                }
                let dx = self.rng.range(-r, r + 1);
                let dy = self.rng.range(-r, r + 1);
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let x = t.x + dx;
                let y = t.y + dy;
                if !self.grid.in_bounds(x, y) || !self.is_passable(x, y) {
                    continue; // food grows on land only
                }
                let i = self.grid.idx(x, y);
                // Wild food only on unclaimed land: owned tiles are fed by farms,
                // so trees are the wilderness/bootstrap supply, not a free top-up
                // for every clan's territory.
                if self.grid.pellet[i] == 0 && self.grid.owner[i] == NO_OWNER {
                    if self.building_cells[i] == 0 {
                        self.grid.pellet[i] = energy;
                        self.pellet_total += 1;
                    }
                    spawned += 1;
                }
            }
        }
        self.trees = trees;
    }

    // --- stats ---
    pub fn population(&self) -> usize {
        self.entities.len()
    }
    pub fn leader_count(&self) -> usize {
        self.entities.iter().filter(|e| e.is_leader).count()
    }
    pub fn clan_count(&self) -> usize {
        self.clans.iter().filter(|c| !c.disbanded).count()
    }
    pub fn pellet_count(&self) -> usize {
        self.pellet_total
    }
}

fn terrain_move_cost(t: u8) -> f32 {
    match t {
        terrain::WATER => f32::INFINITY,
        terrain::MOUNTAIN => 3.0,
        terrain::HILL => 1.6,
        terrain::SAND => 1.4,
        terrain::FOREST => 1.25,
        _ => 1.0,
    }
}

fn road_cost_saved_milli(t: u8, offroad_multiplier: f32) -> u64 {
    let base = terrain_move_cost(t);
    if !base.is_finite() {
        return 0;
    }
    ((base * offroad_multiplier.max(1.0) - base * 0.5) * 1000.0)
        .round()
        .max(0.0) as u64
}

fn route_distance_sq(point: (i32, i32), start: (i32, i32), end: (i32, i32)) -> i64 {
    let vx = (end.0 - start.0) as i64;
    let vy = (end.1 - start.1) as i64;
    let wx = (point.0 - start.0) as i64;
    let wy = (point.1 - start.1) as i64;
    let length_sq = vx * vx + vy * vy;
    if length_sq == 0 {
        return wx * wx + wy * wy;
    }
    let scale = 1024i64;
    let t = ((wx * vx + wy * vy) * scale / length_sq).clamp(0, scale);
    let projected_x = start.0 as i64 * scale + vx * t;
    let projected_y = start.1 as i64 * scale + vy * t;
    let dx = point.0 as i64 * scale - projected_x;
    let dy = point.1 as i64 * scale - projected_y;
    (dx * dx + dy * dy) / (scale * scale)
}

fn detection_radius(base: i32, hidden: bool) -> i32 {
    if hidden {
        ((base.max(1) + 2) / 5).max(1)
    } else {
        base.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clan_sim_survives_long_run() {
        let mut w = World::new(120, 42);
        w.params.clan_grace_ticks = 50;
        w.params.war_threshold = 0.0;
        w.params.starve_ticks = 600;
        w.maintain_pop = 90;
        w.populate(60, 45, 6);
        assert!(w.clan_count() >= 1, "clans should form on populate");

        for _ in 0..8000 {
            w.step();
        }

        println!(
            "after 8000 ticks: pop={} clans={} starved={} killed={} food_on_map={}",
            w.population(),
            w.clan_count(),
            w.deaths_starved,
            w.deaths_combat,
            w.pellet_count()
        );
        assert!(w.population() > 0, "world should not be empty");
        assert!(w.clan_count() > 0, "at least one clan should survive");
        assert!(
            w.deaths_starved < w.population() as u64,
            "starvation should not dominate the survival baseline"
        );
    }

    #[test]
    fn sim_is_deterministic() {
        let run = || {
            let mut w = World::new(100, 7);
            w.populate(40, 30, 4);
            for _ in 0..3000 {
                w.step();
            }
            let trade_deliveries = w
                .clans
                .iter()
                .map(|clan| clan.stats.trade_deliveries)
                .sum::<u32>();
            let relationships: Vec<(i32, i32, u32, u32)> = w
                .diplomacy
                .relationships()
                .iter()
                .map(|relation| {
                    (
                        relation.clan_low,
                        relation.clan_high,
                        relation.trust.to_bits(),
                        (relation.recent_food_delivered + relation.recent_wood_delivered).to_bits(),
                    )
                })
                .collect();
            (
                w.population(),
                w.deaths_starved,
                w.deaths_combat,
                trade_deliveries,
                relationships,
            )
        };
        assert_eq!(run(), run(), "same seed must produce identical runs");
    }

    #[test]
    fn seasonal_state_names_quarters_tracks_trend_and_is_neutral_when_off() {
        let mut world = World::new(8, 11);
        world.params.season_length = 400;
        world.params.season_amp = 0.5;
        for (tick, phase, factor, trend) in [
            (0, SeasonPhase::Spring, 1.0, 1.0),
            (100, SeasonPhase::Summer, 1.5, 0.0),
            (200, SeasonPhase::Autumn, 1.0, -1.0),
            (300, SeasonPhase::Winter, 0.5, 0.0),
            (400, SeasonPhase::Spring, 1.0, 1.0),
        ] {
            world.tick = tick;
            let state = world.season_state();
            assert_eq!(state.phase, phase);
            assert!((state.yield_factor - factor).abs() < 1e-5);
            assert!((state.trend - trend).abs() < 1e-5);
        }

        world.tick = 50;
        let spring = world.season_state();
        world.tick = 150;
        let autumn_approach = world.season_state();
        assert!((spring.yield_factor - autumn_approach.yield_factor).abs() < 1e-5);
        assert!(spring.trend > 0.0 && autumn_approach.trend < 0.0);

        world.params.season_amp = 0.0;
        let off = world.season_state();
        assert_eq!(off.phase, SeasonPhase::Off);
        assert_eq!(off.yield_factor, 1.0);
        assert_eq!(off.trend, 0.0);
        assert_eq!(world.seasonal_soil_recovery(), 2);
        assert_eq!(world.seasonal_wood_regrow_chance(), WOOD_REGROW_CHANCE);
        assert_eq!(world.seasonal_offroad_multiplier(), 1.0);
        assert_eq!(world.seasonal_birth_multiplier(), 1.0);
    }

    #[test]
    fn seasonal_soil_recovery_and_birth_restraint_follow_named_phases() {
        let mut world = World::new(8, 13);
        world.params.season_length = 400;
        world.params.season_amp = 0.6;
        world.params.farm_interval = 1;
        world.params.farm_yield = 0.0;
        world.params.soil_depletion_rate = 1.0;
        let cell = 0;
        world.grid.owner[cell] = 1;
        world.grid.terrain[cell] = terrain::PLAINS;

        let mut recovery_at = |tick| {
            world.tick = tick;
            world.grid.depletion[cell] = 100;
            world.grow_farms();
            100 - world.grid.depletion[cell]
        };
        assert_eq!(recovery_at(0), 4);
        assert_eq!(recovery_at(100), 2);
        assert_eq!(recovery_at(200), 1);
        assert_eq!(recovery_at(300), 1);

        world.tick = 0;
        assert_eq!(world.seasonal_birth_multiplier(), 1.0);
        world.tick = 100;
        assert_eq!(world.seasonal_birth_multiplier(), 1.0);
        world.tick = 200;
        assert!((world.seasonal_birth_multiplier() - 0.7).abs() < 1e-6);
        world.tick = 300;
        assert!((world.seasonal_birth_multiplier() - 0.4).abs() < 1e-6);
        assert_eq!(world.seasonal_wood_regrow_chance(), 0.0);
        let winter_hunger: i32 = (300..330)
            .map(|tick| {
                world.tick = tick;
                world.seasonal_hunger_increment(7)
            })
            .sum();
        assert!(winter_hunger > 30, "winter must raise subsistence cost");
        world.tick = 100;
        assert_eq!(world.seasonal_hunger_increment(7), 1);
    }

    #[test]
    fn autumn_deliveries_prepare_reserve_before_normal_overflow() {
        let mut world = World::new(50, 19);
        world.populate(0, 0, 1);
        world.params.season_length = 400;
        world.params.season_amp = 0.5;
        let clan = 0;
        let pop = world.clan_roster_size(clan);
        world.tick = 200;
        world.clans[clan].food = pop * AUTUMN_STOCKPILE_FOOD_PER_MEMBER;
        world.clans[clan].reserve_food = 0;
        world.deposit_food(clan, 1);
        assert_eq!(
            world.clans[clan].food,
            pop * AUTUMN_STOCKPILE_FOOD_PER_MEMBER
        );
        assert_eq!(world.clans[clan].reserve_food, 1);

        world.clans[clan].reserve_food = world.reserve_capacity(clan);
        world.deposit_food(clan, 1);
        assert_eq!(
            world.clans[clan].food,
            pop * AUTUMN_STOCKPILE_FOOD_PER_MEMBER + 1
        );

        world.tick = 100;
        world.clans[clan].food = pop * AUTUMN_STOCKPILE_FOOD_PER_MEMBER;
        world.clans[clan].reserve_food = 0;
        world.deposit_food(clan, 1);
        assert_eq!(
            world.clans[clan].food,
            pop * AUTUMN_STOCKPILE_FOOD_PER_MEMBER + 1,
            "summer keeps the existing four-per-member ordinary floor"
        );
        assert_eq!(world.clans[clan].reserve_food, 0);
    }

    #[test]
    fn winter_slows_offroad_travel_while_roads_bypass_and_account_for_it() {
        let mut world = World::new(50, 21);
        world.populate(0, 0, 1);
        world.params.season_length = 400;
        world.params.season_amp = 0.6;
        world.tick = 300;
        let clan = 0;
        let leader_id = world.clans[clan].leader_id;
        let entity_index = world.entity_index(leader_id).unwrap();
        let mut leader = world.entities.remove(entity_index);
        let target = if leader.x + 1 < world.grid.size {
            (leader.x + 1, leader.y)
        } else {
            (leader.x - 1, leader.y)
        };
        let target_index = world.grid.idx(target.0, target.1);
        world.grid.terrain[target_index] = terrain::PLAINS;
        world.grid.road[target_index] = 0;
        assert!((world.move_cost_to(target.0, target.1) - 1.3).abs() < 1e-6);

        world.grid.road[target_index] = 1;
        assert_eq!(world.move_cost_to(target.0, target.1), 0.5);
        leader.move_budget = 10.0;
        world.rebuild_occupancy(std::slice::from_ref(&leader));
        assert!(world.try_step(&mut leader, target.0, target.1));
        assert_eq!(world.clans[clan].stats.road_steps, 1);
        assert_eq!(world.clans[clan].stats.road_cost_saved_milli, 800);
    }

    #[test]
    fn cut_off_territory_is_pruned() {
        // A clan owns a contiguous blob plus a disconnected island; prune frees
        // the island.
        let mut w = World::new(60, 1);
        w.populate(0, 0, 1);
        let id = w.clans[0].id;
        let (sx, sy) = w.clans[0].stockpile.unwrap();
        // stamp a disconnected island far from the stockpile
        let ix = w.grid.clamp(sx + 20);
        let iy = w.grid.clamp(sy + 20);
        let ii = w.grid.idx(ix, iy);
        w.grid.owner[ii] = id;
        w.prune_territory();
        assert_eq!(w.grid.owner[ii], NO_OWNER, "cut-off tile should be freed");
        // the stockpile tile stays owned
        let si = w.grid.idx(sx, sy);
        assert_eq!(w.grid.owner[si], id, "base stays owned");
    }

    #[test]
    fn workforce_is_deterministic_sticky_and_keeps_special_roles_on_leader() {
        let mut w = World::new(60, 17);
        w.populate(8, 8, 1);
        let ci = 0;
        let utilities = [0.95, 0.8, 0.75, 0.65, 0.55, 0.45, 0.2];
        let feasible = [true; N_MODES];
        w.tick = CLAN_THINK_INTERVAL;
        w.rebalance_workforce(ci, ClanMode::Recruit, &utilities, &feasible);

        let id = w.clans[ci].id;
        let leader_id = w.clans[ci].leader_id;
        let assigned: Vec<(u32, ClanMode)> = w
            .entities
            .iter()
            .filter(|e| e.clan == id && !e.dead)
            .map(|e| (e.id, e.work_role))
            .collect();
        assert_eq!(
            assigned.len(),
            w.clans[ci].workforce.iter().map(|&n| n as usize).sum()
        );
        assert!(w.clans[ci].workforce[ClanMode::Gather.index()] >= 1);
        assert!(w.clans[ci].workforce[ClanMode::Defend.index()] >= 1);
        for e in w.entities.iter().filter(|e| e.clan == id && !e.dead) {
            if matches!(e.work_role, ClanMode::Recruit | ClanMode::Scout) {
                assert_eq!(e.id, leader_id, "special roles must stay leader-only");
            }
        }

        w.rebalance_workforce(ci, ClanMode::Recruit, &utilities, &feasible);
        let reassigned: Vec<(u32, ClanMode)> = w
            .entities
            .iter()
            .filter(|e| e.clan == id && !e.dead)
            .map(|e| (e.id, e.work_role))
            .collect();
        assert_eq!(
            assigned, reassigned,
            "unchanged quotas should preserve jobs"
        );
    }

    #[test]
    fn reserve_only_takes_surplus_and_releases_when_working_food_is_empty() {
        let mut w = World::new(50, 23);
        assert!(Params::default().community_logistics);
        w.populate(0, 0, 1);
        let ci = 0;
        let floor = w.clan_roster_size(ci) * STOCKPILE_FOOD_PER_MEMBER;
        w.clans[ci].food = floor;
        w.deposit_food(ci, 5);
        assert_eq!(w.clans[ci].food, floor);
        assert_eq!(w.clans[ci].reserve_food, 5);
        assert_eq!(w.clans[ci].stats.food_delivered, 5);

        w.clans[ci].food = 0;
        w.clans[ci].reserve_food = 1;
        let leader_id = w.clans[ci].leader_id;
        let ei = w.entity_index(leader_id).unwrap();
        let mut leader = w.entities.remove(ei);
        (leader.x, leader.y) = w.clans[ci].stockpile.unwrap();
        assert!(w.eat_from_stockpile(&mut leader, ci));
        w.entities.insert(ei, leader);
        assert_eq!(w.clans[ci].reserve_food, 0);
        assert_eq!(w.clans[ci].stats.reserve_released, 1);
    }

    #[test]
    fn logistics_off_keeps_food_delivery_but_disables_reserves_and_wood_work() {
        let mut w = World::new(50, 29);
        w.params.community_logistics = false;
        w.populate(0, 0, 1);
        let ci = 0;
        let leader_id = w.clans[ci].leader_id;
        let ei = w.entity_index(leader_id).unwrap();
        let mut leader = w.entities.remove(ei);
        (leader.x, leader.y) = w.clans[ci].stockpile.unwrap();

        w.clans[ci].food = 0;
        w.clans[ci].reserve_food = 2;
        leader.wood = 1;
        assert!(!w.eat_from_stockpile(&mut leader, ci));
        assert_eq!(w.clans[ci].reserve_food, 2);
        assert_eq!(w.clans[ci].stats.reserve_released, 0);
        assert!(!w.should_gather_wood(&leader, ci));

        w.deposit_food(ci, 5);
        assert_eq!(w.clans[ci].food, 5);
        assert_eq!(w.clans[ci].reserve_food, 2);
        assert_eq!(w.clans[ci].stats.food_delivered, 5);
        assert_eq!(w.clans[ci].stats.reserve_deposited, 0);
    }

    #[test]
    fn wood_labor_waits_for_full_food_buffers() {
        let mut w = World::new(50, 37);
        w.populate(0, 0, 1);
        let ci = 0;
        let pop = w.clan_roster_size(ci);
        w.clans[ci].workforce[ClanMode::Expand.index()] = 1;
        w.clans[ci].wood = 0;
        w.clans[ci].food = pop * STOCKPILE_FOOD_PER_MEMBER;
        w.clans[ci].reserve_food = pop * RESERVE_FOOD_PER_MEMBER - 1;
        let entity = w.entities.remove(0);

        assert!(!w.should_gather_wood(&entity, ci));
        w.clans[ci].reserve_food = pop * RESERVE_FOOD_PER_MEMBER;
        assert!(w.should_gather_wood(&entity, ci));
    }

    #[test]
    fn road_counters_measure_effective_clan_member_steps() {
        let mut w = World::new(50, 47);
        w.populate(0, 0, 1);
        let ci = 0;
        let leader_id = w.clans[ci].leader_id;
        let ei = w.entity_index(leader_id).unwrap();
        let mut leader = w.entities.remove(ei);
        let source = (leader.x, leader.y);
        let target = if leader.x + 1 < w.grid.size {
            (leader.x + 1, leader.y)
        } else {
            (leader.x - 1, leader.y)
        };
        let source_i = w.grid.idx(source.0, source.1);
        let target_i = w.grid.idx(target.0, target.1);
        w.grid.terrain[target_i] = terrain::FOREST;
        w.grid.road[target_i] = 1;
        leader.move_budget = 10.0;
        w.rebuild_occupancy(std::slice::from_ref(&leader));

        assert_eq!(w.move_cost_to(target.0, target.1), 0.625);
        assert!(w.try_step(&mut leader, target.0, target.1));
        assert_eq!(w.clans[ci].stats.road_steps, 1);
        assert_eq!(w.clans[ci].stats.road_cost_saved_milli, 625);

        w.params.community_logistics = false;
        w.grid.terrain[source_i] = terrain::FOREST;
        w.grid.road[source_i] = 1;
        leader.move_budget = 10.0;
        assert_eq!(w.move_cost_to(source.0, source.1), 1.25);
        assert!(w.try_step(&mut leader, source.0, source.1));
        assert_eq!(w.clans[ci].stats.road_steps, 1);
        assert_eq!(w.clans[ci].stats.road_cost_saved_milli, 625);
    }

    #[test]
    fn wood_regrowth_rng_is_common_across_logistics_arms_and_fullness() {
        let mut enabled = World::new(12, 53);
        let mut disabled = World::new(12, 53);
        enabled.params.community_logistics = true;
        disabled.params.community_logistics = false;
        for &i in &[3usize, 11, 27, 89] {
            enabled.grid.terrain[i] = terrain::FOREST;
            disabled.grid.terrain[i] = terrain::FOREST;
            enabled.grid.wood[i] = FOREST_WOOD_CAP - 1;
            disabled.grid.wood[i] = FOREST_WOOD_CAP;
        }
        enabled.tick = WOOD_REGROW_INTERVAL;
        disabled.tick = WOOD_REGROW_INTERVAL;

        enabled.regrow_wood();
        disabled.regrow_wood();

        assert_eq!(enabled.rng.f32(), disabled.rng.f32());
        assert!(disabled
            .grid
            .wood
            .iter()
            .all(|&wood| wood == 0 || wood == FOREST_WOOD_CAP));
    }

    #[test]
    fn forest_wood_resets_and_busy_owned_tiles_become_roads() {
        let mut w = World::new(80, 31);
        w.populate(0, 0, 1);
        let forest: Vec<usize> = w
            .grid
            .terrain
            .iter()
            .enumerate()
            .filter_map(|(i, &t)| (t == terrain::FOREST).then_some(i))
            .collect();
        assert!(!forest.is_empty(), "seeded terrain should contain forest");
        assert!(forest.iter().all(|&i| w.grid.wood[i] == FOREST_WOOD_CAP));

        let ci = 0;
        let id = w.clans[ci].id;
        let road_i = w
            .grid
            .owner
            .iter()
            .position(|&owner| owner == id)
            .expect("founding territory");
        w.grid.traffic[road_i] = ROAD_MIN_TRAFFIC + 5;
        w.clans[ci].wood = ROAD_WOOD_COST;
        w.clans[ci].workforce[ClanMode::Expand.index()] = 1;
        let builder = w
            .entities
            .iter_mut()
            .find(|entity| entity.clan == id && entity.is_active())
            .expect("active road builder");
        builder.work_role = ClanMode::Expand;
        w.tick = ROAD_BUILD_INTERVAL;
        w.build_roads();
        assert_eq!(w.grid.road[road_i], 1);
        assert_eq!(w.clans[ci].wood, 0);
        assert_eq!(w.clans[ci].stats.roads_built, 1);

        w.params.community_logistics = false;
        let second_i = w
            .grid
            .owner
            .iter()
            .enumerate()
            .find_map(|(i, &owner)| (owner == id && i != road_i).then_some(i))
            .expect("second founding tile");
        w.grid.traffic[second_i] = ROAD_MIN_TRAFFIC + 5;
        w.clans[ci].wood = ROAD_WOOD_COST;
        w.build_roads();
        assert_eq!(w.grid.road[second_i], 0);
        assert_eq!(w.clans[ci].wood, ROAD_WOOD_COST);
        assert_eq!(w.logistics_signals(ci), (0.0, 0.0));

        w.clear();
        assert!(w.grid.road.iter().all(|&v| v == 0));
        assert!(w.grid.wood.iter().all(|&v| v == 0));
        assert!(w.grid.traffic.iter().all(|&v| v == 0));
    }

    #[test]
    fn settlement_project_requires_food_reserve_wood_and_physical_expand_work() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let pop = world.clan_roster_size(0);
        world.clans[0].food = pop * STOCKPILE_FOOD_PER_MEMBER;
        world.clans[0].reserve_food = pop * RESERVE_FOOD_PER_MEMBER - 1;
        world.clans[0].wood = BuildingKind::Granary.cost().wood + SETTLEMENT_WOOD_MARGIN;
        world.tick = SETTLEMENT_PLAN_INTERVAL;
        world.plan_settlement_projects();
        assert!(
            world.buildings.is_empty(),
            "unsafe settlement work must not start"
        );

        world.clans[0].reserve_food = pop * RESERVE_FOOD_PER_MEMBER;
        world.plan_settlement_projects();
        assert_eq!(world.buildings.len(), 1);
        assert_eq!(world.buildings[0].kind, BuildingKind::Granary);
        let building_id = world.buildings[0].id;
        let footprint: Vec<_> =
            building_footprint_cells(world.grid.size, world.buildings[0].x, world.buildings[0].y)
                .map(|(x, y)| world.grid.idx(x, y))
                .collect();
        assert_eq!(footprint.len(), 9);
        assert!(footprint
            .iter()
            .all(|&cell| world.building_cells[cell] == building_id.0));
        assert_eq!(
            world.clans[0].wood, SETTLEMENT_WOOD_MARGIN,
            "wood is reserved exactly once when the site opens"
        );
        let builder = world
            .entities
            .iter()
            .position(|entity| entity.clan == clan_id && !entity.is_leader)
            .unwrap();
        let target = world.buildings[0].position();
        let mut entities = std::mem::take(&mut world.entities);
        world.rebuild_occupancy(&entities);
        let mut worker = entities.remove(builder);
        world.entities = entities;
        worker.work_role = ClanMode::Expand;
        worker.x = target.0;
        worker.y = (target.1 + 1).min(world.grid.size - 1);
        for _ in 0..BuildingKind::Granary.cost().work {
            assert!(world.handle_construction(&mut worker, 0));
        }
        assert!(world.buildings[0].is_complete());
        assert_eq!(world.settlements[0].stats.buildings_completed, 1);
        world.disband_clan(0);
        assert!(footprint
            .iter()
            .all(|&cell| world.building_cells[cell] == 0));
    }

    #[test]
    fn workshop_passively_researches_and_physical_scouts_accelerate_it() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let home = world.clans[0].stockpile.unwrap();
        let mut workshop = Building::new(
            BuildingId(world.next_building_id),
            clan_id,
            home.0 + 1,
            home.1,
            BuildingKind::Workshop,
        );
        world.next_building_id += 1;
        workshop.add_construction(BuildingKind::Workshop.cost().work);
        world.reserve_building_footprint(workshop.x, workshop.y, workshop.id);
        world.buildings.push(workshop);
        let leader = world.entity_index(world.clans[0].leader_id).unwrap();
        let mut entities = std::mem::take(&mut world.entities);
        world.rebuild_occupancy(&entities);
        let mut researcher = entities.remove(leader);
        world.entities = entities;
        researcher.x = home.0;
        researcher.y = home.1;
        researcher.work_role = ClanMode::Scout;

        world.tick = PASSIVE_RESEARCH_INTERVAL - 1;
        world.advance_workshop_research();
        assert!(world.settlements.is_empty());
        world.tick = PASSIVE_RESEARCH_INTERVAL;
        world.advance_workshop_research();
        assert_eq!(world.settlements[0].stats.research_ticks, 1);

        for _ in 0..39 {
            world.tick += RESEARCH_INTERVAL;
            assert!(world.handle_research(&mut researcher, 0));
        }

        assert_eq!(world.settlement_tech(clan_id).level, 1);
        assert_eq!(world.settlements[0].stats.research_ticks, 40);
    }

    #[test]
    fn default_healing_cannot_erase_one_unarmed_hit_per_cooldown() {
        assert_eq!(D_HEAL_RATE, 0.008);
        assert!(D_HEAL_RATE * (D_ATTACK_COOLDOWN as f32) < D_ATTACK_DAMAGE);
    }

    #[test]
    fn building_footprint_blocks_food_and_road_production() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let (x, y) = world.settlement_site(0).unwrap();
        let mut building = Building::new(
            BuildingId(world.next_building_id),
            clan_id,
            x,
            y,
            BuildingKind::House,
        );
        world.next_building_id += 1;
        building.add_construction(building.kind.cost().work);
        world.reserve_building_footprint(x, y, building.id);
        world.buildings.push(building);
        let footprint: Vec<_> = building_footprint_cells(world.grid.size, x, y)
            .map(|(cell_x, cell_y)| world.grid.idx(cell_x, cell_y))
            .collect();
        for &cell in &footprint {
            world.grid.owner[cell] = clan_id;
            world.grid.terrain[cell] = terrain::PLAINS;
            world.grid.fertility[cell] = u8::MAX;
            world.grid.pellet[cell] = 0;
            world.grid.road[cell] = 0;
            world.grid.traffic[cell] = 0;
        }

        world.params.farm_interval = 1;
        world.params.farm_yield = 1.0;
        world.params.max_pellet_fraction = 1.0;
        world.params.season_length = 0;
        world.tick = 1;
        world.grow_farms();
        assert!(footprint.iter().all(|&cell| world.grid.pellet[cell] == 0));

        let blocked = footprint[0];
        world.grid.traffic[blocked] = ROAD_MIN_TRAFFIC + 10;
        world.clans[0].wood = ROAD_WOOD_COST;
        world.tick = ROAD_BUILD_INTERVAL;
        world.build_roads();
        assert_eq!(world.grid.road[blocked], 0);
        assert_eq!(world.clans[0].wood, ROAD_WOOD_COST);

        let tree_x = blocked as i32 % world.grid.size;
        let tree_y = blocked as i32 / world.grid.size;
        world.grid.owner[blocked] = NO_OWNER;
        world.params.tree_interval = 1;
        world.params.tree_per_cycle = 1;
        world.params.tree_radius = 0;
        world.trees = vec![Tree {
            x: tree_x,
            y: tree_y,
            last_spawn: 0,
            destroyed: false,
        }];
        world.tick = 1;
        world.update_trees();
        assert_eq!(world.grid.pellet[blocked], 0);

        world.grid.terrain[blocked] = terrain::FOREST;
        world.grid.wood[blocked] = 0;
        for cycle in 1..=256 {
            world.tick = cycle * WOOD_REGROW_INTERVAL;
            world.regrow_wood();
        }
        assert_eq!(world.grid.wood[blocked], 0);
    }

    #[test]
    fn settlement_ablation_zeros_signals_and_disables_additive_capacity() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let home = world.clans[0].stockpile.unwrap();
        for (offset, kind) in [(1, BuildingKind::House), (2, BuildingKind::Granary)] {
            let mut building = Building::new(
                BuildingId(world.next_building_id),
                clan_id,
                home.0 + offset,
                home.1,
                kind,
            );
            world.next_building_id += 1;
            building.add_construction(kind.cost().work);
            world.reserve_building_footprint(building.x, building.y, building.id);
            world.buildings.push(building);
        }
        let base_cap = {
            world.community_settlement = false;
            world.clan_member_cap(0)
        };
        world.community_settlement = true;
        assert_eq!(world.clan_member_cap(0), base_cap + HOUSE_MEMBER_CAPACITY);
        assert_eq!(
            world.reserve_capacity(0),
            world.clan_roster_size(0) * RESERVE_FOOD_PER_MEMBER + GRANARY_RESERVE_CAPACITY
        );
        assert!(world.settlement_signals(0).0 > 0.0);
        let state_idx = world.ensure_settlement_index(clan_id);
        world.settlements[state_idx].tech.level = 2;
        let base_reserve = world.clan_roster_size(0) * RESERVE_FOOD_PER_MEMBER;
        world.clans[0].reserve_food = base_reserve + GRANARY_RESERVE_CAPACITY;
        world.community_settlement = false;
        world.enforce_settlement_ablation_limits();
        assert_eq!(world.settlement_signals(0), (0.0, 0.0));
        assert_eq!(world.clan_member_cap(0), base_cap);
        assert_eq!(world.clans[0].reserve_food, base_reserve);
        let quality = crate::quality::score_clan(&world, clan_id);
        assert_eq!(quality.infrastructure, 0.0);
        assert_eq!(quality.technology, 0.0);
    }

    #[test]
    fn military_pipeline_requires_physical_mining_delivery_and_forging() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let home = world.clans[0].stockpile.unwrap();
        let mut workshop = Building::new(
            BuildingId(world.next_building_id),
            clan_id,
            home.0 + 1,
            home.1,
            BuildingKind::Workshop,
        );
        world.next_building_id += 1;
        workshop.add_construction(workshop.kind.cost().work);
        world.reserve_building_footprint(workshop.x, workshop.y, workshop.id);
        world.buildings.push(workshop);
        let pop = world.clan_roster_size(0);
        world.clans[0].food = pop * STOCKPILE_FOOD_PER_MEMBER;
        world.clans[0].reserve_food = pop * RESERVE_FOOD_PER_MEMBER;
        world.clans[0].wood = MILITARY_WOOD_MARGIN + EquipmentKind::Spear.recipe().wood;
        let miner_id = {
            let miner = world
                .entities
                .iter_mut()
                .find(|entity| entity.clan == clan_id && !entity.is_leader)
                .unwrap();
            miner.work_role = ClanMode::Gather;
            miner.id
        };
        world.plan_military_work();
        assert_eq!(world.militaries[0].miner_entity_id, Some(miner_id));
        let deposit_idx = world
            .nearest_ore_deposit(world.entity_by_id(miner_id).unwrap(), world.grid.size)
            .unwrap();
        let deposit = world.ore_deposits[deposit_idx].position();

        let miner_idx = world.entity_index(miner_id).unwrap();
        let mut miner = world.entities.remove(miner_idx);
        world.rebuild_occupancy(&world.entities.clone());
        let before = world.ore_deposits[deposit_idx].remaining;
        miner.x = world.grid.clamp(deposit.0 + 3);
        miner.y = deposit.1;
        miner.move_budget = 0.0;
        world.mine_ore(&mut miner, 0);
        assert_eq!(world.ore_deposits[deposit_idx].remaining, before);
        miner.x = deposit.0;
        miner.y = deposit.1;
        for _ in 0..MAX_CARRIED_ORE {
            world.mine_ore(&mut miner, 0);
        }
        assert_eq!(
            ore_cargo_for(&world.ore_cargo, miner_id).unwrap().ore,
            MAX_CARRIED_ORE
        );
        (miner.x, miner.y) = home;
        world.mine_ore(&mut miner, 0);
        assert!(ore_cargo_for(&world.ore_cargo, miner_id).is_none());
        assert_eq!(world.militaries[0].ore_stockpile, MAX_CARRIED_ORE as i32);
        world.entities.push(miner);
        world.entities.sort_by_key(|entity| entity.id);

        let smith_id = world
            .entities
            .iter_mut()
            .find(|entity| entity.clan == clan_id && entity.id != miner_id)
            .map(|entity| {
                entity.work_role = ClanMode::Expand;
                entity.id
            })
            .unwrap();
        world.plan_military_work();
        assert_eq!(
            world.militaries[0].project.unwrap().recipient_entity_id,
            smith_id
        );
        let smith_idx = world.entity_index(smith_id).unwrap();
        let mut smith = world.entities.remove(smith_idx);
        world.rebuild_occupancy(&world.entities.clone());
        let forge = world.active_workshop(clan_id).unwrap();
        smith.x = if forge.0 < world.grid.size / 2 {
            world.grid.size - 1
        } else {
            0
        };
        smith.y = if forge.1 < world.grid.size / 2 {
            world.grid.size - 1
        } else {
            0
        };
        smith.move_budget = 0.0;
        assert!(world.handle_military_production(&mut smith, 0));
        assert_eq!(world.militaries[0].stats.production_work, 0);
        smith.x = home.0;
        smith.y = home.1;
        for _ in 0..EquipmentKind::Spear.recipe().work {
            assert!(world.handle_military_production(&mut smith, 0));
        }
        assert_eq!(
            equipment_for(&world.equipment, smith_id).unwrap().weapon,
            Some(EquipmentKind::Spear)
        );
        assert_eq!(world.militaries[0].stats.equipment_completed, 1);
    }

    #[test]
    fn military_ablation_zeros_signals_scoring_and_combat_effects() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let entity_id = world
            .entities
            .iter()
            .find(|entity| entity.clan == clan_id)
            .unwrap()
            .id;
        assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: entity_id,
                kind: EquipmentKind::Spear,
            },
        );
        let state = world.ensure_military_index(clan_id);
        world.militaries[state].ore_stockpile = 12;
        assert!(world.military_signals(0).0 > 0.0);
        world.community_military = false;
        assert_eq!(world.military_signals(0), (0.0, 0.0));
        assert_eq!(crate::quality::score_clan(&world, clan_id).military, 0.0);
    }

    #[test]
    fn military_equipment_applies_exact_combat_value_only_when_enabled() {
        let (mut world, attacker, victim, attacker_clan, victim_clan) = trade_test_world();
        world.params.community_trade = false;
        world.params.community_care = false;
        world.params.clan_grace_ticks = 0;
        world.params.war_threshold = 0.0;
        world.params.attack_damage = 4.0;
        world.entities[attacker].x = 10;
        world.entities[attacker].y = 10;
        world.entities[attacker].work_role = ClanMode::Attack;
        world.entities[attacker].attack_cooldown = 0;
        world.entities[victim].x = 11;
        world.entities[victim].y = 10;
        assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: world.entities[attacker].id,
                kind: EquipmentKind::Spear,
            },
        );
        assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: world.entities[victim].id,
                kind: EquipmentKind::Armor,
            },
        );
        let victim_health = world.entities[victim].max_health;
        world.entities[victim].health = victim_health;
        world.resolve_combat();
        assert!((victim_health - world.entities[victim].health - 3.75).abs() < 1e-5);
        assert!(world.militaries[attacker_clan].stats.bonus_damage_milli > 0);
        assert!(world.militaries[victim_clan].stats.damage_prevented_milli > 0);

        world.community_military = false;
        world.entities[attacker].attack_cooldown = 0;
        world.entities[victim].health = victim_health;
        world.resolve_combat();
        assert!((victim_health - world.entities[victim].health - 4.0).abs() < 1e-5);
    }

    #[test]
    fn military_food_gate_blocks_assignment_and_resource_spending() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let home = world.clans[0].stockpile.unwrap();
        let mut workshop = Building::new(
            BuildingId(world.next_building_id),
            clan_id,
            home.0 + 1,
            home.1,
            BuildingKind::Workshop,
        );
        world.next_building_id += 1;
        workshop.add_construction(workshop.kind.cost().work);
        world.reserve_building_footprint(workshop.x, workshop.y, workshop.id);
        world.buildings.push(workshop);
        world.clans[0].food = 0;
        world.clans[0].reserve_food = 0;
        world.clans[0].wood = 100;
        let state = world.ensure_military_index(clan_id);
        world.militaries[state].ore_stockpile = 100;
        let before_wood = world.clans[0].wood;
        world.plan_military_work();
        assert!(world.militaries[state].project.is_none());
        assert_eq!(world.clans[0].wood, before_wood);
        let worker_id = world
            .entities
            .iter()
            .find(|entity| entity.clan == clan_id && !entity.is_leader)
            .unwrap()
            .id;
        let worker_idx = world.entity_index(worker_id).unwrap();
        let mut worker = world.entities.remove(worker_idx);
        world.rebuild_occupancy(&world.entities.clone());
        let deposit_idx = world
            .nearest_ore_deposit(&worker, world.grid.size)
            .expect("bootstrap ore");
        (worker.x, worker.y) = world.ore_deposits[deposit_idx].position();
        let remaining = world.ore_deposits[deposit_idx].remaining;
        world.mine_ore(&mut worker, 0);
        assert_eq!(world.ore_deposits[deposit_idx].remaining, remaining);
        assert_eq!(world.militaries[state].stats.unsafe_work_ticks, 1);

        add_entity_ore(&mut world.ore_cargo, worker_id, 3);
        worker.goal = Goal::Wander;
        world.mine_ore(&mut worker, 0);
        assert_eq!(worker.goal, Goal::HaulingOre);
        assert_eq!(world.ore_deposits[deposit_idx].remaining, remaining);
        (worker.x, worker.y) = home;
        world.mine_ore(&mut worker, 0);
        assert!(ore_cargo_for(&world.ore_cargo, worker_id).is_none());
    }

    #[test]
    fn military_cleanup_removes_dead_and_disbanded_ownership_references() {
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let member_id = world.clans[0].members[0];
        add_entity_ore(&mut world.ore_cargo, member_id, 3);
        assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: member_id,
                kind: EquipmentKind::Spear,
            },
        );
        let state = world.ensure_military_index(clan_id);
        world.militaries[state].miner_entity_id = Some(member_id);
        world.militaries[state].project = Some(crate::military::EquipmentProject::new(
            member_id,
            EquipmentKind::Spear,
        ));
        let member = world.entity_index(member_id).unwrap();
        world.entities[member].dead = true;
        world.detach_dead();
        assert!(ore_cargo_for(&world.ore_cargo, member_id).is_none());
        assert!(equipment_for(&world.equipment, member_id).is_none());
        assert!(world.militaries[state].project.is_none());

        let survivor_id = world.clans[0].members[0];
        add_entity_ore(&mut world.ore_cargo, survivor_id, 2);
        assign_equipment(
            &mut world.equipment,
            crate::military::ProducedEquipment {
                recipient_entity_id: survivor_id,
                kind: EquipmentKind::Armor,
            },
        );
        world.disband_clan(0);
        assert!(ore_cargo_for(&world.ore_cargo, survivor_id).is_none());
        assert!(equipment_for(&world.equipment, survivor_id).is_none());
        assert!(world
            .militaries
            .iter()
            .all(|state| state.clan_id != clan_id));
    }

    #[test]
    fn ground_loot_is_physical_persistent_and_stealable() {
        let mut world = World::new(12, 77);
        let mut victim = world.make_entity(4, 5, false);
        victim.food = 3;
        victim.wood = 2;
        victim.trade_food = 4;
        victim.trade_wood = 1;
        assert_eq!(add_entity_ore(&mut world.ore_cargo, victim.id, 6), 6);
        world.drop_detached_entity_resources(&mut victim);
        assert_eq!(victim.food, 0);
        assert_eq!(victim.wood, 0);
        assert_eq!(world.ground_loot.len(), 1);
        assert_eq!(world.ground_loot[0].food, 7);
        assert_eq!(world.ground_loot[0].wood, 3);
        assert_eq!(world.ground_loot[0].ore, 6);

        let mut thief = world.make_entity(4, 5, false);
        world.collect_ground_loot(&mut thief);
        assert_eq!((thief.food, thief.wood), (7, 3));
        assert_eq!(ore_cargo_for(&world.ore_cargo, thief.id).unwrap().ore, 6);
        assert!(world.ground_loot.is_empty());
    }

    #[test]
    fn cells_accept_three_units_and_reject_a_fourth() {
        let mut world = World::new(8, 91);
        for _ in 0..4 {
            world.spawn_entity(3, 3, false);
        }
        assert_eq!(
            world
                .entities
                .iter()
                .filter(|entity| (entity.x, entity.y) == (3, 3))
                .count(),
            3
        );
        let entities = world.entities.clone();
        world.rebuild_occupancy(&entities);
        assert_eq!(world.occupied[world.grid.idx(3, 3)], MAX_ENTITIES_PER_CELL);
    }

    #[test]
    fn saturated_spawn_fallback_does_not_create_a_fourth_unit() {
        let mut world = World::new(2, 92);
        for y in 0..world.grid.size {
            for x in 0..world.grid.size {
                for _ in 0..MAX_ENTITIES_PER_CELL {
                    world.spawn_entity(x, y, false);
                }
            }
        }
        let population = world.population();
        assert!(world.random_land_cell().is_none());
        assert!(!world.spawn_clan());
        assert_eq!(world.population(), population);
    }

    #[test]
    fn defend_workers_hide_statically_and_cut_detection_to_twenty_percent() {
        assert_eq!(detection_radius(15, true), 3);
        assert_eq!(detection_radius(15, false), 15);
        assert_eq!(detection_radius(12, true), 2);
        assert_eq!(detection_radius(17, true), 3);
        assert_eq!(detection_radius(1, true), 1);
        let mut world = settlement_test_world();
        let clan_id = world.clans[0].id;
        let worker = world
            .entities
            .iter()
            .position(|entity| entity.clan == clan_id && !entity.is_leader)
            .unwrap();
        let home = world.clans[0].stockpile.unwrap();
        let mut entity = world.entities.remove(worker);
        entity.x = home.0;
        entity.y = home.1;
        entity.food = 0;
        entity.wood = 0;
        entity.health = entity.max_health * 0.5;
        entity.work_role = ClanMode::Defend;
        world.clans[0].trade_partner = Some(clan_id + 1);
        world.defend(&mut entity, 0);
        assert_eq!((entity.x, entity.y), home);
        assert_eq!(entity.goal, Goal::Hiding);
    }

    fn settlement_test_world() -> World {
        let mut world = World::new(40, 0x5E77_1E);
        world.populate(0, 0, 1);
        let clan_id = world.clans[0].id;
        let home = world.clans[0].stockpile.unwrap();
        let direction = if home.0 + 3 < world.grid.size { 1 } else { -1 };
        for dx in 1..=3 {
            let x = home.0 + direction * dx;
            for dy in -1..=1 {
                let index = world.grid.idx(x, home.1 + dy);
                world.grid.owner[index] = clan_id;
                world.grid.terrain[index] = terrain::PLAINS;
                world.grid.pellet[index] = 0;
                world.grid.wood[index] = 0;
                world.grid.road[index] = 0;
            }
        }
        world.clans[0].workforce[ClanMode::Expand.index()] = 1;
        let worker = world
            .entities
            .iter_mut()
            .find(|entity| entity.clan == clan_id && !entity.is_leader)
            .unwrap();
        worker.work_role = ClanMode::Expand;
        world
    }

    #[test]
    fn trade_loads_only_true_surplus_and_ablation_moves_nothing() {
        let (mut world, donor, _, donor_idx, _) = trade_test_world();
        let pop = world.clan_roster_size(donor_idx);
        let food_floor = pop * TRADE_FOOD_FLOOR_PER_MEMBER;
        world.clans[donor_idx].food = food_floor + 5;
        world.clans[donor_idx].wood = TRADE_WOOD_FLOOR + 2;
        let mut entities = std::mem::take(&mut world.entities);
        world.rebuild_occupancy(&entities);
        let mut courier = entities.remove(donor);
        world.entities = entities;
        assert!(world.handle_trade(&mut courier, donor_idx));
        assert_eq!(courier.trade_food, TRADE_FOOD_LOAD);
        assert_eq!(courier.trade_wood, TRADE_WOOD_LOAD);
        assert!(world.clans[donor_idx].food >= food_floor);
        assert!(world.clans[donor_idx].wood >= TRADE_WOOD_FLOOR);

        world.cancel_trade(&mut courier, donor_idx);
        world.params.community_trade = false;
        let food = world.clans[donor_idx].food;
        let wood = world.clans[donor_idx].wood;
        assert!(!world.handle_trade(&mut courier, donor_idx));
        assert_eq!(world.clans[donor_idx].food, food);
        assert_eq!(world.clans[donor_idx].wood, wood);
        assert_eq!(courier.trade_target_clan, -1);
    }

    #[test]
    fn trade_partner_ties_are_resolved_by_stable_clan_id() {
        let mut world = World::new(40, 0x71E);
        world.populate(0, 0, 3);
        world.tick = TRADE_REFRESH_INTERVAL;
        world.clans[0].stockpile = Some((10, 10));
        world.clans[1].stockpile = Some((8, 10));
        world.clans[2].stockpile = Some((12, 10));
        world.refresh_diplomacy();
        assert_eq!(world.clans[0].trade_partner, Some(world.clans[1].id));
    }

    #[test]
    fn trade_cargo_changes_ownership_only_after_physical_delivery() {
        let (mut world, donor, _, donor_idx, recipient_idx) = trade_test_world();
        let recipient_food = world.clans[recipient_idx].food;
        let recipient_wood = world.clans[recipient_idx].wood;
        let mut entities = std::mem::take(&mut world.entities);
        world.rebuild_occupancy(&entities);
        let mut courier = entities.remove(donor);
        world.entities = entities;
        assert!(world.handle_trade(&mut courier, donor_idx));
        assert_eq!(world.clans[recipient_idx].food, recipient_food);
        assert_eq!(world.clans[recipient_idx].wood, recipient_wood);

        for _ in 0..20 {
            courier.move_budget += 1.0;
            world.handle_trade(&mut courier, donor_idx);
            if courier.trade_target_clan < 0 {
                break;
            }
        }
        assert_eq!(courier.trade_target_clan, -1);
        assert_eq!(
            world.clans[recipient_idx].food,
            recipient_food + TRADE_FOOD_LOAD
        );
        assert_eq!(
            world.clans[recipient_idx].wood,
            recipient_wood + TRADE_WOOD_LOAD
        );
        assert_eq!(
            world.clans[donor_idx].stats.trade_food_sent,
            TRADE_FOOD_LOAD as u32
        );
        assert_eq!(
            world.clans[recipient_idx].stats.trade_food_received,
            TRADE_FOOD_LOAD as u32
        );
        assert_eq!(world.clans[donor_idx].stats.trade_deliveries, 1);
        let relation = world
            .diplomacy
            .lookup(world.clans[donor_idx].id, world.clans[recipient_idx].id)
            .expect("delivery creates relationship memory");
        assert!(relation.trust > 0.0);
        assert_eq!(relation.last_trade_tick, Some(world.tick));
    }

    #[test]
    fn diplomacy_requires_repeated_physical_aid_before_a_pact() {
        let (mut world, donor, _, donor_idx, recipient_idx) = trade_test_world();
        let donor_id = world.clans[donor_idx].id;
        let recipient_id = world.clans[recipient_idx].id;
        world.clans[donor_idx].food += 20;
        world.clans[donor_idx].wood += 20;
        world.clans[recipient_idx].food = 0;
        world.clans[recipient_idx].wood = 0;
        let mut entities = std::mem::take(&mut world.entities);
        world.rebuild_occupancy(&entities);
        let mut courier = entities.remove(donor);
        world.entities = entities;

        for delivery in 1..=3 {
            (courier.x, courier.y) = world.clans[donor_idx].stockpile.unwrap();
            assert!(world.handle_trade(&mut courier, donor_idx));
            let destination = world.clans[recipient_idx].stockpile.unwrap();
            (courier.x, courier.y) = (destination.0 - 1, destination.1);
            assert!(world.handle_trade(&mut courier, donor_idx));
            assert!(courier.trade_returning);
            assert_eq!(
                world.are_allied(donor_id, recipient_id),
                delivery >= 3,
                "a pact must follow repeated delivered aid"
            );
            (courier.x, courier.y) = world.clans[donor_idx].stockpile.unwrap();
            assert!(world.handle_trade(&mut courier, donor_idx));
            assert_eq!(courier.trade_target_clan, -1);
        }
    }

    #[test]
    fn starvation_refunds_dedicated_trade_cargo() {
        let (mut world, donor, _, donor_idx, _) = trade_test_world();
        let mut courier = world.entities.remove(donor);
        assert!(world.handle_trade(&mut courier, donor_idx));
        let cargo = courier.trade_food + courier.trade_wood;
        world.clans[donor_idx].food = 0;
        world.clans[donor_idx].wood = 0;
        world.grid.pellet.fill(0);
        world.pellet_total = 0;
        world.params.starve_ticks = 1;
        world.params.starve_damage = 10.0;
        courier.ticks_since_food = 1;
        courier.health = 1.0;

        world.update_entity(&mut courier);

        assert!(courier.dead);
        assert_eq!(courier.trade_target_clan, -1);
        assert_eq!(world.clans[donor_idx].food + world.clans[donor_idx].wood, 0);
        assert_eq!(world.ground_loot[0].food + world.ground_loot[0].wood, cargo);
    }

    #[test]
    fn trade_ablation_disables_existing_pacts_immediately() {
        let (mut world, _, _, donor_idx, recipient_idx) = trade_test_world();
        let donor_id = world.clans[donor_idx].id;
        let recipient_id = world.clans[recipient_idx].id;
        world
            .diplomacy
            .set_pact(donor_id, recipient_id, world.tick + 1000);
        assert!(world.are_allied(donor_id, recipient_id));

        world.params.community_trade = false;
        assert!(!world.are_allied(donor_id, recipient_id));
        world.refresh_diplomacy();
        assert!(world.clans.iter().all(|clan| clan.trade_partner.is_none()));
        assert!(world
            .clans
            .iter()
            .all(|clan| clan.trade_route_threat.is_none()));
    }

    #[test]
    fn invited_courier_has_narrow_passage_before_an_alliance() {
        let (mut world, courier, recipient, donor_idx, recipient_idx) = trade_test_world();
        world.params.community_care = false;
        world.params.clan_grace_ticks = 0;
        world.params.war_threshold = 10.0;
        (world.entities[courier].x, world.entities[courier].y) = (20, 20);
        world.entities[courier].goal = Goal::Starving;
        world.entities[courier].trade_target_clan = world.clans[recipient_idx].id;
        world.entities[courier].trade_food = 1;
        (world.entities[recipient].x, world.entities[recipient].y) = (21, 20);
        world.entities[recipient].work_role = ClanMode::Defend;
        let tile = world.grid.idx(20, 20);
        world.grid.owner[tile] = world.clans[recipient_idx].id;
        let before = world.entities[courier].health;

        assert!(!world.are_allied(world.clans[donor_idx].id, world.clans[recipient_idx].id));
        world.resolve_combat();

        assert_eq!(world.entities[courier].health, before);
    }

    #[test]
    fn route_guards_ignore_passers_and_can_engage_actual_attackers() {
        let mut world = World::new(40, 0x6A2D);
        world.populate(0, 0, 3);
        world.tick = TRADE_REFRESH_INTERVAL;
        world.params.war_threshold = 10.0;
        world.clans[0].stockpile = Some((5, 5));
        world.clans[1].stockpile = Some((15, 5));
        world.clans[2].stockpile = Some((32, 32));
        let guard = world
            .entities
            .iter()
            .position(|entity| entity.clan == world.clans[0].id && !entity.is_leader)
            .unwrap();
        let attacker = world
            .entities
            .iter()
            .position(|entity| entity.clan == world.clans[2].id && !entity.is_leader)
            .unwrap();
        let bystander = world
            .entities
            .iter()
            .position(|entity| {
                entity.clan == world.clans[2].id
                    && !entity.is_leader
                    && entity.id != world.entities[attacker].id
            })
            .unwrap();
        (world.entities[guard].x, world.entities[guard].y) = (9, 5);
        world.entities[guard].work_role = ClanMode::Defend;
        (world.entities[attacker].x, world.entities[attacker].y) = (10, 5);
        world.entities[attacker].work_role = ClanMode::Gather;

        world.refresh_diplomacy();
        assert_eq!(world.clans[0].trade_route_threat, None);

        world.entities[attacker].work_role = ClanMode::Attack;
        world.refresh_diplomacy();
        assert_eq!(
            world.clans[0].trade_route_threat,
            Some(world.entities[attacker].id)
        );
        (world.entities[attacker].x, world.entities[attacker].y) = (25, 25);
        (world.entities[bystander].x, world.entities[bystander].y) = (10, 5);
        world.entities[bystander].work_role = ClanMode::Gather;
        let bystander_health = world.entities[bystander].health;
        world.resolve_combat();
        assert_eq!(world.entities[bystander].health, bystander_health);

        (world.entities[attacker].x, world.entities[attacker].y) = (10, 5);
        (world.entities[bystander].x, world.entities[bystander].y) = (25, 24);
        world.entities[guard].attack_cooldown = 0;
        let before = world.entities[attacker].health;
        world.resolve_combat();
        assert!(world.entities[attacker].health < before);
    }

    #[test]
    fn allied_passage_suppresses_combat_but_not_foreign_harvesting() {
        let (mut world, donor, recipient, donor_idx, recipient_idx) = trade_test_world();
        world.params.community_care = false;
        world.params.clan_grace_ticks = 0;
        world.params.war_threshold = 0.0;
        world.entities[donor].x = 20;
        world.entities[donor].y = 20;
        world.entities[donor].work_role = ClanMode::Attack;
        world.entities[recipient].x = 21;
        world.entities[recipient].y = 20;
        world.entities[recipient].work_role = ClanMode::Attack;
        let donor_health = world.entities[donor].health;
        let recipient_health = world.entities[recipient].health;
        world.diplomacy.adjust(
            world.clans[donor_idx].id,
            world.clans[recipient_idx].id,
            1.0,
        );
        world.resolve_combat();
        assert_eq!(world.entities[donor].health, donor_health);
        assert_eq!(world.entities[recipient].health, recipient_health);

        let tile = world.grid.idx(20, 20);
        world.grid.owner[tile] = world.clans[recipient_idx].id;
        world.grid.pellet[tile] = 1;
        world.pellet_total += 1;
        let mut outsider = world.entities.remove(donor);
        outsider.ticks_since_food = 0;
        assert!(!world.consume_pellet_at(&mut outsider, false));
        assert_eq!(world.grid.pellet[tile], 1);
    }

    fn trade_test_world() -> (World, usize, usize, usize, usize) {
        let mut world = World::new(40, 0x7ADE);
        world.populate(0, 0, 2);
        world.tick = TRADE_REFRESH_INTERVAL;
        let donor_idx = 0;
        let recipient_idx = 1;
        let donor_id = world.clans[donor_idx].id;
        let recipient_id = world.clans[recipient_idx].id;
        let donor = world
            .entities
            .iter()
            .position(|entity| entity.clan == donor_id && !entity.is_leader)
            .expect("donor courier");
        let recipient = world
            .entities
            .iter()
            .position(|entity| entity.clan == recipient_id && !entity.is_leader)
            .expect("recipient member");
        world.clans[donor_idx].stockpile = Some((8, 8));
        world.clans[recipient_idx].stockpile = Some((14, 8));
        world.clans[donor_idx].trade_partner = Some(recipient_id);
        world.clans[recipient_idx].trade_partner = Some(donor_id);
        for x in 8..=14 {
            let i = world.grid.idx(x, 8);
            world.grid.terrain[i] = terrain::PLAINS;
        }
        for (index, entity) in world.entities.iter_mut().enumerate() {
            entity.x = 2 + index as i32;
            entity.y = 30;
        }
        world.entities[donor].x = 8;
        world.entities[donor].y = 8;
        world.entities[donor].speed = 1.0;
        world.entities[donor].work_role = ClanMode::Gather;
        world.entities[recipient].x = 14;
        world.entities[recipient].y = 9;
        let pop = world.clan_roster_size(donor_idx);
        world.clans[donor_idx].food = pop * TRADE_FOOD_FLOOR_PER_MEMBER + 5;
        world.clans[donor_idx].wood = TRADE_WOOD_FLOOR + 2;
        (world, donor, recipient, donor_idx, recipient_idx)
    }

    #[test]
    fn community_care_turns_a_lethal_hit_into_one_rescue_window() {
        let (mut world, attacker, casualty, _) = care_combat_world(true);
        let second_attacker = world
            .entities
            .iter()
            .position(|entity| entity.clan == world.clans[0].id && !entity.is_leader)
            .expect("second queued attacker");
        world.entities[second_attacker].x = 10;
        world.entities[second_attacker].y = 11;
        world.entities[second_attacker].work_role = ClanMode::Attack;
        world.resolve_combat();
        let deadline = world.entities[casualty].incapacitated_until;
        world.entities[attacker].attack_cooldown = 0;
        world.resolve_combat();

        assert!(!world.entities[casualty].dead);
        assert_eq!(world.entities[casualty].health, 0.0);
        assert_eq!(world.entities[casualty].incapacitated_until, deadline);
        assert!(deadline > world.tick);
        assert_eq!(world.deaths_combat, 0);
        assert_eq!(world.clans[1].stats.incapacitations, 1);
        assert_eq!(world.clans[0].stats.kills, 0);
        assert_eq!(world.clan_population(world.clans[1].id), 3);
    }

    #[test]
    fn defender_physically_carries_an_incapacitated_clanmate_home() {
        let (mut world, _, casualty, rescuer) = care_combat_world(true);
        world.resolve_combat();
        let home = world.clans[1].stockpile.expect("care requires a clan home");
        let direction = if home.0 + 5 < world.grid.size { 1 } else { -1 };
        for step in 0..=5 {
            let x = home.0 + direction * step;
            let i = world.grid.idx(x, home.1);
            world.grid.terrain[i] = terrain::PLAINS;
        }
        world.entities[casualty].x = home.0 + direction * 4;
        world.entities[casualty].y = home.1;
        world.entities[rescuer].x = home.0 + direction * 5;
        world.entities[rescuer].y = home.1;
        world.entities[rescuer].work_role = ClanMode::Defend;
        world.entities[rescuer].speed = 1.0;
        for (index, entity) in world.entities.iter_mut().enumerate() {
            if index != casualty
                && index != rescuer
                && entity.y == home.1
                && (entity.x - home.0).abs() <= 6
            {
                entity.y = world.grid.clamp(home.1 + 3);
            }
        }

        let mut patient_positions = Vec::new();
        for _ in 0..30 {
            world.prepare_rescues();
            for entity in &mut world.entities {
                if entity.rescue_target.is_some() {
                    entity.goal = Goal::Rescuing;
                    entity.move_budget += 1.0;
                }
            }
            world.advance_rescues();
            patient_positions.push((world.entities[casualty].x, world.entities[casualty].y));
            if world.entities[casualty].incapacitated_until == 0 {
                break;
            }
            world.tick += 1;
        }

        assert_eq!(world.entities[casualty].incapacitated_until, 0);
        assert!(world.entities[casualty].health > 0.0);
        assert_eq!(world.entities[casualty].downed_by_clan, -1);
        assert_eq!(world.entities[casualty].carried_by, None);
        assert_eq!(world.entities[rescuer].rescue_target, None);
        assert_eq!(world.clans[1].stats.rescues, 1);
        assert!(patient_positions.windows(2).any(|pair| pair[0] != pair[1]));
        let evacuated = (world.entities[casualty].x, world.entities[casualty].y);
        assert!(
            (evacuated.0 - home.0)
                .abs()
                .max((evacuated.1 - home.1).abs())
                <= 2
        );
    }

    #[test]
    fn care_ablation_and_bleedout_preserve_death_kill_and_loot_accounting() {
        let (mut disabled, attacker, casualty, _) = care_combat_world(false);
        disabled.entities[casualty].food = 2;
        disabled.resolve_combat();
        assert!(disabled.entities[casualty].dead);
        assert_eq!(disabled.deaths_combat, 1);
        assert_eq!(disabled.clans[0].stats.kills, 1);
        assert_eq!(disabled.entities[attacker].food, 0);
        assert_eq!(disabled.ground_loot[0].food, 2);

        let (mut enabled, attacker, casualty, _) = care_combat_world(true);
        enabled.entities[casualty].food = 2;
        enabled.resolve_combat();
        enabled.tick = enabled.entities[casualty].incapacitated_until;
        enabled.prepare_rescues();
        assert!(enabled.entities[casualty].dead);
        assert_eq!(enabled.deaths_combat, 1);
        assert_eq!(enabled.clans[1].stats.bleedouts, 1);
        assert_eq!(enabled.clans[0].stats.kills, 1);
        assert_eq!(enabled.entities[attacker].food, 0);
        assert_eq!(enabled.ground_loot[0].food, 2);
    }

    fn care_combat_world(community_care: bool) -> (World, usize, usize, usize) {
        let mut world = World::new(40, 0xCA2E);
        world.populate(0, 0, 2);
        world.params.community_care = community_care;
        world.params.clan_grace_ticks = 0;
        world.params.war_threshold = 0.0;
        world.params.attack_damage = 100.0;
        world.tick = 1;

        for (i, entity) in world.entities.iter_mut().enumerate() {
            entity.x = 2 + (i as i32 * 3) % 34;
            entity.y = 2 + (i as i32 * 5) % 34;
            entity.attack_cooldown = 0;
            entity.work_role = ClanMode::Gather;
        }
        let attacker = world
            .entity_index(world.clans[0].leader_id)
            .expect("first clan leader");
        let casualty = world
            .entity_index(world.clans[1].leader_id)
            .expect("second clan leader");
        let rescuer = world
            .entities
            .iter()
            .position(|entity| entity.clan == world.clans[1].id && !entity.is_leader)
            .expect("second clan follower");
        world.entities[attacker].x = 10;
        world.entities[attacker].y = 10;
        world.entities[attacker].work_role = ClanMode::Attack;
        world.entities[casualty].x = 11;
        world.entities[casualty].y = 10;
        world.entities[casualty].health = 1.0;
        (world, attacker, casualty, rescuer)
    }
}
