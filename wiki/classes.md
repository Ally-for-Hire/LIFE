# Module Reference

The core Rust types and what they own. See the source under
[`life-rs/src/`](../life-rs/src/) for full detail.

## `World` (`world.rs`)

The single source of truth and the per-tick simulation.

| Member | Description |
| --- | --- |
| `grid: Grid` | terrain / fertility / owner / wood / traffic / road / pellet layers |
| `entities: Vec<Entity>` | every NPC |
| `trees: Vec<Tree>` | persistent food sources |
| `clans: Vec<Clan>` | active clans |
| `params: Params` | all live-tunable settings |
| `rng: Rng` | this world's deterministic PRNG |
| `deaths_starved` / `deaths_combat` / `births` | population counters |
| `maintain_pop` | optional population floor |
| `maintain_clans` | clan floor — re-form villages from refugees when war thins the field |
| `champion: Option<Brain>` | arena trainer's best brain; new villages may inherit it |
| `step()` | one tick (farms → trees → think → entities → recruit → combat → raid → births → prune) |
| `season_factor()` | seasonal yield multiplier (sine over `season_length`) |
| `populate(neutrals, trees, clans)` | generate terrain and seed the world |
| `setup_arena(brains, trees, neutrals)` | headless training arena setup |
| `seed_clan(brain)` | drop a champion into the live world |

Internally: `grow_farms` (per-tick farm growback on owned land), `breed_brain`
(in-vivo evolution for new villages), `form_refugee_clan`, `find_frontier`
(fertility-scored), and a per-clan `fertile_capacity` cap.

## `Params` (`world.rs`)

Every tunable variable, read live each tick: food/tree rates, **farms**
(`farm_yield`, `farm_interval`, `home_range`, `expand_claim_radius`), **seasons**
(`season_length`, `season_amp`), hunger/health, speed/vision, combat,
`claim_interval`, `members_per_claim`, reproduction (`birth_*`), and terrain
(`terrain_on`, `water_level`, `mountain_level`). See [Parameters](save-format.md).

## `Entity` + `Goal` (`entity.rs`)

One NPC: `id`, position, `speed`/`move_budget`, `health`, `is_leader`, carried
`food` and `wood`, hunger state, `last_food` memory, `attack_cooldown`, and `clan`
(id or -1). `work_role` is its sticky workforce assignment; `Goal` remains the
human-readable immediate intent shown in the inspector (including gathering or
hauling food/wood and building roads). Hunger and immediate defense may override
the assigned role without erasing it.

## `Clan` + `ClanMode` (`clan.rs`)

A leader + followers with a `Brain`. Holds the `stockpile`, stored `food`,
protected `reserve_food`, shared `wood`, deterministic `workforce` counts,
`territory` count, **`fertile_capacity`** (summed fertility of owned tiles → the
RDH population cap), `aggression`, current `mode`, cached targets (`enemy_pos`,
`recruit_target`, `neutral_pos`, `trespasser_pos`, `expand_target`),
`last_claim_tick`, and `stats` (kills / losses / recruits / peak / founded /
`on_terr_tick_sum`, role time, wood delivered, roads built, and reserve
deposits/releases). `ClanMode` is one of Gather / Recruit / Expand / Defend /
Attack / Scout; `mode` is now the headline order while members can simultaneously
hold different roles.

## `Brain` (`brain.rs`)

A **hierarchical mixture-of-experts** leader policy. A master *gate* network
(32 inputs → 10 hidden tanh → `N_EXPERTS` softmax) routes over several *sub-minds*,
each a small net (32 → 12 tanh → 7 sigmoid). `evaluate(inputs)` returns the
gate-weighted blend of the sub-minds' action vectors **and** the routing weights:
outputs 0..5 are clan-mode utilities, output 6 is the aggression dial. Nothing is
hardcoded — evolution specialises the sub-minds and learns the routing.
`mutate`/`crossover` operate over every expert and the gate; `last_out` and
`last_gate` are kept for the inspector (so you can see which sub-mind the master
is delegating to). This is the substrate for "master control AIs with sub-minds."

## `Trainer` + `TrainCfg` (`trainer.rs`)

Owns the population and evolution. `evaluate_parallel(pop, cfg, gen)` fans
independent arenas across all cores (rayon) and returns mean fitness per brain;
`finish_generation` records best/avg history and breeds the next generation
(elitism + tournament selection + crossover + mutation). `best_brain` is the
hall-of-fame champion.

## `Grid` (`grid.rs`)

The typed-array tile layers plus `idx(x, y) = y*size + x`, `in_bounds`, and
`clamp`. Terrain kinds live in the `terrain` submodule.

## `Rng` (`rng.rs`)

Deterministic, seedable xoshiro256** (seeded via SplitMix64). Each `World` and
each training arena owns one, so randomness is explicit and reproducible.
