# Roadmap

## Done

The project was rebuilt from the JS/HTML/Node prototype into a native Rust
(egui) application, and the core civ systems are in and verified:

- **Watchable playground** — variable 1–5000 ticks/s, pan/zoom, click-to-inspect,
  toggleable panels, live graphs, one-click presets, a slider for every parameter.
- **Grid-layer core** — typed-array terrain/owner/road/fertility/pellet layers,
  an occupancy grid (one NPC per tile), O(1) ownership, deterministic per-world RNG.
- **Entities** — hunger/health, food memory (no wander-deaths), foraging
  (clan-food-first then survival).
- **Clans + neural leaders** — leader brain picks the goal, workers implement it;
  3 starting followers; deliberate recruiting only; succession & disbanding.
- **Territory as the economy** — **farms** (owned, fertile land grows food),
  **despotic exclusion** (only owners harvest their land), connected rate-limited
  claims toward the **fertile frontier**, cut-off pruning, trespasser defense, and
  an **RDH population cap** keyed to fertile capacity.
- **Villages** — central-place / home-range foraging keeps clans settled; refugee
  genesis re-forms villages from the masterless so the world stays a patchwork.
- **Combat** — trespasser kills, war by aggression, *on-campaign* raiding; war
  parties (the healthy fight, the rest keep farming); loot.
- **Seasons** — a global yield cycle (boom/bust) that drives lean-season wars; the
  season phase is a leader-brain input.
- **Hierarchical neural leaders** — a master gate routing over specialist
  sub-minds (mixture-of-experts); the net picks mode + aggression directly, with
  no hand-coded strategy gates.
- **Evolution, two ways** — an offline arena trainer (parallel rayon, village-
  shaped fitness) *and* in-vivo evolution in the live world, with automatic
  champion transfer from trainer to world.

## Next (civ layers)

- **Soil depletion / patch rotation** — harvesting briefly lowers a tile's yield,
  forcing clans to spread across their claim and onto the frontier.
- **Roads** — buildable road tiles that lower movement cost (hook already wired)
  and later carry trade/logistics.
- **Buildings + tech tree** — houses, granaries, walls, barracks, markets with
  footprints, HP, and production; new brain build/research actions and sub-minds.
- **Trade & diplomacy** — friendly clans exchange food/resources; a second
  resource so holding *connected, varied* land matters; relationship memory.
- **Weapons / military** — equipment from resources + tech that boosts
  attack/defense; brains learn to arm before war.
- **Save / load** — persist a world and champion brains (a `(seed, params)` pair
  already reproduces a run).

## Engineering notes

- **Spatial index** — combat and target search currently rebuild a per-tick
  occupancy hashmap and do bounded scans. If populations grow large, switch to a
  cell-bucketed spatial hash for neighbor queries.
- **Save / load** — persist a world and champion brains to disk (a `(seed,
  params)` pair already reproduces a run; explicit serialization is the next step).
- **Fitness** — the training score is a smooth weighted sum; consider fixed
  benchmark opponents so scores are comparable across generations.
