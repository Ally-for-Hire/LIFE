# Architecture

LIFE is a single native binary: simulation, renderer, and trainer share memory
and run in one process. The view reads world state directly each frame — there is
no JSON snapshot or HTTP layer like the old JS prototype had.

## Modules (`life-rs/src/`)

| File | Responsibility |
| --- | --- |
| `main.rs` | Entry point; sets up the eframe/egui native window. |
| `app.rs` | The egui application: rendering, panels, knobs, graphs, the training window, and the variable-rate sim driver. |
| `world.rs` | `World` + `Params`: the grid, entities, trees, clans, the per-tick `step`, and all gameplay rules. |
| `entity.rs` | `Entity` (one NPC) and its `Goal` (the "idea" shown in the inspector). |
| `clan.rs` | `Clan`, `ClanMode`, clan stats, and the clan color helper. |
| `brain.rs` | `Brain`: the hierarchical mixture-of-experts leader policy — a master gate routing over sub-minds (evaluate / mutate / crossover). |
| `trainer.rs` | `Trainer` + `TrainCfg`: parallel arena evaluation, village-shaped scoring, and evolution. |
| `grid.rs` | `Grid`: the typed-array tile layers and index math. |
| `rng.rs` | `Rng`: a deterministic, seedable xoshiro256** PRNG. |
| `diag.rs` | Headless diagnostics (test-only): run scenarios and print village/conflict metrics. |

## Data model: grid layers

The old prototype kept tile state in `Map<"x,y">` / `Set<"x,y">`, which meant
building a string per access and scanning every clan to answer "who owns this
tile?". Here each layer is a flat `Vec` indexed by `y * size + x`:

- `terrain: Vec<u8>` — water / sand / plains / forest / hill / mountain.
- `fertility: Vec<u8>` — scales food growth.
- `owner: Vec<i32>` — owning clan id per tile (`-1` = none); **O(1) ownership**.
- `road: Vec<u8>` — road level (halves move cost; hook for the roads layer).
- `pellet: Vec<u8>` — food energy per cell.

The world also keeps an **occupancy grid** (`Vec<u16>`, rebuilt each tick) that
enforces **one NPC per tile**, a reusable flood-fill buffer for territory
pruning, and an O(1) running `pellet_total`.

Entities and clans are plain `Vec`s; the dead are removed with in-place
`retain`/swap-remove instead of per-tick reallocation.

## Tick loop (`World::step`)

1. **grow_farms** — owned, fertile tiles grow food (× `season_factor`); farms get
   first call on the food budget so cultivated land out-produces wilderness.
2. **update_trees** — wild food drops on *unclaimed* passable cells only (× season).
3. **clan_think** — every 120 ticks the leader's mixture-of-experts brain picks a
   mode + aggression from a 17-input situation vector (no strategy gates, just
   physical feasibility); every 15 ticks it refreshes cached targets (nearest
   enemy, neutral, trespasser, fertility-scored frontier). Targets are cached on
   the clan so per-entity updates never scan the world.
4. **rebuild occupancy**, then **update each entity** (hunger/individual foraging
   first as a safety net, then the clan-mode behaviour, movement).
5. **recruitment** (deliberate only), **combat** (trespasser / on-campaign / war),
   **raiding** (stockpile theft), **detach the dead** (losses, succession,
   disbanding), **record stats**.
6. **maintain** — refugee-village genesis to a clan floor (+ optional pop floor),
   **reproduce**, and every 200 ticks **prune cut-off territory** (also recomputes
   each clan's `territory` and `fertile_capacity`).

## Threading & determinism

- The **live sim** steps on the UI thread, paced by the `ticks/s` slider against
  wall-clock time (with a per-frame cap so a hitch can't spiral).
- **Training** runs on a background `std::thread`. Each generation snapshots the
  population under a brief lock, then evaluates arenas **in parallel across all
  CPU cores via rayon** (unlocked), then applies results. The UI reads progress
  through `Arc<Mutex<Trainer>>`.
- Every `World` owns its own `Rng`; there is **no global RNG**. Same seed →
  identical run (covered by a test).

## Rendering

Each frame the world is painted into a `Color32` pixel buffer (terrain base →
territory tint → pellets → trees → stockpiles → entities), uploaded as a
NEAREST-filtered texture, and drawn into the viewport with pan/zoom. One cell =
one texel.
