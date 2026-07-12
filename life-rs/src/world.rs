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
use crate::entity::{Entity, Goal};
use crate::grid::{terrain, Grid, NO_OWNER};
use crate::rng::Rng;
use std::collections::HashMap;

// Default values (also slider starting points).
pub const D_STARVE_TICKS: i32 = 1400;
pub const D_STARVE_DAMAGE: f32 = 0.05;
pub const D_HEAL_RATE: f32 = 0.08;
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
const ROAD_BUILD_INTERVAL: i32 = 60;
const ROAD_WOOD_COST: i32 = 2;
const ROAD_MIN_TRAFFIC: u16 = 3;
const RESERVE_FOOD_PER_MEMBER: i32 = 4;
const STOCKPILE_FOOD_PER_MEMBER: i32 = 4;
/// Above this hunger, an outsider will steal from foreign farmland to survive —
/// which makes them a trespasser and triggers the owner's defense. Below it,
/// a clan's crops feed only its own people (despotic exclusion).
const EMERGENCY_STEAL: f32 = 0.9;

#[derive(Clone)]
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
            birth_chance: D_BIRTH_CHANCE,
            birth_interval: D_BIRTH_INTERVAL,
            birth_food_cost: D_BIRTH_FOOD_COST,
            terrain_on: true,
            water_level: D_WATER_LEVEL,
            mountain_level: D_MOUNTAIN_LEVEL,
        }
    }
}

pub struct Tree {
    pub x: i32,
    pub y: i32,
    pub last_spawn: i32,
    pub destroyed: bool,
}

