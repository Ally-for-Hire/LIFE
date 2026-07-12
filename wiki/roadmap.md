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
- **Generalized robust training** — common-random-number world evaluation,
  domain randomization, stage curriculum, hall-of-fame self-play, and fixed
  king-of-the-hill champion benchmarks.
- **Environmental pressure** — soil depletion and deterministic regional
  disasters enter progressively at higher curriculum stages.
- **Survival-gated quality diversity** — deterministic clan-vs-neutral and
  routing-health benchmarks plus persistent survivor, builder, cooperator,
  defender, and raider elites.
- **Community Logistics V1** — deterministic sticky workforce quotas let one
  village gather, expand, defend, recruit, attack, and scout simultaneously;
  renewable forest wood feeds shared stockpiles, travelled owned routes become
  wood-built roads, and emergency food reserves protect clans through shocks.
- **Logistics Validation V1.1 instrumentation** — a live deterministic
  infrastructure ablation, no-road-benefit control semantics, food-delivery
  throughput, real road member-steps, and measured movement-cost savings replace
  road-building activity as the evidence needed for causal comparison.
- **Guarded Retraining V1.2** — marathon champion promotion is fail-closed behind
  fixed-world quality plus paired logistics/trade gates, preserving survival, food
  security, clan fairness, routing health, transport value, and reserve behavior.
- **Community Care V1** — lethal combat creates a bounded rescue window;
  Gather/Defend workers physically carry casualties home, while inactive patients
  are excluded from work, reproduction, hostility, and survival metrics.
- **Trade/Diplomacy V1** — stable symmetric relationship memory, temporary pacts,
  survival-buffered physical food/wood delivery, allied passage, route defense,
  causal counters, and a paired no-trade benchmark.
- **Full-world save/load V1** — versioned/checksummed atomic snapshots, exact RNG
  continuation, state/reference validation, UI controls, and trainer-sync isolation.

## Buildings and technology V1

Food-secure clans now spend harvested wood on physical one-cell construction
sites. Expand workers complete houses, granaries, workshops, markets, and walls;
Scout leaders perform research at workshops. The settlement ablation and
promotion gate require construction, causal public-good value, survival, food
security, and clan fairness. `LIFEWRLD` V2 persists the complete settlement state
and explicitly migrates V1 worlds.

## Next (civ layers)

- **Weapons / military** — equipment from resources + tech that boosts
  attack/defense; brains learn to arm before war.

## Engineering notes

- **Spatial index** — combat and target search currently rebuild a per-tick
  occupancy hashmap and do bounded scans. If populations grow large, switch to a
  cell-bucketed spatial hash for neighbor queries.
- **Quality diversity** — the five-niche archive is intentionally compact. Add
  richer behavior descriptors only when new economy/diplomacy mechanics create
  genuinely distinct strategic axes.
