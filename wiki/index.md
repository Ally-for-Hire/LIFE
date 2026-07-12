# LIFE Wiki

LIFE is a native Rust (egui) civilization/life simulator: clans led by evolving
neural networks live on a procedurally generated, terrain-rich grid, gathering
food, growing, claiming territory, defending borders, and warring — watchable
live at 1–5000 ticks/s in one fast desktop window.

The simulation, renderer, and trainer all live in [`life-rs/`](../life-rs/) and
run in a single process (no browser, HTML, HTTP, or serialization to draw).

## Pages

| Page | What it covers |
|---|---|
| [Architecture](architecture.md) | Modules, grid-layer data model, tick loop, threading, determinism |
| [Simulation & Gameplay](behaviors.md) | Entities, clans, farms/territory, foraging, combat, seasons — the rules |
| [Module Reference](classes.md) | The Rust types (`World`, `Clan`, `Brain`, `Trainer`, …) and what they own |
| [Prior art & design](prior-art.md) | Sugarscape, central-place foraging, IFD/IDD, RDH — why the design is what it is |
| [Devlog](devlog.md) | Running log of attempts, ideas, and measured results |
| [Controls & UI](editor.md) | Panels, the tick-rate slider, inspector, presets, training window |
| [Parameters](save-format.md) | Every tunable world parameter and the training config |
| [Roadmap](roadmap.md) | What's done and what's next |

## The core idea

Clans are villages whose leaders are **evolving hierarchical neural nets** (a
master controller routing over specialist sub-minds). Territory is the economy:
**owned, fertile land grows food** (farms), and only its owners may harvest it
(despotic exclusion), so villages settle on good land, work it within a home
range, expand onto fertile frontier, take in refugees, and **fight over the best
valleys** — especially across named seasons: spring renewal, summer prosperity,
autumn preparation, and winter scarcity. Almost nothing is hand-scripted:
the nets allocate a simultaneous workforce, communities turn forest wood into
roads and emergency reserves, and evolution (offline in arenas + in-vivo in the live
world) finds the strategies. See [prior-art.md](prior-art.md) and
[devlog.md](devlog.md).

## Quick facts

- **Determinism:** every `World` owns its own seeded PRNG — same seed, identical run.
- **Performance:** the view reads sim memory directly; training fans independent
  arenas across all CPU cores via rayon.
- **One NPC per tile:** movement is gated by a per-cell occupancy grid.
- **Community logistics:** sticky roles, renewable wood, traffic-shaped roads,
  and protected famine/disaster reserves create complementary village jobs;
  V1.1 adds a live ablation plus hauling/road-benefit evidence.
- **Community care:** Gather/Defend workers can physically evacuate incapacitated
  clanmates before bleed-out; a live ablation restores immediate combat death.
- **Trade/diplomacy:** surplus food/wood travels by physical courier, delivery
  builds symmetric trust, and temporary partners gain non-aggression and passage.
- **World persistence:** `world.lifeworld` is versioned, checksummed, validated,
  atomically replaced, and resumes the exact saved deterministic trajectory.
- **Buildings and technology:** food-secure clans construct physical settlement
  sites with Expand workers; Scout leaders research at workshops, and completed
  buildings provide measured public goods behind a live causal ablation.
- **Military equipment:** one safe Gather miner hauls finite ore and an Expand
  smith physically forges entity-owned equipment at a Workshop; paired promotion
  proves the supply chain without rewarding violence over survival.
- **Death only from scarcity or combat:** NPCs remember food and home to it, so
  they never wander off and starve for no reason.
- **Build/run:** `cd life-rs && cargo run --release`.
