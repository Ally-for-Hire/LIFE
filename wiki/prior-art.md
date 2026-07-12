# Prior art & design rationale

Why LIFE works the way it does, and how it compares to the bodies of work it
borrows from. The headline design problem was: *clans had no reason to use
territory, so they roamed and foraged.* The fix came straight from these models —
**make resource value local, depletable, and owner-internalised**, and settling /
using land / expanding / fighting become the optimal policy on their own.

## The models we borrowed from

**Sugarscape** (Epstein & Axtell, *Growing Artificial Societies*). Resource grows
back *per cell* up to a fixed capacity; agents climb the resource gradient,
collect, and pack onto the peaks until carrying capacity saturates. Combat reward
= local resource + capped loot, attack-down-only, with a no-retaliation filter.
→ **In LIFE:** the `fertility` layer is Sugarscape's capacity field; farms
(`grow_farms`) are per-cell, fertility-scaled growback on *owned* land; trees are
the fertility/peak bootstrap. Loot-on-kill and trespasser hunting are the combat
analogues.

**Central-place foraging / Marginal Value Theorem / Giving-Up Density** (Orians &
Pearson, Charnov, Brown 1988). A forager based at a home maximises *delivery rate*
(value ÷ round-trip distance), depletes a patch to the habitat-average rate, then
moves to the next near patch. → **In LIFE:** the stockpile is the central place;
`gather` keeps working members within `home_range` and clusters them home when
idle; carried food is hauled back. This is what turned "scatter and forage" into
"orbit the village."

**Ideal Free → Ideal Despotic Distribution** (Fretwell & Lucas). Free foragers
spread until every patch is equally good; the only way to keep a good patch *good*
is to **exclude** rivals — which is the despotic distribution and the engine of
conflict. → **In LIFE:** despotic exclusion — a clan's farm pellets feed only its
members (`consume_pellet_at` owner gate); outsiders can't harvest owned land
unless starving, and stealing makes them a hunted trespasser. This is the single
load-bearing fix that gave land exclusive value.

**Economic defendability** (Brown 1964) + **Resource Dispersion Hypothesis**
(Macdonald). Defend a resource only when its benefit exceeds the defence cost;
territoriality peaks at *intermediate, predictable, clumped* abundance. Territory
size tracks dispersion; group size tracks richness. → **In LIFE:** worldgen makes
fertile land rare and clumped (a low-frequency fertility field) so there are
valleys worth fighting for; the population cap scales with *fertile* capacity
(`fertile_capacity`), not raw area, so a rich valley feeds a real village and
scrub supports only a few.

**Colony/strategy games** (Settlers, Banished, Dwarf Fortress, RimWorld, Civ).
Food production is tied to claimed, worked tiles; borders expand toward yield;
storage and worker assignment to nearby tiles. → **In LIFE:** `find_frontier`
scores frontier tiles by fertility × proximity, so villages grow toward the best
farmland; claims are rate-limited and connectivity-pruned.

## What LIFE adds on top

- **Hierarchical mixture-of-experts leaders.** Each clan leader is a *master gate*
  routing (softmax) over several *sub-minds*, each proposing a full action vector;
  the decision is the gate-weighted blend. Evolution specialises the sub-minds and
  learns the routing — so one brain can handle famine, war, and growth without any
  of it being hardcoded. (No prior art dictates this; it's the substrate for
  "master control AIs with sub-minds.")
- **No strategy gates.** The net picks the mode and aggression directly; the only
  gates are physical (can't recruit with no neutral, expand with no frontier,
  attack with no enemy). Individual hunger foraging is a physiological safety net,
  not a strategy rule.
- **Two evolution paths.** An offline arena trainer (parallel, rayon) optimises
  brains against the same economy and a village-shaped fitness; the live world
  also evolves in-vivo (new villages inherit from thriving ones) and inherits the
  trainer's champion automatically.
- **Seasons.** A global yield cycle turns abundance into recurring scarcity — the
  boom/bust that makes peace and war alternate.

## Pitfalls watched (from the same literature)

- **Exclusion starvation cascade** — kept a wild-food bootstrap + an emergency
  steal override so clanless agents don't silently starve (the override also
  generates conflict).
- **Lanchester runaway** — a single clan can snowball to monoculture; the RDH cap
  and refugee-village genesis are the brakes. Still the main tuning risk.
- **Determinism** — every new random draw (breeding, worldgen fertility) goes
  through the world RNG in fixed order; the determinism test still passes.

See [devlog.md](devlog.md) for the attempt-by-attempt history and measurements.
