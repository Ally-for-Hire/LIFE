# Devlog — making villages that settle, use land, and fight

A running log of attempts, ideas, and results while reworking LIFE so clans form
real villages that *use* their territory instead of roaming the map foraging.
Newest entries at the bottom of each section. Metrics come from the headless
diagnostics in [`life-rs/src/diag.rs`](../life-rs/src/diag.rs)
(`cargo test --release diag_gentle -- --nocapture`).

Key watched metrics:
- **homeDist** — mean distance of members from their stockpile (low = tight village).
- **onTerr%** — share of members standing on land their clan owns (high = using territory).
- **terr / owned_tiles** — territory claimed (was frozen before; should grow).
- **kill** — combat deaths (was ~0; we want real but non-extinction conflict).

## The problem (baseline)

Default 20k run, before changes: territory **frozen at 29 tiles/clan** the entire
run; **onTerr ≈ 2–8%**; ~180/200 NPCs "gathering food" scattered up to half the
map from home; **5 kills in 20k**. A peaceful forage-fest, no villages.

**Root cause:** territory had *zero economic value*. Food (pellets) grew from
trees scattered on any passable land regardless of ownership, so the optimal
behaviour genuinely *was* to roam and forage. The owner grid was decorative.

## Prior art consulted

A research pass (Sugarscape, central-place foraging / MVT / giving-up density,
ideal free & despotic distribution, economic defendability, resource dispersion
hypothesis, and colony/strategy games) converged on one diagnosis: **make
resource value local, depletable, and owner-internalised**, and settling /
using land / expanding / fighting become the optimal policy. See
[prior-art.md](prior-art.md) for the full comparison and rule set.

## Attempts & results

### A1 — Territory economy (farms + home-range + real expansion)
- **Farms:** owned + fertile tiles grow food each `farm_interval`
  (`grow_farms`). Cultivated land out-produces wilderness.
- **Home-range foraging:** working members harvest within `home_range` of the
  stockpile and cluster home when idle (central-place foraging).
- **Expansion:** `find_frontier` scores frontier tiles by fertility×proximity;
  workers claim toward the best land; claims rate-limited but productive.
- **Result:** territory 145→**259** (then much more), homeDist 20–130→**5–14**,
  onTerr 2–8%→**33–77%**, kills 5→25. Villages appeared.

### A2 — Close the food leaks (the real fix)
- **Despotic exclusion:** a clan's farm pellets feed only its members; outsiders
  can't harvest owned tiles unless starving (`EMERGENCY_STEAL`), and stealing
  makes them a hunted trespasser. (`consume_pellet_at` + harvestable search.)
- **Wild food gated & scarce:** trees drop pellets only on *unclaimed* land, and
  farms get first call on the food budget (`grow_farms` before `update_trees`).
- **Clumped fertility worldgen:** a low-frequency fertility field makes good
  farmland rare and patchy — contested valleys, not uniform plains.
- **RDH population cap:** cap scales with *fertile* territory
  (`fertile_capacity`), not raw tile count — rich land supports a big village,
  scrub stays small.
- **Result (40k):** owned tiles → **1397**, onTerr to **60–83%**, kills→37,
  but clans consolidated 5→2 (a slow drift to monoculture) and a large pool of
  masterless neutrals roamed (the old "running around eating" reappearing as a
  peasant underclass).

### A3 — Persist a multi-village world
- **Refugee-village genesis:** when war thins the field below `maintain_clans`,
  masterless refugees on good unclaimed land elect a leader and settle
  (`form_refugee_clan`). Defeated peoples re-coalesce into villages.
- **Recruitment fix:** any fed village with spare capacity recruits nearby
  refugees (not just tiny clans) — absorbs the wanderers.
- **Result (40k):** **all 5 villages persist**, onTerr 41–78%, owned 4357 —
  but conflict fell to **0 kills**: with refugee genesis keeping everyone alive
  and free land everywhere, my hardcoded gates never triggered war. Too peaceful.

