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
| `season_length` | 3000 | ticks per full four-phase year/cycle (0 = seasons off) |
| `season_amp` | 0.55 | yield swing and harshness amplitude; lean yield ≈ `1-amp`× and winter pressure scales with amp |

The named phase is derived from `tick % season_length`; no new field is saved.
Seasonal Reality V1 therefore stays within `LIFEWRLD` V3. Existing snapshots
continue exactly under the current rules, including across a phase boundary.

## Growth / expansion

| Parameter | Default | Meaning |
| --- | --- | --- |
| `birth_chance` | 0.025 | chance per pair of NPCs per reproduction check |
| `birth_interval` | 180 | ticks between reproduction checks |
| `birth_food_cost` | 4 | food a clan spends per birth |

## Community Logistics V1 / Validation V1.1

V1's mechanics use fixed deterministic constants. V1.1 adds one live causal
validation switch:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `community_logistics` | true | enables wood jobs, protected reserves, road construction, and road movement savings; false is the paired infrastructure-disabled ablation |
| `community_care` | true | enables combat incapacitation, Gather/Defend rescue, physical evacuation, and recovery; false keeps immediate combat death |
| `community_trade` | true | enables relationship memory, temporary pacts, physical food/wood couriers, allied passage, and route defense; false is the no-trade control |

Disabling logistics does not erase existing road cells, which keeps world state
and rendering comparable. Those roads provide **no movement-cost benefit** while
the toggle is off, and the UI renders them gray. Delivered food goes entirely to
the ordinary stockpile; no reserve is filled/released, no wood job is selected,
and no road is built. Simultaneous sticky workforce assignments stay enabled so
the ablation isolates infrastructure rather than reverting the whole leadership
model. Wood regrowth consumes the same deterministic RNG draws in both arms, but
mutates wood only when enabled; this prevents avoidable regrowth-branch drift,
while later behavior-driven divergence remains part of the treatment effect.

The fixed V1 mechanics are:

| Rule | Value | Meaning |
| --- | --- | --- |
| workforce commitment | 240 ticks | normal quota balancing keeps assignments sticky for two leader decisions |
| forest wood capacity | 6 | maximum harvestable wood stored on one forest tile |
| wood regrowth | 8% every 360 ticks | deterministic world-RNG chance for a depleted forest tile to regain one wood |
| road cost | 2 wood | shared stockpile material spent per road cell |
| road work interval | 60 ticks | cadence for turning qualifying traffic cells into roads |
| road traffic floor | 3 | minimum recent movement pressure before a cell qualifies |
| ordinary food target | 4 per member | hauled food fills this working stockpile first |
| emergency reserve cap | 4 per member | surplus food protected for direct feeding after ordinary food runs out |
| wood-labor safety gate | ordinary floor + full reserve | gathering shifts to wood only after both food buffers are ready |

Brain inputs 16, 20, and 21 now expose road coverage, stored wood per member,
and reachable forest wood. The network dimensions remain fixed, so existing
champion files remain compatible.

Community Care uses a 240-tick rescue window, 12-cell assignment radius, and 35%
revival health. Input 28 now reports normalized roster health. These mechanics add
runtime entity/clan state but do not change the fixed brain dimensions or `LFB1`
champion format.

Trade V1 uses fixed safety/flow constants: 60-cell partner range, 720-tick pact,
nine delivered material before pact creation, 0.15 allied-trust threshold, eight
food per roster member plus six wood retained before need-directed donation, and
maximum courier loads of two food/one wood. Inputs 22–24 now
carry relation, partner count, and recent delivered volume; existing brain files
remain dimension-compatible.

Buildings/Technology uses a World-level `community_settlement` switch rather than
`Params`, because toggling it is a live causal treatment over retained state.
Project planning runs every 120 ticks only with at least four members, ordinary
food of four/member, full reserve of four/member, hunger below 0.85, no active
site, and enough wood for the cost plus a four-wood margin. Building costs are:
House 12 wood/24 work, Granary 18/36, Workshop 24/48, Market 30/60, and Wall
10/20. Research runs every 30 ticks at a physical workshop and costs 40/90/160
for levels 1/2/3. Inputs 17–18 carry normalized development and technology; the
fixed brain and champion formats do not change.

Military Equipment uses a World-level `community_military` switch. One stable
non-leader Gather miner carries at most eight ore; a stable Expand recipient must
perform every production tick adjacent to a completed Workshop. Both stages require
four members, ordinary food of four/member, full reserve of four/member, and a
personally non-hungry worker. Recipes are Spear (tech 0, 2 ore/4 wood/16 work),
Sword (tech 1, 5/2/24), and Armor (tech 2, 8/2/36). Spear/Sword add 25%/45%
outgoing base damage and Armor prevents 25% incoming damage. Input 19 is equipped
strength; input 30 is stored/reachable ore. The `LFB1` dimensions remain fixed.

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
| `pop_size` | 108 | brains in the population |
| `episode_ticks` | 7000 | ticks each arena runs |
| `clans_per_arena` | 6 | brains competing per arena |
| `repeats` | 4 | minimum repeat count (CPU fan-out may raise it) |
| `world_size` | 130 | arena grid size |
| `arena_trees` / `arena_neutrals` | 110 / 48 | arena food and free recruits |
| `mutation_rate` / `mutation_strength` | 0.10 / 0.35 | per-weight mutation |
| `elite` | 6 | top brains carried over unchanged, alongside niche elites |

## Persistence files

Champion brains remain independently serialized as `life-rs/champion.bin` in the
`LFB1` format. Stage/generation snapshots and marathon logs are generated artifacts.

The Controls panel saves the complete live simulation to `world.lifeworld`. The
`LIFEWRLD` envelope contains an explicit version, fixed payload length, CRC32,
and a fixed-integer little-endian payload capped at 512 MiB. Saving validates the
live state, writes a temporary file, flushes it, and atomically replaces the target
(with write-through replacement on Windows).

V1 preserves grid layers, entities/trees/clans in update order, every parameter and
counter, exact xoshiro256** state, brains including inspector routing state, care
links, courier/return state, diplomacy memory, cached clan decisions, and the live
champion. Only the reusable flood-fill and occupancy buffers are omitted; the next
tick rebuilds them. Loading validates checksums, sizes, floats, coordinates, IDs,
rosters, references, brain dimensions, RNG, and diplomacy before returning a new
world. The UI installs it transactionally, pauses it, clears stale view/history
state, and disables trainer-champion auto-sync until explicitly re-enabled.

V2 keeps the frozen V1 base DTO and adds buildings, the one-cell building lookup
layer, sorted clan settlement/technology state, causal counters, the live ablation,
and the next stable building id. Loading a V1 file explicitly initializes an empty
enabled settlement layer; it never reinterprets V1 bytes as the newer layout.
Validation rejects duplicate/missing building ids, inconsistent footprints,
out-of-bounds sites, invalid construction/HP, bad tech/research values, unsorted
settlement records, project references that are not live incomplete buildings
owned by the same clan, and orphaned incomplete sites without exactly one active
clan project.

V3 wraps the frozen V2 DTO and adds stable deposits, sorted entity ore cargo,
sorted physical loadouts, sorted clan miner/project/counter state, the military
ablation, and next deposit id. V1/V2 loads explicitly regenerate deterministic
reachable deposits and initialize empty enabled cargo/production/ownership state.
Validation rejects duplicate/out-of-order deposits, bad positions/
amounts/next ids, unsorted or orphaned cargo/loadouts, empty/invalid equipment,
negative clan ore, and miner/project recipients outside the owning live clan.
