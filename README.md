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
- **Community Logistics V1:** the leader's six action utilities allocate a sticky,
  simultaneous workforce instead of issuing one village-wide order. Gatherers
  deliver forest wood, expanders turn travelled owned cells into wood-built roads,
  and clans protect hauled food in an emergency reserve that releases during need.
- **Logistics Validation V1.1:** a live logistics enable/ablation switch creates a
  clean counterfactual, while food-delivery throughput, road member-steps, and
  measured movement-cost savings distinguish useful infrastructure from activity.
  With logistics disabled, existing roads remain visible but provide no movement
  benefit.
- **Guarded Retraining V1.2:** a marathon challenger must improve fixed-world
  quality and then pass a paired logistics-on/off promotion gate. Survival, food
  security, clan fairness, expert routing, transport value, and reserve use can no
  longer be traded away for a higher headline score.
- **Community Care V1:** lethal combat incapacitates clan members for a bounded
  rescue window. Nearby Gather/Defend workers abandon routine jobs, reach the
  casualty, physically carry them home, and restore them; untreated wounds bleed
  out with ordinary death, kill, loss, and loot accounting.
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
  growth/expansion, Community Logistics and Community Care ablations, terrain).
- **Training window:** start/stop evolution, edit the training config, watch the
  fitness graph, and seed the best brain into the live world.

## Tests

```powershell
cd life-rs
cargo test --release
cargo test --release ai_quality_benchmark_is_deterministic -- --nocapture
cargo test --release logistics_ablation_is_deterministic -- --nocapture
cargo test --release tracked_champion_logistics_preserves_survival_gates -- --nocapture
cargo test --release champion_promotion -- --nocapture
cargo test --release tracked_champion_care_preserves_survival_gates -- --nocapture
```

The V1.1 tests run paired logistics-on/off worlds with the same brain, seeds, and
world specifications. Ordinary training does not pay this doubled simulation
cost; the ablation is an explicit release-validation gate.

Marathon training pays that paired cost only when a fixed-world challenger first
beats the incumbent. Rejected candidates are logged with concrete reasons and
never overwrite `champion.bin`.

Community Care has a separate same-world treatment/control benchmark. The tracked
peaceful champion preserves **1.000** robust survival, **0.929** food security,
and **+0.009** clan fairness; it produced no clan-member wound opportunities in
the natural 13-world sample, so deterministic forced-combat tests provide the
causal rescue proof without claiming an unobserved natural-play gain.

Current tracked-champion result across 13 paired worlds: initial-clan survival
**1.000 / 1.000** (enabled/disabled), food security **0.928 / 0.935**, hauling
throughput **0.438 / 0.372**, road utility **0.290 / 0**, and enabled fairness
**+0.009**. The small security cost is bounded by a strict one-point tolerance.

`life-rs/champion.bin` is the tracked deployable model. Marathon logs, stage/gen
snapshots, backup champions, and `target/` are generated locally and git-ignored.

## Documentation

See the [`wiki/`](wiki/index.md): Architecture, Simulation & Gameplay, Module
Reference, Controls & UI, Parameters, and Roadmap.