### A4 — PIVOT: hand control to the neural nets (user direction)
Removed the hand-written decision gates (survival_stress / scarcity_conflict /
land_hungry / feasibility thresholds). Now the **leader brain picks the mode and
aggression directly**; the only gates left are *physical* (can't recruit with no
neutral, can't expand with no frontier, can't attack with no enemy). Per-entity
hunger foraging stays as a physiological safety net so a bold leader choice can't
instantly starve a clan.

Rebuilt the brain as a **hierarchical mixture-of-experts**: a master *gate*
network routes (softmax) over several *sub-minds*, each proposing a full action
vector; the decision is the gate-weighted blend. Nothing is hardcoded —
evolution specialises the sub-minds (survival / war / settler / …) and learns
when the master should delegate to each. Inputs enriched to 16 (added headroom,
frontier-exists, enemy-near, food-climate/famine sense). Outputs = 6 mode
utilities + a dedicated aggression dial.

- **Result (40k, *random* brains, no evolution yet):** 5 villages persist,
  onTerr **77–100%** for settlers, homeDist tight, **34 kills**, warlords
  (K17, K12) and a roaming raider clan emerged, all six modes in use,
  pop 96→200, owned→6050, mild starvation (6). Rich emergent behaviour from the
  environment alone — exactly the target. Evolution should now *optimise* it.

### A5 — In-vivo evolution (the live world trains itself)
Replacement clans (refugees, maintained slots) no longer get random brains:
`breed_brain` tournament-selects two *thriving* living clans (by population,
fertile capacity, territory, kills), crosses them over, and mutates — with a 15%
fresh-immigrant rate for diversity. Initial clans stay random (diversity at
seeding matters enormously — a bug where initial clans were near-clones collapsed
a whole run to a peaceful 254-tile equilibrium; fixed by seeding random).
- **Result:** strategies that survive and dominate now propagate to new villages.
  Turnover (and thus selection) is strong when the world is violent, weak when
  it's peaceful — so the arena trainer remains the main optimisation engine.

