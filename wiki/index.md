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
valleys** — especially when **seasons** turn lean. Almost nothing is hand-scripted:
the nets pick what to do, and evolution (offline in arenas + in-vivo in the live
world) finds the strategies. See [prior-art.md](prior-art.md) and
[devlog.md](devlog.md).

## Quick facts

- **Determinism:** every `World` owns its own seeded PRNG — same seed, identical run.
- **Performance:** the view reads sim memory directly; training fans independent
  arenas across all CPU cores via rayon.
- **One NPC per tile:** movement is gated by a per-cell occupancy grid.
- **Death only from scarcity or combat:** NPCs remember food and home to it, so
  they never wander off and starve for no reason.
- **Build/run:** `cd life-rs && cargo run --release`.
