//! Entity: one NPC on the grid. Kept as a plain struct in a `Vec` with
//! swap-remove on death (no per-tick array reallocation like the JS `filter`).

/// What the NPC is currently trying to do — surfaced in the inspector so you
/// can read each NPC's "idea" at a glance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Goal {
    Wander,
    SeekFood,
    Eating,
    Starving,
    Gathering,
    Hauling,
    Fighting,
    Recruiting,
    Defending,
    Claiming,
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
            Goal::Fighting => "fighting",
            Goal::Recruiting => "recruiting",
            Goal::Defending => "defending",
            Goal::Claiming => "claiming land",
        }
    }
}

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
    pub ticks_since_food: i32,
    /// Personal hunger trigger in [0.3, 0.7], re-rolled after each meal.
    pub hunger_threshold: f32,
    pub goal: Goal,
    /// Owning clan id, or -1 when unaffiliated.
    pub clan: i32,
    /// Last cell where this NPC saw or ate food — its food "memory". Lets it
    /// navigate back to a feeding ground instead of wandering off and starving.
    pub last_food: Option<(i32, i32)>,
    /// Ticks until this NPC can attack again.
    pub attack_cooldown: i32,
    pub dead: bool,
}

impl Entity {
    #[inline]
    pub fn hunger(&self, starve_ticks: i32) -> f32 {
        self.ticks_since_food as f32 / starve_ticks as f32
    }
}
