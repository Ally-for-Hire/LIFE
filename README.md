# LIFE

Source repository: <https://github.com/Ally-for-Hire/LIFE>

A native, watchable **civilization / life simulator** written in Rust (egui).
Clans led by **evolving neural networks** gather, grow, claim territory, defend
their borders, and wage war on a procedurally generated, terrain-rich grid — all
in one fast desktop window with no browser, HTML, or server.

The project was rewritten from an earlier vanilla-JS / HTML / Node prototype into
a single native application that lives under [`life-rs/`](life-rs/).

## Build & run

Requires the Rust toolchain (installed via [rustup](https://rustup.rs)). This
machine uses the `x86_64-pc-windows-gnu` toolchain (no MSVC / Visual Studio C++
required).

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"   # if cargo isn't on PATH
cd life-rs
cargo run --release        # first release build takes a few minutes, then it's fast
```

> **Windows note:** Smart App Control must be **off** to run locally-built,
> unsigned binaries (Settings → Privacy & security → Windows Security → App &
> browser control → Smart App Control → Off).

## What it does

- **Watchable playground** at a variable **1–5000 ticks/s** — slow enough to
  watch one NPC think, fast enough to run whole civilizations.
- **Procedural terrain** (water, sand, plains, forest, hills, mountains) with a
  **clumped fertility field** — good farmland is rare and patchy, so there are
  valleys worth fighting for.
- **Territory is the economy:** **owned, fertile land grows food** (farms), and
  only its owners may harvest it (despotic exclusion). Villages **settle** on good
  land, work it within a **home range**, **expand** onto the fertile frontier, take
  in **refugees**, and **war over the best valleys** — especially when the
  **seasons** turn lean. A population cap keyed to *fertile* land sizes each
  village.
- **Hierarchical neural leaders:** each clan leader is a **master controller**
  routing over specialist **sub-minds** (a mixture-of-experts). The net picks the
  mode *and* aggression directly — there are **no hand-coded strategy gates**, so
  evolution discovers how to handle famine, growth, and war.
- **Evolution, two ways:** an offline **arena trainer** that **saturates every CPU
  core** (rayon) on a background thread, *and* **in-vivo evolution** in the live
  world (new villages inherit from thriving ones) — with the trainer's champion
  flowing into the world automatically.
- **Survival-gated quality diversity:** extinction cannot be traded for a flashy
  score. Viable brains are preserved in survivor, builder, cooperator, defender,
  and raider niches, while fixed-world benchmarks guard clan-vs-neutral fairness,
  food security, and mixture-of-experts routing health.
- **Combat**, **food-gated reproduction**, **food memory**, and **one NPC per
  tile**.
- Game-like **toggleable panels**, an **NPC inspector** (its current "idea"),
  live **graphs**, **one-click presets**, and a slider for **every world
  parameter**.

## Controls

- **Top bar:** Run/Pause, Step, the `ticks/s` slider, live stats, and panel
  toggles (Controls / Inspector / Graphs / Training).
- **Viewport:** drag to pan, scroll to zoom, click an NPC to inspect it.
- **Controls panel:** presets, populate counts, and every tunable world
  parameter (food/trees, hunger/health, movement/perception, clans/combat,
  growth/expansion, terrain).
- **Training window:** start/stop evolution, edit the training config, watch the
  fitness graph, and seed the best brain into the live world.

## Tests

```powershell
cd life-rs
cargo test --release
cargo test --release ai_quality_benchmark_is_deterministic -- --nocapture
```

`life-rs/champion.bin` is the tracked deployable model. Marathon logs, stage/gen
snapshots, backup champions, and `target/` are generated locally and git-ignored.

## Documentation

See the [`wiki/`](wiki/index.md): Architecture, Simulation & Gameplay, Module
Reference, Controls & UI, Parameters, and Roadmap.
