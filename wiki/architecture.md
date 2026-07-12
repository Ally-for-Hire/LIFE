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
| `world/persistence.rs` | Versioned `LIFEWRLD` DTOs and migration, validation, checksum envelope, atomic replacement, and deterministic continuation tests. |
| `settlement.rs` | Deterministic building, cost, technology, development, and settlement-stat data contracts. |
| `military.rs` | Deterministic deposits, carried ore, recipes/projects, physical equipment ownership, and military counters. |
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
pruning, and an O(1) running `pellet_total`. Settlement data stays at World level:
`buildings` owns stable sites, `building_cells` is a one-cell footprint lookup,
`settlements` is sorted per-clan tech/project/stat state, and
`community_settlement` is the live treatment switch. Military state is also
World-level so frozen V1/V2 nested DTOs stay decodable: stable deposits, sorted
entity cargo/loadouts, sorted clan production state, and `community_military`.

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
   22–24 expose relation, partner count, and delivered volume. Inputs 17–18 expose
   settlement development and technology. Food-secure clans may reserve wood for
   a new physical construction site at this cadence.
4. **plan settlement and military work**, **prepare rescues**, **rebuild occupancy**, then **update each entity**
   (hunger first; assigned care response next; then its ordinary community role,
   movement, delivery, road work, physical Expand construction, or physical Scout
   workshop research, ore mining/hauling, or adjacent workshop forging). **advance rescues** physically carries patients toward the
   clan stockpile.
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
  a persistent six-slot quality-diversity archive: survivor, builder, cooperator,
  defender, raider, and contextual specialist. Those elites stay in the breeding
  pool alongside the strongest generalist.
- MoE quality uses 16 deterministic context probes. Its composite combines
  balanced expert utilization, decisive per-context routing, normalized
  context/expert mutual information, top-1 coverage, and output divergence among
  selected experts. This rejects both uniform soft mixing and one-expert collapse;
  it shapes selection only after survival/security/fairness eligibility.
- The tracked champion is regression-tested on deterministic fixed worlds. The
  benchmark follows initial clan and neutral cohorts separately so recruitment
  cannot hide a clan-vs-neutral survival regression.
- Marathon promotion is two-stage. A challenger must first beat the incumbent on
  the 24 fixed headline worlds, then pass 13-world paired logistics, trade, and
  settlement treatment/control gates.
  The second gate enforces absolute survival/security/fairness/specialization floors,
  incumbent non-regression tolerances, positive transport and reserve effects,
  causal logistics-value retention, settlement infrastructure, and technology/
  physical-research non-regression. Only a passing challenger is serialized.
- Every `World` owns its own `Rng`; there is **no global RNG**. Same seed →
  identical run (covered by a test).
- Full-world persistence stores every behavior-affecting field in vector order,
  including exact xoshiro state and cached decisions. Only `reach` and `occupied`
  scratch buffers are omitted and rebuilt. V2 adds buildings, the one-cell
  footprint layer, clan technology/stats, and the settlement ablation; V1 loads
  migrate explicitly to empty enabled settlement state. V3 wraps frozen V2 with
  deposits, cargo, equipment, production/counters, and the military ablation;
  V1/V2 migrate to enabled military with deterministically regenerated reachable
  deposits and empty cargo/production/ownership state.
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
- `community_settlement=false` keeps inputs 17–18 at zero and disables project
  planning, construction, research, and every building effect while retaining the
  structural treatment state for a clean live counterfactual. Excess reserve food
  above the non-granary cap is discarded on a mid-world toggle.
- `community_military=false` keeps inputs 19/30 at zero and disables mining,
  hauling, forging, readiness scoring, weapon bonuses, armor protection, and
  military counters while retaining valid deposits, projects, and ownership.

## Military validation counters

Clan military state records extracted/delivered ore, adjacent forge work, completed
and equipped items, equipped-member ticks, unsafe work, bonus damage, prevented
damage, and loss cleanup. The paired gate requires the same tracked clan to deliver
ore, complete production, and own equipment while preserving survival, food
security, mean/worst clan fairness, and incumbent supply/ownership.
Historical logistics, care, trade, and settlement paired tests explicitly disable
the newer military layer so their established contracts remain comparable; the
military pair is the integrated gate with every earlier layer enabled.

## Settlement validation counters

Each clan records physical construction work, completed buildings, workshop
research ticks, tech levels, granary reserve use, shelter healing, market material,
and wall damage prevention. The paired benchmark requires survival/security/fairness
floors, completed physical construction, and positive causal public-good value;
marathon promotion also guards incumbent infrastructure non-regression.

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
territory tint → wood/roads → ore/pellets → trees → buildings → stockpiles → equipped entities), uploaded as a
NEAREST-filtered texture, and drawn into the viewport with pan/zoom. One cell =
one texel.
