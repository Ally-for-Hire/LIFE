//! Typed-array grid layers — the foundation the JS sim never had.
//!
//! The JS version stored all per-tile state as `Map<"x,y", _>` / `Set<"x,y">`,
//! which meant building a string per access and scanning every clan to answer
//! "who owns this tile?". Here each layer is a flat `Vec` indexed by
//! `y * size + x`, so every tile query is O(1) and cache-friendly. This is what
//! makes terrain, roads, ownership, fog, and buildings cheap to add.

/// Terrain kinds. Plains for now; the rest are reserved so the renderer,
/// movement cost, and production code can light up without a data migration.
pub mod terrain {
    pub const PLAINS: u8 = 0;
    pub const WATER: u8 = 1;
    pub const FOREST: u8 = 2;
    pub const HILL: u8 = 3;
    pub const MOUNTAIN: u8 = 4;
    pub const SAND: u8 = 5;
}

pub const NO_OWNER: i32 = -1;

pub struct Grid {
    pub size: i32,
    /// Terrain class per tile (see `terrain`).
    pub terrain: Vec<u8>,
    /// Soil fertility 0..=255; scales tree/pellet growth and (later) farming.
    pub fertility: Vec<u8>,
    /// Owning clan id per tile, or `NO_OWNER`. O(1) ownership lookups.
    pub owner: Vec<i32>,
    /// Road level per tile, 0 = none. Higher = cheaper movement (later).
    pub road: Vec<u8>,
    /// Pellet energy per tile, 0 = none. Replaces the JS pellet `Map`.
    pub pellet: Vec<u8>,
    /// Soil depletion per tile, 0 = fresh .. 255 = exhausted. Harvesting raises
    /// it; it recovers over time. Scales farm yield, so a clan must spread across
    /// and rotate its land rather than camp a few tiles. 0 everywhere when the
    /// `soil_depletion_rate` param is 0 (feature off).
    pub depletion: Vec<u8>,
}

impl Grid {
    pub fn new(size: i32) -> Self {
        let n = (size * size) as usize;
        Grid {
            size,
            terrain: vec![terrain::PLAINS; n],
            fertility: vec![128; n],
            owner: vec![NO_OWNER; n],
            road: vec![0; n],
            pellet: vec![0; n],
            depletion: vec![0; n],
        }
    }

    #[inline]
    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.size + x) as usize
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.size && y < self.size
    }

    #[inline]
    pub fn clamp(&self, v: i32) -> i32 {
        v.clamp(0, self.size - 1)
    }

    #[inline]
    pub fn pellet_at(&self, x: i32, y: i32) -> u8 {
        self.pellet[self.idx(x, y)]
    }
}