### A6 — Seasons (complexity for the sub-minds)
A slow global yield cycle (`season_factor`, sine over `season_length`, amplitude
`season_amp`) multiplies both farm and wild-food yield. Lean seasons throttle
food → scarcity → raiding; the season phase is a brain input (#15) so leaders can
anticipate winter. This is the first real "situation" for the master to route a
sub-mind at.
- **Result (40k, seed 0x1234):** kills **18 → 87**; homeDist and onTerr% now
  *oscillate with the seasons* (settle and farm in summer, send war-parties in
  winter), and clan turnover rose (brain_gen climbing). Boom/bust → war works.

### A7 — Arena trainer carries the MoE brain; fitness rewards villages
Fitness (`score_clan`) rebalanced to reward a **settled, fed, defended village**:
a new `on_terr_tick_sum` stat measures member-time spent on owned land, fed into
a big `settled_score`; territory reward made diminishing (`fertile_cap.sqrt()`)
so the optimum is "enough fertile land to feed the village," not grab-all; winning
land pays (`kills*1.2`), losses cost less. Arena economy mirrors the live design
(farms, sparse wild food, seasons, short grace, low war threshold).
- **Result:** training improves (avg fitness climbs steadily). First pass (heavy
  raw-territory reward) bred *monomaniacal land-grabbers* (expand×5, onTerr ~1%);
  after rebalancing, trained champions both **settle (onTerr 99–100%) and fight**
  (warlord K21), 26 kills, pop 189 — villages, not nomads.

### A8 — Champion transfer (offline evolution → live world, automatically)
The app auto-runs the background trainer and keeps `world.champion` synced with
its best brain; `breed_brain` seeds ~30% of new villages from the champion
(lightly mutated). So the live world continuously benefits from offline evolution
with no manual "seed" click — though the manual seed button remains.

## Status

The original to-do is met and then some: clans form tight villages, farm and use
their owned land, take in refugees, expand onto fertile frontier, and fight over
it — all driven by an evolving hierarchical (master + sub-minds) neural net with
no hand-coded strategy gates, under a seasonal boom/bust economy. All 7 tests
pass; runs are deterministic.

### A9 — Generalization: the "ultimate, transferable" champion
Goal shifted from "win one world" to "**be world-champion across a distribution of
worlds**" (so the strategies transfer to a game / real opponents). Three additions
to the trainer, all used by both the marathon and the GUI trainer:

- **Domain randomization** (`random_world_spec`): every arena draws a random world
  — size, food density, farm yield, season amplitude/length, war threshold, grace,
  starvation, terrain water/mountain levels, vision, tree density. A brain's fitness
  is its mean over many varied arenas, so it must be *generally* strong.
- **Plateau-driven curriculum** (`stage`, `finish_general`): when per-stage fitness
  stalls for ~20 generations, the `stage` rises — widening the randomization ranges,
  growing the **map border** (up to ~216 cells), harshening seasons, and adding more
  self-play opponents. Crucially each arena samples a stage in `0..=stage`, so easy
  regimes stay represented — the champion can't forget what it already mastered.
- **Hall-of-fame self-play** (`hof`): champions are frozen into a hall of fame and
  dropped into arenas as opponents, so the population must beat *diverse strong
  strategies*, not just its current peers — the path to real-opponent-level robustness.

The live world also benefits: the app keeps `world.champion` synced to the trainer's
best and seeds ~30% of new villages from it.

### A10 — 8-hour unattended marathon (durable, all-core)
A headless `train_marathon` (`#[ignore]`d test, `LIFE_TRAIN_HOURS` env) runs evolution
for N hours, **using every logical core** (24 here; sustained ~92% CPU), and:
- saves the rolling champion to `champion.bin` **every generation** (atomic temp +
  rename + `fsync`), so a crash/power-loss costs at most one generation;
- writes per-stage (`champion-stageN.bin`) and per-50-gen (`champion-genN.bin`)
  snapshots as deeper history;
- appends an fsync'd line to `training-log.txt` every generation.

The app auto-loads `champion.bin` on startup (continuing training from it and using it
live). Launch: `LIFE_TRAIN_HOURS=8 cargo test --release train_marathon -- --ignored --nocapture`.

### A11 — Champion-selection fix + reserved inputs + live supervision
The first marathon froze its champion: under domain randomization, "best single
generation ever" locked onto an early lucky draw (`best_ever` stuck, `champion.bin`
never improved). Fixed with **common random numbers** (every brain in a generation
is scored on the *same* shared worlds → fair, low-variance ranking; `avg` now
climbs) and a **king-of-the-hill benchmark champion** (champion + challenger scored
on a *fixed* world set; only a real winner replaces it). Also **reserved 15 future
inputs** (brain is now a fixed 32-dim: roads/buildings/tech/trade/2nd-resource/
day-night/soil/disaster/morale/water/…) so future world features change only the
*world*, never the brain — `champion.bin` keeps loading. The run was then babysat
on an hourly loop; on each plateau the world was made harder (below).

