# Parameters

Every world parameter is live-tunable from the **World Parameters** groups in the
Controls panel and read fresh each tick (`World::params`, type `Params`). Changes
take effect immediately — except terrain, which regenerates on the next *Populate
fresh* / preset. **reset** restores the defaults below; **presets** (Gentle /
Balanced / Buffet / Famine) override several at once.

## Food / trees

| Parameter | Default | Meaning |
| --- | --- | --- |
| `tree_interval` | 110 | ticks between a tree's pellet drops (wild food only, on unclaimed land) |
| `tree_per_cycle` | 6 | pellets dropped per cycle (× season) |
| `tree_radius` | 7 | drop spread radius |
| `pellet_energy` | 10 | energy stored per pellet |
| `max_pellet_fraction` | 0.09 | cap total pellets at this fraction of cells (shared by farms + wild) |

## Hunger / health

| Parameter | Default | Meaning |
| --- | --- | --- |
| `starve_ticks` | 1400 | ticks without food before health drains |
| `starve_damage` | 0.05 | health lost per tick while starving |
| `heal_rate` | 0.08 | health regained per tick while fed |
| `base_health` | 10 | villager health |
| `leader_health` | 24 | leader health |
| `hunger_min` / `hunger_max` | 0.16 / 0.42 | personal hunger-trigger range |

## Movement / perception

| Parameter | Default | Meaning |
| --- | --- | --- |
| `min_speed` / `max_speed` | 0.25 / 0.5 | cells per tick (per-NPC roll) |
| `vision_radius` | 15 | how far an NPC sees food / others |
| `leader_chance` | 0.02 | fraction of new NPCs born as leaders |

## Clans / combat

| Parameter | Default | Meaning |
| --- | --- | --- |
| `carry_limit` | 10 | carried food before a worker hauls to the stockpile |
| `attack_damage` | 0.45 | damage per hit |
| `attack_cooldown` | 20 | ticks between attacks |
| `clan_grace_ticks` | 1800 | opening peace period |
| `war_threshold` | 1.05 | war when two clans' combined aggression ≥ this |
| `recruit_radius` | 3 | distance at which a leader recruits a neutral |

## Farming / seasons

The heart of the territory economy — owned, fertile land grows food, and a slow
season cycle turns plenty into scarcity.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `farm_yield` | 0.16 | per owned fertile tile, pellet-spawn chance per pass (× fertility × season) |
| `farm_interval` | 16 | ticks between farm growth passes |
| `home_range` | 24 | how far working members roam from the stockpile |
| `expand_claim_radius` | 1 | radius of a single worker land claim while expanding |
| `claim_interval` | 14 | min ticks between a clan's territory claims |
| `members_per_claim` | 2 | population per unit of fertile capacity (sets the pop cap) |
| `season_length` | 3000 | ticks per full season cycle (0 = seasons off) |
| `season_amp` | 0.55 | yield swing amplitude; lean season yields ≈ `1-amp`× |

## Growth / expansion

| Parameter | Default | Meaning |
| --- | --- | --- |
| `birth_chance` | 0.025 | chance per pair of NPCs per reproduction check |
| `birth_interval` | 180 | ticks between reproduction checks |
| `birth_food_cost` | 4 | food a clan spends per birth |

## Terrain (applies on Populate)

| Parameter | Default | Meaning |
| --- | --- | --- |
| `terrain_on` | true | generate terrain (off = flat plains) |
| `water_level` | 0.32 | elevation below which tiles are water |
| `mountain_level` | 0.80 | elevation above which tiles are mountain |

## Training config (`TrainCfg`)

Edited live in the Training window.

| Field | Default | Meaning |
| --- | --- | --- |
| `pop_size` | 24 | brains in the population |
| `episode_ticks` | 4000 | ticks each arena runs |
| `clans_per_arena` | 6 | brains competing per arena |
| `repeats` | 2 | how many arenas each brain appears in per generation |
| `world_size` | 130 | arena grid size |
| `arena_trees` / `arena_neutrals` | 80 / 30 | arena food and free recruits |
| `mutation_rate` / `mutation_strength` | 0.08 / 0.3 | per-weight mutation |
| `elite` | 3 | top brains carried over unchanged |

> **Save/load** of a world or a champion brain to disk is not implemented yet —
> see the [Roadmap](roadmap.md). Determinism means a `(seed, params)` pair
> already reproduces a run exactly.
