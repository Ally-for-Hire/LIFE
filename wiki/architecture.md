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
| `diplomacy.rs` | Deterministic sorted relationship ledger: trust, temporary pacts, delivered volume, and decay/pruning. |
| `brain.rs` | `Brain`: the hierarchical mixture-of-experts leader policy — a master gate routing over sub-minds (evaluate / mutate / crossover). |
| `quality.rs` | Survival/security metrics, routing-health probes, and the survivor/builder/cooperator/defender/raider niche definitions. |
| `trainer.rs` | `Trainer` + `TrainCfg`: parallel arena evaluation, fixed behavioral benchmarks, quality-diversity archive, and evolution. |
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
- `road: Vec<u8>` — completed community roads; road cells halve movement cost.
- `wood: Vec<u8>` — harvestable material on forest tiles, with deterministic regrowth.
- `traffic: Vec<u16>` — recent movement pressure used to place roads on useful routes.
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
3. **clan_think** — every 120 ticks the leader's mixture-of-experts brain turns
   all six feasible action utilities into deterministic, sticky workforce quotas
   while retaining a headline mode + aggression. Inputs 16/20/21 expose road
   coverage, stored wood, and nearby wood availability without changing the fixed
   32-input brain shape. Every 15 ticks it refreshes cached targets (nearest
   enemy, neutral, trespasser, fertility-scored frontier). Targets are cached on
   the clan so per-entity updates never scan the world. A preceding diplomacy
   refresh selects partners, decays relationship memory, and caches route threats; inputs
   22–24 expose relation, partner count, and delivered volume.
4. **prepare rescues**, **rebuild occupancy**, then **update each entity**
   (hunger first; assigned care response next; then its ordinary community role,
   movement, delivery, and road work). **advance rescues** physically carries
   patients toward the clan stockpile.
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
- Survival and food security are hard eligibility gates. Eligible brains update
  a persistent five-niche quality-diversity archive; those specialists are kept
  in the breeding pool alongside the strongest generalist. Routing entropy and
  expert coverage provide a small tie-shaping pressure against expert collapse.
- The tracked champion is regression-tested on deterministic fixed worlds. The
  benchmark follows initial clan and neutral cohorts separately so recruitment
  cannot hide a clan-vs-neutral survival regression.
- Marathon promotion is two-stage. A challenger must first beat the incumbent on
  the 24 fixed headline worlds, then pass a 13-world paired logistics-on/off gate.
  The second gate enforces absolute survival/security/fairness/routing floors,
  incumbent non-regression tolerances, positive transport and reserve effects,
  and causal logistics-value retention. Only a passing challenger is serialized.
- Every `World` owns its own `Rng`; there is **no global RNG**. Same seed →
  identical run (covered by a test).
- `community_logistics=false` is a deterministic infrastructure ablation. Wood
  regrowth still consumes the same per-forest RNG draws but does not mutate the
  layer, preventing avoidable RNG drift from the regrowth branch. Later divergence
  caused by the mechanics changing movement, survival, or population is causal.
  Existing roads remain in state but are ignored by movement-cost/pathing calculations.
- `community_care=false` is an immediate-combat-death control. In the enabled
  arm, only active entities participate in work, hostility, reproduction, and
  cohort survival; deterministic rescue assignments and persistent carrying
  links make care part of world state without changing brain dimensions.
- `community_trade=false` keeps reserved inputs 22–24 at zero and disables every
  pact, courier, relationship target, allied-passage, and route-defense effect.
  Trade uses no feature-specific random draws, so paired arms diverge only after
  delivered resources and diplomacy alter behavior.

## Logistics validation counters

V1 activity counters (`wood_delivered`, `roads_built`, reserve deposits/releases)
show that the system ran. V1.1 adds causal-use evidence: `food_delivered` measures
hauling throughput, `road_steps` counts real member movement on active roads, and
`road_cost_saved_milli` accumulates the movement cost those road steps avoided.
Quality/training expose hauling throughput and road utility separately while
retaining the composite logistics field for compatibility.

## Community care validation counters

Each clan records incapacitations, completed rescues, and bleed-outs. Quality uses
the completed-rescues/incapacitations ratio rather than raw activity, so creating
more casualties cannot improve selection. A paired benchmark checks survival,
security, and clan fairness with care enabled/disabled; focused forced-combat
tests prove wound, assignment, physical evacuation, recovery, and delayed death
accounting when the peaceful tracked champion creates no natural opportunities.

## Rendering

Each frame the world is painted into a `Color32` pixel buffer (terrain base →
territory tint → wood/roads → pellets → trees → stockpiles → entities), uploaded as a
NEAREST-filtered texture, and drawn into the viewport with pan/zoom. One cell =
one texel.
