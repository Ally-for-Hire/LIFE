# Simulation & Gameplay

The rules every NPC and clan follows. All values are live-tunable — see
[Parameters](save-format.md). The throughline: **territory is the economy**, so
clans settle on good land, work it, and fight over it. See
[prior-art.md](prior-art.md) for why these rules produce villages.

## Entities

Each NPC has hunger (`ticks_since_food`), health, a personal speed, a food
**memory** (`last_food`), and an attack cooldown. Per tick:

1. **Hunger & health** — hunger rises; once starving, health drains; when fed and
   hurt, it heals.
2. **Eat if hungry** (past the personal threshold) — see *Foraging*.
3. Otherwise act on the clan's current goal, else wander.

**One NPC per tile:** movement is gated by an occupancy grid, so NPCs queue and
spread instead of stacking.

**Death model.** An NPC dies only from genuine scarcity or combat. Individual
hunger foraging always runs *before* the clan's collective goal, so a clan never
starves just because its leader chose a bold project — physiology is the safety
net, strategy is the brain's job.

## Farms — territory is the food supply

Every `farm_interval` ticks, each **owned, fertile, passable** tile has a chance
(`farm_yield × fertility/255 × season_factor`) to grow a pellet. This is the
economic engine: claimed fertile land feeds the village, so owning and working
land beats roaming. Farms get first call on the global food budget
(`max_pellet_fraction`); **wild trees** drop pellets only on *unclaimed* land and
are a sparse bootstrap, not a free top-up.

**Despotic exclusion.** A clan's farm pellets feed **only its own members**.
Outsiders (rival clans, neutrals) can't harvest owned tiles unless near
starvation (`EMERGENCY_STEAL`), and stealing makes them a hunted trespasser. This
is what gives land exclusive value and turns borders into something worth
defending.

## Foraging

- **Working members** (`gather`) harvest the nearest pellet on **their own or
  unclaimed** land, within `home_range` of the stockpile, hauling full loads home
  and clustering near home when there's nothing to pick — central-place foraging,
  so villages stay compact.
- **Hungry members / neutrals** range wider for survival, still preferring
  own/unclaimed food; only the near-starving cross into foreign land to steal.

## Clans — evolving hierarchical leaders

A clan is a leader plus followers sharing a color and a `Brain`. Clans start with
3 followers. Recruiting is deliberate (no auto-recruit): a leader chooses Recruit
and walks to a neutral.

The leader's brain is a **mixture-of-experts**: a master *gate* network reads the
clan's situation and routes (softmax) over several *sub-minds*, each proposing a
full action vector; the decision is the gate-weighted blend. The six action
utilities allocate the clan's workforce while the strongest feasible utility
remains its headline **mode**; the brain also sets an **aggression** dial. There
are no strategy gates, only
physical feasibility (you can't recruit with no neutral, expand with no frontier,
or attack with no enemy). Evolution shapes both the sub-minds and the routing.

| Mode | Workers do… |
| --- | --- |
| Gather | work the village's land (home-range harvest + haul) |
| Recruit | leader walks to the nearest neutral and recruits on contact |
| Expand | workers walk to the fertile frontier and claim (rate-limited) |
| Defend | hold near the stockpile |
| Attack | a war-party (healthy members) marches on the enemy; the rest keep working |
| Scout | leader explores |

## Community Logistics V1

Clans now work on several complementary jobs at once. Every leader decision turns
the six feasible utilities into deterministic member quotas. Assignments are
sticky long enough to prevent workers from oscillating between jobs, and small or
stressed clans keep a gathering safety core. Personal hunger and an immediate
trespasser threat still override any assignment, preserving the original
survival-first contract.

- **Wood:** forest tiles hold a separate, renewable material supply. Gathering
  workers fetch wood when the clan needs construction material and haul it to the
  shared stockpile; food remains their first survival responsibility.
- **Roads:** movement leaves a traffic trace. Expanders spend shared wood to pave
  useful, owned, passable cells, and completed roads halve movement cost for
  hauling, defense, and reinforcement.
- **Emergency reserve:** surplus hauled food is protected separately from the
  ordinary stockpile. The reserve is unavailable to routine births and raiding,
  then releases automatically when ordinary food runs lean or a disaster strikes.
