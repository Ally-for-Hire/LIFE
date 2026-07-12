//! Clan: a leader plus followers, sharing a color and a neural brain.
//!
//! The brain sets `mode` and `aggression` each decision; per-entity behavior
//! reads those (plus the cached target positions computed in the clan pre-pass),
//! so no entity ever has to scan the world during its own update.

use crate::brain::Brain;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClanMode {
    Gather,
    Recruit,
    Expand,
    Attack,
    Defend,
    Scout,
}

impl ClanMode {
    pub fn from_index(i: usize) -> ClanMode {
        match i {
            0 => ClanMode::Recruit,
            1 => ClanMode::Expand,
            2 => ClanMode::Gather,
            3 => ClanMode::Attack,
            4 => ClanMode::Defend,
            _ => ClanMode::Scout,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            ClanMode::Gather => "gather",
            ClanMode::Recruit => "recruit",
            ClanMode::Expand => "expand",
            ClanMode::Attack => "attack",
            ClanMode::Defend => "defend",
            ClanMode::Scout => "scout",
        }
    }
}

#[derive(Default)]
pub struct ClanStats {
    pub kills: u32,
    pub losses: u32,
    pub recruits: u32,
    pub peak_pop: u32,
    pub founded_tick: i32,
    pub alive_ticks: u32,
    pub pop_tick_sum: u64,
    pub hunger_tick_sum: f32,
    pub starving_ticks: u32,
    pub food_tick_sum: u64,
    /// Member-ticks spent standing on the clan's own land — measures how much the
    /// clan actually *settles and works* its territory vs roams. Drives fitness
    /// toward villages that use their land, not nomads that just claim it.
    pub on_terr_tick_sum: u64,
}

pub struct Clan {
    pub id: i32,
    pub leader_id: u32,
    pub members: Vec<u32>, // followers, leader excluded
    pub color: [u8; 3],
    pub brain: Brain,
    pub stockpile: Option<(i32, i32)>,
    pub food: i32,
    pub territory: u32,
    /// Sum of (fertility/255) over owned tiles — the *productive* size of the
    /// territory (Resource Dispersion Hypothesis). Drives the population cap, so
    /// a clan on a fertile valley supports a real village while one on scrub
    /// stays small and is pressured to expand or fight toward better land.
    pub fertile_capacity: f32,
    /// Mean soil depletion (0..1) over owned tiles — how exhausted the clan's
    /// farmland is. Fed to the leader brain so it can learn to rotate/expand.
    pub soil_depletion: f32,
    pub aggression: f32,
    pub mode: ClanMode,
    // cached targets from the pre-pass, read during entity updates
    pub enemy_pos: Option<(i32, i32)>,
    pub recruit_target: Option<u32>,
    pub neutral_pos: Option<(i32, i32)>,
    /// Frontier tile a worker should walk to and claim next (Expand goal).
    pub expand_target: Option<(i32, i32)>,
    /// Nearest non-member standing on this clan's territory (to hunt & kill).
    pub trespasser_pos: Option<(i32, i32)>,
    /// Tick of the clan's last territory claim (for the claim rate limit).
    pub last_claim_tick: i32,
    pub stats: ClanStats,
    pub disbanded: bool,
}

impl Clan {
    pub fn new(id: i32, leader_id: u32, color: [u8; 3], brain: Brain, founded_tick: i32) -> Self {
        Clan {
            id,
            leader_id,
            members: Vec::new(),
            color,
            brain,
            stockpile: None,
            food: 0,
            territory: 0,
            fertile_capacity: 0.0,
            soil_depletion: 0.0,
            aggression: 0.0,
            mode: ClanMode::Gather,
            enemy_pos: None,
            recruit_target: None,
            neutral_pos: None,
            expand_target: None,
            trespasser_pos: None,
            last_claim_tick: -100000,
            stats: ClanStats {
                founded_tick,
                peak_pop: 1,
                ..Default::default()
            },
            disbanded: false,
        }
    }

    /// People count = leader (1) + followers. (Liveness is filtered by caller.)
    pub fn size(&self) -> usize {
        1 + self.members.len()
    }
}

/// HSV→RGB for distinct clan colors (full sat/val, hue spread by id).
pub fn hue_color(hue_deg: f32) -> [u8; 3] {
    let h = (hue_deg.rem_euclid(360.0)) / 60.0;
    let c = 0.85f32;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = 0.18f32; // lift so colors aren't too dark on the dark background
    [
        (((r + m) * 255.0).min(255.0)) as u8,
        (((g + m) * 255.0).min(255.0)) as u8,
        (((b + m) * 255.0).min(255.0)) as u8,
    ]
}