### A12 — Soil depletion + regional disasters (added live, on plateau)
Two new mechanics added mid-session when the curriculum maxed and the champion
plateaued, each wired to a reserved input and ramped in only at higher curriculum
stages (so easy-world skill is retained), each verified by a headless smoke:
- **Soil depletion** (input #26): harvesting exhausts a tile's yield; it recovers
  over time → clans must spread, rotate, and expand (expansion *rose* with it on).
- **Regional disasters** (input #27): periodic blight/drought discs wipe food and
  exhaust soil → clans must keep reserves and recover. Deterministic per world, so
  CRN keeps it fair.

### A13 — 8-hour session result
~1650 generations across the session; the curriculum climbed to stage 12 (the cap)
in each phase and the benchmark champion **improved as the world got harder**:
stage-12 champion **914 → 819 → 1021** across the broaden→soil→disaster phases —
i.e. it got *better at the hardest worlds even while we added difficulty*. The
final `champion.bin` (plus every `champion-stageN.bin`) is on disk and auto-loads
in the app.

**Showcase** (`diag_showcase`, all 5 clans run the final champion, live-default
world, 24k ticks): pop **321**, owned **8388** tiles, **onTerr 87–100 %**, tight
homes (13–30), huge food reserves — but only **1 kill**. The evolved "best
strategy" is **peaceful economic/territorial dominance**: settle the best land,
work it to ~100 %, expand relentlessly, out-grow rivals — *fighting isn't worth it*
under the current fitness (settle + feed + hold land; combat reward modest). It
nails "villages that settle and use territory"; it is a builder, not a warmonger.
To get a more militaristic champion, raise the combat/conquest weight in
`score_clan` and/or add a "take enemy land" fitness term, then re-run the marathon
(it will resume from the current champion).

### A14 — Survival-gated quality diversity and behavioral contract

The source is now preserved at `github.com/Ally-for-Hire/LIFE`; `champion.bin` is
the tracked deployable model, while build output and marathon snapshots/logs are
ignored. Training no longer treats survival as one tradeable fitness term:

- brains must clear robust-survival and food-security floors before entering the
  elite pool;
- eligible specialists are retained in survivor, builder, cooperator, defender,
  and raider archive slots and reintroduced into breeding;
- fixed deterministic worlds track original clan and neutral cohorts separately,
  plus settlement, security, expert coverage, and routing entropy;
- the training UI and marathon log surface survival, routing balance, and archive
  coverage.

The pre-existing tracked champion passes the initial 13-world, all-stage contract
at 4,000 ticks: robust survival **1.00**, clan and neutral cohort survival
**1.00 / 1.00**, food security **0.88**, routing entropy **0.18**, and expert
coverage **0.75**.
The thresholds intentionally freeze today's survival floor while future training
has room to improve specialization.

### A15 — Community Logistics V1

Clan decisions now turn the unchanged six brain utilities into deterministic,
sticky workforce assignments instead of one village-wide order. Every viable
workforce keeps gathering and defense cores while the leader remains the only
Recruit/Scout specialist; hunger and immediate border defense still override jobs.

Forest tiles now supply renewable wood. Gatherers haul it to the shared stockpile,
and active expand crews spend it on traffic-shaped roads across owned land; roads
halve movement cost. Food haulers fill ordinary working stores first, then protect
surplus in an emergency reserve that births and raiders cannot consume. The reserve
automatically feeds hungry members after ordinary food runs out. Reserved brain
inputs 16, 20, and 21 now expose road coverage, stored wood per member, and local
wood availability without changing `champion.bin` dimensions.

The cooperator niche now measures logistics throughput, reserve security, and task
coverage in addition to recruitment. These remain quality signals, not hard gates;
survival, food security, and clan-vs-neutral fairness still determine eligibility.
The fixed 13-world tracked-champion benchmark passed with robust survival **1.00**,
food security **0.93**, clan/neutral cohort survival **1.00 / 0.995**, logistics
**0.39**, reserve security **0.57**, and task coverage **0.63**. A 24k-tick showcase
ended with population **288**, five surviving clans, **zero starvation**, 23 kills,
and 1,274 roads built across the five communities.

### A16 — Logistics Validation V1.1

V1 showed that communities delivered wood, built roads, and used reserves, but
those activity counters could not prove that infrastructure improved transport or
survival. V1.1 adds `community_logistics` as a live deterministic treatment/
ablation switch. The disabled arm keeps sticky simultaneous roles and ordinary
food hauling, but disables reserve use, wood jobs, road construction, and every
movement/pathing benefit from existing roads. Retained roads render gray.

The two arms consume matching forest-regrowth random draws so later simulation
randomness stays aligned. New counters record food delivered, actual member-steps
on active roads, and movement cost saved; quality/training surface normalized
hauling throughput and road utility separately from the compatibility logistics
composite. The UI exposes the switch and causal counters directly.

The first 13-world, all-stage paired benchmark passes the survival-first gate.
Enabled/disabled initial-clan survival is **1.000 / 1.000**; food security is
**0.928 / 0.935** (a 0.7-point cost, inside the strict 1-point tolerance), while
clan-vs-neutral fairness improves to **+0.009 / +0.002**. Useful transport is now
measurable: hauling throughput is **0.438 / 0.372**, road utility **0.290 / 0**,
reserve use **0.262 / 0**, and reserve security **0.612 / 0**. Survival-first
tuning now withholds wood labor until both the ordinary working-food floor and
the emergency reserve are full. In-vivo selection uses food delivery and actual
road steps rather than rewarding raw road construction.

### A17 — Guarded Retraining V1.2

Marathon training previously promoted a challenger on the fixed-world headline
score alone. Promotion is now two-stage: only a headline winner pays for a
13-world paired Community Logistics benchmark, and only a candidate that also
preserves robust survival, food security, clan-vs-neutral fairness, routing
entropy, expert coverage, useful transport, reserve activation, and the
incumbent's causal logistics value may overwrite `champion.bin`. Every rejection
is appended to the training log with its failed contracts.

A 0.05-hour release-mode trial used all 24 logical cores and completed 33
generations. Two headline challengers reached the causal gate; both were rejected
for incumbent regressions (causal logistics value, plus expert coverage for the
first). The tracked champion's SHA-256 remained unchanged, demonstrating that the
new path retains the incumbent when no fully qualified winner emerges.

### A18 — Community Care V1

Combat wounds now become a 240-tick incapacitation window for clan members.
Nearby healthy Gather/Defend workers receive a true emergency override, reach the
patient, and carry it cell-by-cell to the stockpile before reviving it at 35%
health. A missed deadline preserves ordinary combat death, kill, loss, and delayed
loot accounting. Care can be disabled independently for immediate-death control
runs, and input 28 now reports average roster health.

The implementation treats incapacitation as inactive state throughout the sim:
patients cannot work, fight, raid, recruit, reproduce, build roads, occupy quotas,
or count as surviving benchmark cohort members. Rescue quality is completed
rescues per incapacitation, preventing injury farming. The release suite passes
35 tests with one ignored marathon. The natural 13-world tracked-champion sample
preserved 1.000 robust survival, 0.929 security, and +0.009 fairness but produced
no clan-member wound opportunities; deterministic forced-combat tests therefore
carry the causal evidence for assignment, transport, recovery, bleed-out, and loot.

### A19 — Trade/Diplomacy V1

Clans now keep deterministic symmetric relationship memory in a sorted ledger.
Gather couriers load no more than two food/one wood above an eight-food-per-member
and six-wood donor floor only when the partner is poorer, physically travel to the
partner stockpile, and build trust only after delivery. Nine delivered material
establishes a temporary pact. Pacts/trust suppress allied combat, raiding, trespass
targeting, and movement avoidance without permitting foreign farm harvest. Defend
workers cache and answer threats near active routes. Inputs 22–24 now expose
relation, partner count, and recent delivered volume while preserving `LFB1`.

The first integrated suite caught a reserved-input compatibility regression:
feeding 0.5 for "no partner" changed the tracked champion and broke the strict
logistics security gate. Restoring zero until a real relationship exists fixed the
old contract. Independent review also caught premature peace, starvation cargo
loss, incomplete ablation, passive route targeting, hollow promotion gates, and
missing trade-context routing probes; final re-review also led to narrow invited-
courier return passage and hostile-entity route targeting. Each issue now has a
focused regression test or promotion gate. The final 13-world trade pair preserves
1.000 survival and +0.010 fairness; enabled/disabled security is 0.932/0.936,
and enabled worlds deliver 6.3 food plus 3.8 wood in 5.8 completed trips versus zero in the control.
The release suite passes 52 tests with one ignored marathon.

### A20 — Full-world persistence V1

The app now saves and loads `world.lifeworld` from Controls. The versioned
`LIFEWRLD` envelope uses a bounded fixed-endian payload, CRC32, validation before
save/load, temporary-file flush, and atomic write-through replacement on Windows.
It preserves vector order, exact xoshiro state, brain routing state, terrain and
economy layers, care links, active outbound/returning couriers, diplomacy, cached
clan decisions, and the live champion; only flood-fill/occupancy scratch is rebuilt.

Loading is transactional and pauses the simulation. UI population/maintain knobs,
selection, texture, graphs, and meters are reset to the loaded checkpoint, while
background training remains independent. Trainer-to-world champion sync is disabled
after load so it cannot silently change future village inheritance. Eight focused
tests cover canonical roundtrip, 1,000-tick deterministic continuation, active care/
trade continuation, scratch exclusion, corruption/version rejection, stale-cache/
parameter validation, and Windows overwrite behavior. The integrated release suite
passes 60 tests with one ignored marathon.

### A21 — Buildings & Technology V1

Settlement progression now spends real harvested wood only after ordinary and
reserve food buffers are full. Expand workers walk to one-cell sites and perform
construction; Scout leaders walk to workshops and research a fixed three-level
technology progression. Houses, granaries, workshops, markets, and walls provide
population/healing, reserve, research, trade, and defense value while keeping the
32-input/7-output `LFB1` contract intact through reserved inputs 17–18.

The live settlement ablation disables planning, work, research, signals, and all
effects without deleting state. Quality scores observed infrastructure and useful
public-good counters only inside the survival gate. Marathon promotion requires
completed physical construction, positive causal value, paired survival/security/
fairness, and incumbent infrastructure non-regression. The tracked 13-world pair
preserves 1.000 survival, records 0.930/0.926 enabled/disabled security and +0.002
enabled fairness, and averages 60.9 work, 1.85 completed buildings, and +7.38 useful
value. Natural Scout research was zero, so a deterministic physical-workshop test
proves technology progression without claiming a natural benchmark gain.

`LIFEWRLD` V2 adds exact settlement state and explicit V1 migration. Eleven
persistence tests cover active construction/research, malformed settlement state,
and previous deterministic-continuation guarantees. The integrated release suite
passes 75 tests with one ignored marathon.

### A22 — Resource-backed Military Equipment V1

Military readiness now begins with deterministic finite deposits rather than a
combat toggle. A pre-tick planner selects one stable non-leader Gather miner and
one stable Expand smith only after ordinary and reserve food floors are full. Ore
is extracted on-cell, carried in a V1/V2-safe World-level record, delivered only
at the stockpile, reserved with safe wood, then converted by the same recipient
performing every production tick adjacent to a completed Workshop. Tech-0 spears
keep the tracked champion feasible; tech 1/2 unlock swords and armor.

Inputs 19/30 expose equipped strength and mineral access without changing `LFB1`.
The live ablation retains valid state but zeros signals/scoring and disables every
pipeline/combat effect. Weapons multiply base damage before walls; armor reduces
post-wall damage, with actual marginal damage recorded. Death/disband cleanup
removes cargo, loadouts, and projects. `LIFEWRLD` V3 wraps frozen V2 and explicitly
migrates V1/V2 by regenerating deterministic reachable deposits with empty
cargo/production/ownership state.

The tracked 13-world pair preserves 1.000 clan survival, 0.931/0.935 food security,
and +0.002 enabled fairness. Enabled worlds average 15.4 ore delivered, 46.8 forge
work, 2.9 completed items, 4,070 equipped-member ticks, and 38% same-clan full
pipeline completion; the disabled arm records zero. Promotion rejects unsafe work,
hollow pipelines, and incumbent supply/production/ownership loss. The integrated
release suite passes 96 tests with one ignored marathon.

## Open ideas / next

- **Next roadmap cycle:** all requested civilization milestones through Military
  Equipment V1 are complete; choose the next evidence-backed layer separately.
- **More sub-minds / deeper hierarchy:** specialist nets for diplomacy, logistics;
  a meta-gate over gates.
- **Live-world continuous evolution without violence:** cultural imitation so
  struggling clans adopt bred brains even in peace (keeps selection flowing).
- **Tuning:** seed 0x1234 is an unusually violent outlier (one K83 warlord,
  pop 226→85); most seeds settle at 5–37 kills with stable ~125 pop. Consider a
  mild cap on snowballing (Lanchester runaway) if monoculture becomes common.