- **Brain compatibility:** inputs 16, 20, and 21 now report road coverage, stored
  wood per member, and local wood availability. The network dimensions and saved
  `champion.bin` format do not change.

### Logistics Validation V1.1

V1 proves that logistics activity occurs; V1.1 measures whether it is useful.
`Params::community_logistics` is a live treatment/ablation switch. Turning it off
disables protected reserve deposits/releases, wood jobs, road construction, and
road movement-cost reductions. Delivered food goes to the ordinary stockpile.
Existing road cells remain visible in gray so the map can be compared without
hiding prior infrastructure. Sticky simultaneous assignments remain active, so
the comparison isolates infrastructure rather than changing the leader policy.
Wood regrowth consumes matching random draws in both arms but changes the wood
layer only when enabled, removing avoidable regrowth-branch RNG drift; later
behavioral divergence is part of the treatment effect.

Wood labor is survival-gated: Gather workers do not leave food work until the
ordinary stockpile floor and protected reserve are both full.

Each clan now records causal evidence alongside activity:

- `food_delivered` measures actual hauling throughput rather than stored-food
  snapshots;
- `road_steps` measures member movement that truly used roads;
- `road_cost_saved_milli` accumulates the movement cost avoided by those steps.

Training reports normalized hauling throughput and road utility separately from
the compatibility `logistics` composite. Paired deterministic benchmarks can
therefore compare the same brain, seeds, and worlds with logistics enabled and
disabled instead of treating road construction count as proof of benefit.

## Community Care V1

With `Params::community_care` enabled, lethal combat incapacitates a clan member
for 240 ticks instead of deleting it immediately. The casualty contributes zero
active workforce and cannot move, fight, raid, recruit, reproduce, build, or count
as a surviving benchmark cohort member. Input 28 reports mean normalized health
across the living roster, with incapacitated members contributing zero.

Healthy Gather and Defend workers within 12 cells receive a deterministic rescue
override. Hunger remains the rescuer's personal priority; otherwise it reaches the
patient, carries the patient one cell behind it to the stockpile, and restores 35%
health. A missed deadline becomes a normal combat death and retains delayed kill,
loss, and loot credit. The disabled arm keeps immediate-death behavior for causal
comparison. Care quality is completed rescues divided by incapacitations, so
creating more injuries cannot improve the score by itself.

## Trade & Diplomacy V1

Every 120 ticks, clans deterministically select the nearest non-hostile settlement
within 60 cells, breaking equal-distance ties by clan id. Gather workers at home
load at most two food and one wood into dedicated cargo only when the donor is
richer than the recipient and remains above eight food per roster member plus six
wood. Couriers walk to the partner stockpile and transfer ownership only on
arrival, then physically return home. Only the active invited courier receives
pre-alliance passage; it does not create global peace. Offers, loading, empty
trips, and reciprocal shuffling do not count as useful trade evidence.

Relationship memory is symmetric and stored in stable pair order. Completed
delivery increases trust; after at least nine delivered material, repeated aid
creates a temporary pact. Trust and recent volume decay, hostile attacks/raids
decrease trust, and dead-clan relationships are pruned. An active pact or trust
of at least 0.15 grants non-aggression and allied passage, but despotic harvest
exclusion still prevents allies from taking each other's farm food. Defend workers
cache a hostile entity id near the stockpile route and revalidate that exact
attacker before engaging, so stale coordinates cannot endanger passers. Inputs 22–24 now
report partner relation, partner count, and recent delivered volume without
changing the `LFB1` brain dimensions.

## Buildings & Technology V1

Every 120 ticks, an eligible clan may reserve wood for one physical construction
site only after reaching four members, filling ordinary food plus emergency reserve
floors, and retaining a wood margin. Stable site selection prefers nearby owned,
passable, unoccupied cells and breaks ties by distance then grid index. The project
spends its wood up front; Expand workers must walk adjacent and contribute work.

- **House (12 wood / 24 work):** adds two population capacity and heals nearby
  clan members by 0.02 health/tick within six cells.
