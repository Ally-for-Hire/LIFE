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