pub struct World {
    pub grid: Grid,
    pub tick: i32,
    pub entities: Vec<Entity>,
    pub trees: Vec<Tree>,
    pub clans: Vec<Clan>,
    pub rng: Rng,
    pub params: Params,
    next_entity_id: u32,
    next_clan_id: i32,
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
    occupied: Vec<u16>, // entities per cell, enforces one NPC per tile
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
            next_entity_id: 1,
            next_clan_id: 1,
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
        self.disaster_level = 0.0;
        self.next_clan_id = 1;
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
        o != NO_OWNER && o != own_clan
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
        } else if self.clans[cidx].reserve_food > 0 {
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
        let pop = self.clan_roster_size(cidx);
        let ordinary_floor = pop * STOCKPILE_FOOD_PER_MEMBER;
        let reserve_cap = pop * RESERVE_FOOD_PER_MEMBER;
        let ordinary_needed = (ordinary_floor - self.clans[cidx].food).max(0);
        let to_ordinary = amount.min(ordinary_needed);
        self.clans[cidx].food += to_ordinary;

        let remaining = amount - to_ordinary;
        let reserve_room = (reserve_cap - self.clans[cidx].reserve_food).max(0);
        let to_reserve = remaining.min(reserve_room);
        self.clans[cidx].reserve_food += to_reserve;
        self.clans[cidx].stats.reserve_deposited += to_reserve as u32;
        self.clans[cidx].food += remaining - to_reserve;
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
            dead: false,
        }
    }

    pub fn spawn_entity(&mut self, x: i32, y: i32, is_leader: bool) {
        let e = self.make_entity(x, y, is_leader);
        self.entities.push(e);
    }

    fn random_cell(&mut self) -> (i32, i32) {
        let s = self.grid.size;
        (self.rng.below(s), self.rng.below(s))
    }

    /// Random unowned, passable cell — keeps spawns off water/mountain and out
    /// of existing clan territory (so they don't spawn straight into a kill).
    fn random_land_cell(&mut self) -> (i32, i32) {
        for _ in 0..50 {
            let (x, y) = self.random_cell();
            let i = self.grid.idx(x, y);
            let t = self.grid.terrain[i];
            if t != terrain::WATER && t != terrain::MOUNTAIN && self.grid.owner[i] == NO_OWNER {
                return (x, y);
            }
        }
        for _ in 0..50 {
            let (x, y) = self.random_cell();
            if self.is_passable(x, y) {
                return (x, y);
            }
        }
        self.random_cell()
    }

    /// A passable, unowned cell biased toward high fertility — picks the most
    /// fertile of several random samples. Used to seed trees (and clans) onto
    /// the good land that's worth settling and fighting for.
    fn random_fertile_land_cell(&mut self) -> (i32, i32) {
        let mut best = self.random_land_cell();
        let mut best_f = self.grid.fertility[self.grid.idx(best.0, best.1)];
        for _ in 0..6 {
            let (x, y) = self.random_land_cell();
            let f = self.grid.fertility[self.grid.idx(x, y)];
            if f > best_f {
                best_f = f;
                best = (x, y);
            }
        }
        best
    }

    fn occupied_by_live_entity(&self, x: i32, y: i32) -> bool {
        self.entities
            .iter()
            .any(|e| !e.dead && e.x == x && e.y == y)
    }

    fn nearby_spawn_cell(&mut self, cx: i32, cy: i32, r: i32) -> (i32, i32) {
        let radius = r.max(1);
        for _ in 0..80 {
            let x = self.grid.clamp(cx + self.rng.range(-radius, radius + 1));
            let y = self.grid.clamp(cy + self.rng.range(-radius, radius + 1));
            if self.is_passable(x, y) && !self.occupied_by_live_entity(x, y) {
                return (x, y);
            }
        }
        for rr in radius + 1..=(radius + 8).min(self.grid.size) {
            for yy in (cy - rr).max(0)..=(cy + rr).min(self.grid.size - 1) {
                for xx in (cx - rr).max(0)..=(cx + rr).min(self.grid.size - 1) {
                    if self.is_passable(xx, yy) && !self.occupied_by_live_entity(xx, yy) {
                        return (xx, yy);
                    }
                }
            }
        }
        self.random_land_cell()
    }

    fn spawn_clan(&mut self) {
        let brain = self.breed_brain();
        self.spawn_clan_with(brain);
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
                + c.reserve_food.max(0) as f32 * 0.12
                + c.stats.roads_built as f32 * 0.4
                + c.stats.reserve_released as f32 * 0.15;
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
        let (x, y) = self.random_land_cell();
        let mut leader = self.make_entity(x, y, true);
        leader.clan = -1;
        let lid = leader.id;
        self.entities.push(leader);

        let id = self.create_clan(lid, x, y, brain);
        let li = self.entities.len() - 1;
        self.entities[li].clan = id;
        let idx = self.clan_index(id).unwrap();

        for _ in 0..3 {
            let (fx, fy) = self.nearby_spawn_cell(x, y, 3);
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
        id
    }

    pub fn populate(&mut self, neutrals: i32, trees: i32, clans: i32) {
        self.clear();
        self.generate_terrain();
        for _ in 0..trees {
            let (x, y) = self.random_fertile_land_cell();
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
            let (x, y) = self.random_land_cell();
            self.spawn_entity(x, y, false);
        }
    }

    pub fn setup_arena(&mut self, brains: &[Brain], trees: i32, neutrals: i32) -> Vec<i32> {
        self.clear();
        self.generate_terrain();
        for _ in 0..trees {
            let (x, y) = self.random_fertile_land_cell();
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
            let (x, y) = self.random_land_cell();
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
                let (fx, fy) = self.nearby_spawn_cell(lx, ly, 4);
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
            .find(|e| e.id == id && !e.dead)
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
            .filter(|e| e.clan == id && !e.dead)
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
        (cap * self.params.members_per_claim.max(1) as f32)
            .round()
            .max(3.0) as usize
    }

    fn clan_can_add_member(&self, clan_idx: usize) -> bool {
        self.clan_population(self.clans[clan_idx].id) < self.clan_member_cap(clan_idx)
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
        if self.grid.road[i] > 0 {
            c *= 0.5; // roads halve cost (built later; hook ready)
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

    /// A cell can be entered if it's passable terrain and not already occupied.
    fn can_enter(&self, x: i32, y: i32) -> bool {
        self.move_cost_to(x, y).is_finite() && self.occupied[self.grid.idx(x, y)] == 0
    }

    fn try_step(&mut self, e: &mut Entity, nx: i32, ny: i32) -> bool {
        let c = self.move_cost_to(nx, ny);
        if !c.is_finite() || e.move_budget < c {
            return false; // impassable, or can't afford yet — wait, keep budget
        }
        let ni = self.grid.idx(nx, ny);
        if self.occupied[ni] > 0 {
            return false; // one NPC per tile — cell taken
        }
        let oi = self.grid.idx(e.x, e.y);
        self.occupied[oi] = self.occupied[oi].saturating_sub(1);
        self.occupied[ni] += 1;
        self.grid.traffic[ni] = self.grid.traffic[ni].saturating_add(1);
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
        // Farms (owned land) get first call on the food budget; wild trees fill
        // what's left — so cultivated territory out-produces the wilderness.
        self.grow_farms();
        self.update_trees();
        self.regrow_wood();
        self.maybe_disaster();
        self.clan_think();

        let mut entities = std::mem::take(&mut self.entities);
        self.rebuild_occupancy(&entities);
        for e in entities.iter_mut() {
            self.update_entity(e);
        }
        self.entities = entities;
        self.build_roads();

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
            if e.dead || e.clan < 0 {
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
            self.clans[i].stats.food_tick_sum +=
                (self.clans[i].food.max(0) + self.clans[i].reserve_food.max(0)) as u64;
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
                if e.clan != id || e.dead {
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
            let birth_chance = chance * reserve_pressure;
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
                    self.clans[ci].food -= cost;
                    let (bx, by) = self.nearby_spawn_cell(sx, sy, 2);
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
            .filter(|e| e.clan < 0 && !e.dead)
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
                    let (bx, by) = self.nearby_spawn_cell(nx, ny, 2);
                    if self.is_passable(bx, by) {
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
                if let Some(ci) = self
                    .clans
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !c.disbanded)
                    .filter(|(i, _)| self.clan_can_add_member(*i))
                    .min_by_key(|(_, c)| self.clan_population(c.id))
                    .map(|(i, _)| i)
                {
                    let id = self.clans[ci].id;
                    let (sx, sy) = self.clans[ci]
                        .stockpile
                        .or_else(|| self.entity_pos(self.clans[ci].leader_id))
                        .unwrap_or_else(|| self.random_land_cell());
                    let (x, y) = self.nearby_spawn_cell(sx, sy, 4);
                    let mut e = self.make_entity(x, y, false);
                    e.clan = id;
                    let eid = e.id;
                    self.entities.push(e);
                    self.clans[ci].members.push(eid);
                    self.clans[ci].food = self.clans[ci].food.max(4);
                } else {
                    self.spawn_clan();
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
            .filter(|(_, e)| e.clan < 0 && !e.dead)
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

    fn clan_think(&mut self) {
        let refresh = self.tick % TARGET_REFRESH_INTERVAL == 0;
        let decide = self.tick % CLAN_THINK_INTERVAL == 0;
        if !refresh && !decide || self.clans.is_empty() {
            return;
        }

        let starve_ticks = self.params.starve_ticks.max(1) as f32;
        let vision = self.params.vision_radius;
        let vision2 = (vision * vision) as i64;
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
        let mut clan_members: Vec<(i32, i32, i32)> = Vec::new();
        let mut neutrals: Vec<(i32, i32)> = Vec::new();

        for e in &self.entities {
            if e.dead {
                continue;
            }
            if e.clan >= 0 {
                if let Some(&idx) = idx_by_id.get(&e.clan) {
                    pop[idx] += 1;
                    hunger_sum[idx] += (e.ticks_since_food as f32 / starve_ticks).clamp(0.0, 2.0);
                    if e.is_leader {
                        leader_pos[idx] = Some((e.x, e.y));
                    }
                }
                clan_members.push((e.x, e.y, e.clan));
            } else {
                neutrals.push((e.x, e.y));
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
            for &(ex, ey, ec) in &clan_members {
                if ec == id {
                    continue;
                }
                let dx = (ex - lp.0) as i64;
                let dy = (ey - lp.1) as i64;
                let d = dx * dx + dy * dy;
                if d <= vision2 {
                    enemies_seen += 1;
                }
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
                if c.disbanded || c.id == id || c.food <= 0 {
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
            for &(nx, ny) in &neutrals {
                let dx = (nx - lp.0) as i64;
                let dy = (ny - lp.1) as i64;
                let d = dx * dx + dy * dy;
                if d <= vision2 {
                    neutrals_seen += 1;
                }
                // a neutral standing on our land is a trespasser too
                if self.grid.owner[self.grid.idx(nx, ny)] == id && d < td {
                    td = d;
                    tres = Some((nx, ny));
                }
                if d < nd {
                    nd = d;
                    npos = Some((nx, ny));
                }
            }
            if npos.is_some() {
                // recover the nearest neutral's id for deliberate recruiting
                let target = neutrals
                    .iter()
                    .min_by_key(|&&(nx, ny)| (nx - lp.0).pow(2) as i64 + (ny - lp.1).pow(2) as i64);
                if let Some(&(nx, ny)) = target {
                    ntarget = self.entity_near(nx, ny, 0);
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
                let food = (self.clans[idx].food + self.clans[idx].reserve_food) as f32;
                let terr = self.clans[idx].territory as f32;
                let avg_hunger = if pop[idx] > 0 {
                    hunger_sum[idx] / pop[idx] as f32
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
                    (self.season_factor() - 1.0) / self.params.season_amp.max(0.01), // 15 season phase [-1,1]
                    // --- RESERVED future-feature inputs (read 0.0 until the world
                    //     wires them up; the brain's size never changes) ---
                    road_access, // 16 roads / logistics access near home
                    0.0,         // 17 buildings / settlement development level
                    0.0,         // 18 tech / research level
                    0.0,         // 19 military strength / equipment level
                    (self.clans[idx].wood as f32 / (size * 3.0).max(1.0)).min(1.0), // 20 stored wood / head
                    available_wood, // 21 reachable forest wood
                    0.0,            // 22 diplomacy: relation with nearest clan (0 enemy .. 1 ally)
                    0.0,            // 23 number of allies / trade partners
                    0.0,            // 24 active trade inflow / volume
                    0.0,            // 25 day/night phase (distinct from season)
                    self.clans[idx].soil_depletion.min(1.0), // 26 soil depletion of owned tiles
                    self.disaster_level, // 27 active disaster/event severity
                    0.0,            // 28 average member morale / health
                    0.0,            // 29 water / coast / river access of territory
                    0.0,            // 30 spare
                    1.0,            // 31 bias
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
            .filter(|(_, e)| e.clan == clan_id && !e.dead && e.id != leader_id)
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
            .position(|e| e.id == leader_id && e.clan == clan_id && !e.dead);
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

    fn update_entity(&mut self, e: &mut Entity) {
        e.ticks_since_food += 1;
        e.attack_cooldown = (e.attack_cooldown - 1).max(0);
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
            if self.clans[cidx].food > 0 || self.clans[cidx].reserve_food > 0 {
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
                    e.dead = true;
                    self.deaths_starved += 1;
                    return;
                }
            }
            self.forage(e); // neutral-style survival fallback (avoids enemy land)
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
                    e.goal = Goal::Wander;
                    self.random_walk(e, true);
                } else {
                    self.gather(e, cidx);
                }
            }
            ClanMode::Gather => {
                if self.should_gather_wood(e, cidx) {
                    self.gather_wood(e, cidx);
                } else {
                    self.gather(e, cidx);
                }
            }
        }
    }

    fn should_gather_wood(&self, e: &Entity, cidx: usize) -> bool {
        if e.wood > 0 {
            return true;
        }
        let pop = self.clan_roster_size(cidx);
        let food_safe = self.clans[cidx].food >= pop * STOCKPILE_FOOD_PER_MEMBER;
        let road_workers = self.clans[cidx].workforce[ClanMode::Expand.index()] as i32;
        let wood_target = ROAD_WOOD_COST * (road_workers.max(1) + 2);
        food_safe && road_workers > 0 && self.clans[cidx].wood < wood_target
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
        if let Some((sx, sy)) = self.clans[cidx].stockpile {
            e.goal = Goal::Defending;
            let d = (e.x - sx).abs().max((e.y - sy).abs());
            if d > 6 {
                self.move_toward(e, sx, sy, true);
            } else {
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
                (t.x, t.y, t.clan < 0 && !t.dead)
            };
            if !available {
                continue;
            }
            let id = self.clans[ci].id;
            let near_member = self.entities.iter().any(|e| {
                e.clan == id
                    && !e.dead
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
            if e.dead || e.clan < 0 {
                continue;
            }
            let hunger = e.hunger(starve_ticks);
            for &(victim_idx, victim_id, sx, sy) in &stockpiles {
                if victim_id == e.clan || e.x != sx || e.y != sy {
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
            if ec < 0 || cool > 0 || self.entities[i].dead {
                continue; // only clan members attack
            }
            let aggr_self = self.clan_aggr(ec);
            // A clan ordered to Attack strikes enemy clans wherever it meets
            // them (it's on campaign), not only on its own soil — this is what
            // lets a land-hungry clan press into a neighbour's territory.
            let on_campaign = !grace && self.entities[i].work_role == ClanMode::Attack;
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
                            if o.dead || o.clan == ec {
                                continue;
                            }
                            let on_my_land = self.grid.owner[self.grid.idx(o.x, o.y)] == ec;
                            let at_war =
                                !grace && o.clan >= 0 && aggr_self + self.clan_aggr(o.clan) >= war;
                            let raid = on_campaign && o.clan >= 0;
                            if on_my_land || at_war || raid {
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
                || self.entities[j].dead
            {
                continue;
            }
            self.entities[i].attack_cooldown = cd;
            self.entities[j].health -= dmg;
            if self.entities[j].health <= 0.0 {
                self.entities[j].dead = true;
                let loot = self.entities[j].food;
                self.entities[j].food = 0;
                self.entities[i].food += loot;
                self.deaths_combat += 1;
                let attacker_clan = self.entities[i].clan;
                if let Some(ci) = self.clan_index(attacker_clan) {
                    self.clans[ci].stats.kills += 1;
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
            if cid < 0 {
                continue;
            }
            let ci = match self.clan_index(cid) {
                Some(x) => x,
                None => continue,
            };
            self.clans[ci].stats.losses += 1;
            let dead_id = self.entities[i].id;
            if self.clans[ci].leader_id == dead_id {
                let members = self.clans[ci].members.clone();
                let mut successor = None;
                for mid in members {
                    if let Some(mi) = self.entity_index(mid) {
                        if !self.entities[mi].dead {
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
            if let Some(mi) = self.entity_index(mid) {
                self.entities[mi].clan = -1;
            }
        }
    }

    fn regrow_wood(&mut self) {
        if self.tick % WOOD_REGROW_INTERVAL != 0 {
            return;
        }
        for i in 0..self.grid.wood.len() {
            if self.grid.terrain[i] == terrain::FOREST
                && self.grid.wood[i] < FOREST_WOOD_CAP
                && self.rng.chance(WOOD_REGROW_CHANCE)
            {
                self.grid.wood[i] += 1;
            }
        }
    }

    /// Expand workers turn stored wood into roads on the clan's busiest owned
    /// cells. Traffic decays after each construction pass, keeping placement
    /// responsive to recent hauling and reinforcement routes.
    fn build_roads(&mut self) {
        if self.tick % ROAD_BUILD_INTERVAL != 0 {
            return;
        }
        let mut clan_by_id = HashMap::new();
        for (idx, clan) in self.clans.iter().enumerate() {
            if !clan.disbanded
                && clan.wood >= ROAD_WOOD_COST
                && clan.workforce[ClanMode::Expand.index()] > 0
            {
                clan_by_id.insert(clan.id, idx);
            }
        }
        let mut best: Vec<Option<(u16, usize)>> = vec![None; self.clans.len()];
        for i in 0..self.grid.owner.len() {
            if self.grid.road[i] > 0 || self.grid.traffic[i] < ROAD_MIN_TRAFFIC {
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
            if self.clans[ci].wood < ROAD_WOOD_COST || self.grid.road[i] > 0 {
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
                .filter(|e| e.clan == clan_id && !e.dead && e.work_role == ClanMode::Expand)
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
    pub fn season_factor(&self) -> f32 {
        let len = self.params.season_length;
        if len <= 0 || self.params.season_amp <= 0.0 {
            return 1.0;
        }
        let phase = (self.tick as f32 / len as f32) * std::f32::consts::TAU;
        (1.0 + self.params.season_amp * phase.sin()).max(0.0)
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
                self.grid.depletion[i] = self.grid.depletion[i].saturating_sub(2);
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
            if self.rng.f32() < yield_rate * fert * soil {
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
                    self.grid.pellet[i] = energy;
                    self.pellet_total += 1;
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
            (w.population(), w.deaths_starved, w.deaths_combat)
        };
        assert_eq!(run(), run(), "same seed must produce identical runs");
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
        w.populate(0, 0, 1);
        let ci = 0;
        let floor = w.clan_roster_size(ci) * STOCKPILE_FOOD_PER_MEMBER;
        w.clans[ci].food = floor;
        w.deposit_food(ci, 5);
        assert_eq!(w.clans[ci].food, floor);
        assert_eq!(w.clans[ci].reserve_food, 5);

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
        w.tick = ROAD_BUILD_INTERVAL;
        w.build_roads();
        assert_eq!(w.grid.road[road_i], 1);
        assert_eq!(w.clans[ci].wood, 0);
        assert_eq!(w.clans[ci].stats.roads_built, 1);

        w.clear();
        assert!(w.grid.road.iter().all(|&v| v == 0));
        assert!(w.grid.wood.iter().all(|&v| v == 0));
        assert!(w.grid.traffic.iter().all(|&v| v == 0));
    }
}
