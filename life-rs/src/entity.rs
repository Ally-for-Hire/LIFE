//! Entity: one NPC on the grid. Kept as a plain struct in a `Vec` with
//! swap-remove on death (no per-tick array reallocation like the JS `filter`).

use crate::clan::ClanMode;

/// What the NPC is currently trying to do — surfaced in the inspector so you
/// can read each NPC's "idea" at a glance.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Goal {
    Wander,
    SeekFood,
    Eating,
    Starving,
    Gathering,
    Hauling,
    GatheringWood,
    HaulingWood,
    BuildingRoad,
    Incapacitated,
    Rescuing,
    Trading,
    GuardingTrade,
    Fighting,
    Recruiting,
    Defending,
    Claiming,
    Constructing,
    Researching,
}

impl Goal {
    pub fn label(&self) -> &'static str {
        match self {
            Goal::Wander => "wandering",
            Goal::SeekFood => "seeking food",
            Goal::Eating => "eating",
            Goal::Starving => "starving",
            Goal::Gathering => "gathering food",
            Goal::Hauling => "hauling to stockpile",
            Goal::GatheringWood => "gathering wood",
            Goal::HaulingWood => "hauling wood",
            Goal::BuildingRoad => "building a road",
            Goal::Incapacitated => "incapacitated",
            Goal::Rescuing => "rescuing a clanmate",
            Goal::Trading => "delivering inter-clan aid",
            Goal::GuardingTrade => "guarding a trade route",
            Goal::Fighting => "fighting",
            Goal::Recruiting => "recruiting",
            Goal::Defending => "defending",
            Goal::Claiming => "claiming land",
            Goal::Constructing => "constructing a building",
            Goal::Researching => "researching technology",
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub speed: f32,
    /// Accrues `speed` per tick; moving one cell costs 1.0.
    pub move_budget: f32,
    pub health: f32,
    pub max_health: f32,
    pub is_leader: bool,
    /// Carried food units.
    pub food: i32,
    /// Carried forest wood units.
    pub wood: i32,
    pub ticks_since_food: i32,
    /// Personal hunger trigger in [0.3, 0.7], re-rolled after each meal.
    pub hunger_threshold: f32,
    pub goal: Goal,
    /// Owning clan id, or -1 when unaffiliated.
    pub clan: i32,
    /// Sticky community job. Hunger and immediate defense may override it.
    pub work_role: ClanMode,
    /// Earliest tick at which normal quota balancing may freely reassign it.
    pub work_until: i32,
    /// Last cell where this NPC saw or ate food — its food "memory". Lets it
    /// navigate back to a feeding ground instead of wandering off and starving.
    pub last_food: Option<(i32, i32)>,
    /// Ticks until this NPC can attack again.
    pub attack_cooldown: i32,
    /// Positive while a combat casualty can still be rescued by its clan.
    pub incapacitated_until: i32,
    /// Clan responsible for the incapacitating blow, used for bleed-out credit.
    pub downed_by_clan: i32,
    /// Attacker responsible for the wound, retained for delayed loot transfer.
    pub downed_by_entity: Option<u32>,
    /// Active rescue assignment for a Gather/Defend worker.
    pub rescue_target: Option<u32>,
    /// Rescuer physically carrying this incapacitated member toward home.
    pub carried_by: Option<u32>,
    /// Partner clan and dedicated cargo for a persistent trade delivery.
    pub trade_target_clan: i32,
    /// Delivery completed; courier is physically returning home under passage.
    pub trade_returning: bool,
    pub trade_food: i32,
    pub trade_wood: i32,
    pub dead: bool,
}

impl Entity {
    #[inline]
    pub fn is_active(&self) -> bool {
        !self.dead && self.incapacitated_until == 0
    }

    #[inline]
    pub fn hunger(&self, starve_ticks: i32) -> f32 {
        self.ticks_since_food as f32 / starve_ticks as f32
    }
}
