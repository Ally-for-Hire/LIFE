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
  raider, and contextual-specialist niches, while fixed-world benchmarks guard
  clan-vs-neutral fairness, food security, and mixture-of-experts delegation.
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
- **Promotion-aware Retraining V1.3:** every eight generations screens the top
  twelve unique arena policies plus four retained near-passes on a cheap fixed-world
  proxy. A continuous gate deficit guides breeding; only a proxy passer reaches
  the full release suite. Absolute survival/fairness and causal care, logistics,
  trade, settlement, and military floors always apply. Incumbent-relative checks
  apply only when that incumbent passes the same current contract.
- **Safe MoE Specialization V1:** the trainer distinguishes genuine contextual
  delegation from both uniform expert mixing and single-expert collapse. Promotion
  and a sixth archive slot use utilization, decisiveness, context information,
  top-1 coverage, and expert-output divergence only after survival gates pass.
- **Seasonal Reality V1:** the yield wave is now four named behavioral phases.
  Spring restores soil and forests, summer is the high-yield expansion window,
  autumn diverts surplus into reserves, and winter stops wood regrowth, raises
  off-road movement and metabolic costs, and strongly restrains births. Roads
  bypass the winter travel surcharge, so preparation and infrastructure matter.
- **Community Care V1:** lethal combat incapacitates clan members for a bounded
  rescue window. Nearby Gather/Defend workers abandon routine jobs, reach the
  casualty, physically carry them home, and restore them; untreated wounds bleed
  out with ordinary death, kill, loss, and loot accounting.
- **Trade/Diplomacy V1:** deterministic symmetric relationship memory creates
  temporary trade pacts and allied passage. Gather couriers physically deliver
  only need-directed food/wood above survival floors, repeated delivery builds trust, and
  Defend workers respond to threats near active stockpile routes.
- **Full-world save/load:** the Controls panel writes `world.lifeworld` through a
  versioned, checksummed, atomic format and restores exact RNG, brains, economy,
  care, courier, and diplomacy state for byte-identical continuation.
- **Buildings/Technology V1:** food-secure clans reserve harvested wood for
  3x3 physical construction sites. Expand workers build houses, granaries,
  workshops, markets, and walls; Scout leaders research at completed workshops,
  unlocking stronger civic options without changing the fixed `LFB1` brain shape.
- **Military Equipment V1:** deterministic mineral deposits feed a physical
  ore-to-equipment chain. One food-secure Gather miner hauls ore; an Expand smith
  works beside a workshop and personally receives the spear, sword, or armor it
  finishes. The live ablation makes retained equipment completely inert.
- **Combat**, **food-gated reproduction**, **food memory**, and **up to three NPCs per
  tile**; defeated units leave stealable resource piles and wounded defenders can hide
  statically with 80% lower detection range.
- Game-like **toggleable panels**, an **NPC inspector** (its current "idea"),
  live **graphs**, **one-click presets**, and a slider for **every world
  parameter**.

## Controls

- **Top bar:** Run/Pause, Step, the `ticks/s` slider, live stats, the current
  season/yield/next transition, and panel toggles (Controls / Inspector / Graphs /
  Training).
- **Viewport:** drag to pan, scroll to zoom, click an NPC to inspect it.
- **Controls panel:** save/load a complete world, presets, populate counts, and every tunable world
  parameter (food/trees, hunger/health, movement/perception, clans/combat,
  growth/expansion, Community Logistics/Care/Trade, Buildings/Technology, and
  Military Equipment ablations, terrain).
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
cargo test --release tracked_champion_trade_preserves_survival_gates -- --nocapture
cargo test --release tracked_champion_settlement_preserves_survival_gates -- --nocapture
cargo test --release tracked_champion_military_completes_safe_physical_pipeline -- --nocapture
cargo test --release seasonal_reality_benchmark_is_deterministic -- --nocapture
cargo test --release world::persistence::tests -- --nocapture
```

The V1.1 tests run paired logistics-on/off worlds with the same brain, seeds, and
world specifications. Ordinary training does not pay this doubled simulation
cost; the ablation is an explicit release-validation gate.

Marathon training first screens top-K candidates with a cheap fixed proxy and pays
the full paired cost only for its best passer. Rejected candidates are logged with
continuous deficit plus concrete reasons and never overwrite `champion.bin`.
Use `LIFE_TRAIN_CHAMPION=promotion-diagnostic.bin` and
`LIFE_TRAIN_LOG=promotion-diagnostic.log` for a scratch run that cannot replace
the tracked champion.

Community Care has a separate same-world treatment/control benchmark. The tracked
peaceful champion preserves **1.000** robust survival, **0.933** food security,
and **+0.019** clan fairness; it produced no clan-member wound opportunities in
the natural 13-world sample, so deterministic forced-combat tests provide the
causal rescue proof without claiming an unobserved natural-play gain.

Trade's current 13-world pair preserves **1.000** robust survival and food security
**0.935 / 0.935** (enabled/disabled). The treatment delivers **6.1 food + 7.7 wood**
across **8.6 physical trips** per world while the disabled control delivers none.
The pre-change champion still fails the strict gate on **-0.25 worst-world fairness**;
positive delivery cannot override that safety contract.

Buildings/Technology's 13-world paired result preserves **1.000** robust clan
survival and food security **0.932 / 0.931** (enabled/disabled), with enabled
fairness **+0.003**. Clans perform **40.6 physical construction work**, complete
**1.15 buildings**, average **27.2 research ticks** and **0.10 normalized technology**,
and produce **+3.85 causal public-good value** per world. Completed workshops now
provide baseline research, while a physically present Scout leader contributes
additional research at 3x the baseline cadence.

Military Equipment's 13-world pair preserves **1.000** clan survival, food
security **0.936 / 0.936**, and enabled fairness **+0.007**. Enabled worlds deliver
**9.8 ore**, complete **1.3 items**, accumulate about **1,311 equipped-member
ticks**, and finish the full physical pipeline in **31%** of worlds; the disabled
arm performs no military work.

Current tracked-champion result across 13 paired worlds: initial-clan survival
**1.000 / 1.000** (enabled/disabled), food security **0.932 / 0.933**, hauling
throughput **0.732 / 0.619**, road utility **0.259 / 0**, and enabled fairness
**+0.028**. The small **-0.0013** security delta remains inside the strict 0.01
non-regression tolerance.

Seasonal Reality's separate 13-world, two-cycle harsh-climate contract preserves
**1.000** mean and robust winter clan survival with neutral parity. Food security
edges from **0.924** in summer to **0.922** in winter, while births fall from
**0.0086** of the summer cohort to **0.0020** in winter; winter task coverage remains
**0.619**. Summer stores grow **0.760 food/member**, with **2.77 construction work**
and **0.08 completed buildings** per world. This sample performs no winter reserve
release, so the release does not claim that outcome. Ordinary training does not pay
this extra benchmark cost.

`life-rs/champion.bin` is the tracked deployable model. Marathon logs, stage/gen
snapshots, backup champions, and `target/` are generated locally and git-ignored.
`world.lifeworld` is the separate full simulation snapshot. V2 adds settlement
state; V3 adds deposits, carried ore, forge projects, equipment ownership, military
counters, and the ablation. V4 adds persistent ground loot, exact 3x3 building
footprints, and the three-live-unit cell cap. V1-V3 migrate explicitly without
reinterpreting old bytes. Loading pauses the
world and detaches trainer-champion auto-sync until the user re-enables it.

## Documentation

See the [`wiki/`](wiki/index.md): Architecture, Simulation & Gameplay, Module
Reference, Controls & UI, Parameters, and Roadmap.