- **Granary (18 / 36):** adds six protected reserve-food capacity.
- **Workshop (24 / 48):** gives the clan leader a physical research workplace.
- **Wall (10 / 20):** unlocked at technology level 1; reduces nearby incoming
  damage by 25% within four cells.
- **Market (30 / 60):** unlocked at level 2; increases physical courier loads.

The build sequence establishes granary, workshop, housing, threat-driven walls,
and trade-driven markets before repeating capacity buildings. A Scout-mode leader
walks adjacent to a completed workshop and contributes one research point every 30
ticks; levels cost 40, 90, and 160 points and cap at 3. Level 2 also doubles
physical construction work. Inputs 17 and 18 report
normalized development and technology without changing the 32-input `LFB1` format.
The live ablation zeros those signals and disables all mechanics/effects while
preserving structural state for paired comparison. On a mid-world toggle, protected
food above the ordinary four-per-member reserve cap is discarded immediately so a
disabled granary cannot keep granting survival value.

The tracked 13-world treatment/control pair preserves 1.000 clan survival,
records 0.930/0.926 food security and +0.002 enabled fairness, and averages 60.9
work, 1.85 completed buildings, and +7.38 causal public-good value. It observed no
natural Scout research, so a focused deterministic physical-workshop test proves
research progression without inflating the natural benchmark claim.

If a leader dies a follower is promoted; a clan disbands only when no members
remain (its territory is then freed). To keep the world a living patchwork,
`maintain_clans` re-forms villages from masterless **refugees** when war thins the
field (`form_refugee_clan`).

## Territory (owner grid)

- **Connected only:** the founding claim seeds a contiguous blob; later claims
  must touch owned land, so territory can't be disconnected. Passable land only.
- **Frontier-driven:** `find_frontier` scores frontier tiles by **fertility ×
  proximity**, so villages grow toward the best farmland.
- **Cut-off = useless:** every 200 ticks a flood-fill from each stockpile frees
  any owned tile no longer reachable through owned land.
- **Population cap = fertile capacity** (Resource Dispersion Hypothesis): a clan's
  cap scales with the summed fertility of its owned tiles, not raw area — a fertile
  valley supports a real village; scrub supports only a few, pressuring the clan
  to expand or fight toward better land.

## Combat

A clan member attacks an adjacent target when:

- the target stands on the member's **own territory** (a trespasser, enemy or
  neutral) — always; or
- the member's clan is **on campaign** (Attack mode) and the target is an enemy
  clan member — wherever they meet; or
- the two clans are **at war**: past the grace period with combined aggression
  ≥ `war_threshold`.

Attacks deal `attack_damage`, need adjacency, respect a cooldown, and a kill loots
carried food. For `clan_grace_ticks` at the start there is a peace period.

## Population growth (reproduction)

Every `birth_interval` ticks, fed clans with food reserves and capacity produce
children near the stockpile; neutrals breed only on a clear map-food surplus.
Self-regulating: too many mouths → scarcity → die-off.

## Seasons

A slow global cycle (`season_length`, `season_amp`) multiplies farm and wild-food
yield via `season_factor` (a sine in `[1-amp, 1+amp]`). Lean seasons throttle
food → scarcity → raiding; plentiful seasons → growth and expansion. The season
phase is a brain input, so leaders can learn to stockpile for winter or strike
when a rival's harvest fails.

## Terrain

Procedurally generated each Populate: water, sand, plains, forest, hills,
mountains, plus a **clumped fertility field** (a low-frequency noise so good
farmland is rare and patchy — the valleys worth fighting for). Water is
impassable; mountains/hills/forest cost more to cross; community-built roads halve
cost. Forest terrain also supplies the wood used for those roads. Pellets grow on
passable land only.

## Evolution

Two paths, both real:

- **Arena trainer** (offline, all CPU cores): evaluates populations of leader
  brains in headless arenas under the same economy, scored by a *village-shaped*
  fitness (settled-on-own-land time, fed population, held productive land, won
  conflicts), and breeds the next generation.
- **In-vivo** (live world): new villages inherit + mutate brains from currently
  thriving clans, and ~30% inherit the arena champion automatically — so offline
  and online evolution flow into the same world.
