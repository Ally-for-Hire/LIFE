# Controls & UI

Everything is in one native window. Panels toggle from the top bar, so you can
shape the layout like a game HUD.

## Top bar

- **Run / Pause**, **Step** — drive the live sim.
- **`ticks/s` slider (1–5000, logarithmic)** — the heart of the playground. At 1
  you can watch a single NPC decide; at 5000 it's whole-civilization dynamics.
- **Live stats:** tick, NPC count, clans, born, food on map, starved (and
  rate), killed, and measured ticks/s.
- **Toggles:** Controls · Inspector · Graphs · Training.

## Viewport

- **Drag** to pan, **scroll** to zoom.
- **Click an NPC** to select it (yellow highlight) and read it in the Inspector.
- Terrain is the base layer; clan territory tints over it; brown forest cells
  carry harvestable wood, tan cells are roads, bright green cells are food,
  yellow cells are stockpiles, and leaders are lightened. During the logistics
  ablation, retained roads turn gray because they provide no movement benefit.
  Houses are blue, granaries gold, workshops purple, markets teal, and walls gray;
  incomplete sites brighten as physical construction advances.

## Controls panel (left)

- **Presets:** Gentle (default) · Balanced · Buffet · Famine — one click rebuilds
  the whole economy.
- **Simulation:** run/step, the speed slider.
- **World file:** save/load `world.lifeworld`, see success/failure status, and
  explicitly control whether newly formed clans resume inheriting the trainer champion.
- **Populate:** world size, NPC count, trees, starting clans, and a "maintain"
  population floor → **Populate fresh**.
- **World Parameters** (collapsible groups, all live): Food/trees, Hunger/health,
  Movement/perception, Clans/combat, **Farming/seasons**, Growth/expansion, and
  Terrain. **Community logistics** exposes the live enable/ablation checkbox and
  states exactly which mechanics/no-road-benefit semantics are active. **Community
  care** independently toggles incapacitation/rescue versus immediate combat death.
  **Trade and diplomacy** independently toggles pacts, physical exchange, allied
  passage, and route defense versus the no-trade control. **Buildings and
  technology** independently toggles planning, physical work, research, signals,
  and all building effects. **reset**
  restores defaults; your tuning survives Populate. See
  [Parameters](save-format.md).
- **View:** reset the camera; a legend of NPC/terrain colors.

## Inspector panel (right)

- The selected NPC's **idea** (current goal), sticky **community role**, health
  and hunger bars, carried food/wood, speed, and position.
- If it belongs to a clan: the clan's **order (mode)**, live workforce mix,
  members, ordinary/reserve food, shared wood, public-work counters, territory,
  aggression, K/L/recruited, food delivered, real road member-steps, measured
  movement-cost savings, care incapacitations/rescues/bleed-outs, current trade
  partner/trust, physical cargo, delivered food/wood totals, active building
  counts, technology/research, current construction project and development
  counters, plus the
  **master → sub-mind routing** (which
  expert the leader is delegating to), and the **blended action utilities**.
- A live **clan list** (color, mode, people, food/reserve, wood, roads, K/L).

## Graphs panel (bottom)

Rolling plots of **population & leaders & clans**, **food on the map**, and
**starvation vs combat deaths**.

## Training window (floating)

- **Start / Stop** evolution (runs on a background thread across all CPU cores).
- **Stats:** generation, best/avg/best-ever fitness, robust survival, food and
  reserve security, community logistics, **hauling throughput**, **road utility**,
  task coverage, community care, delivered trade, settlement infrastructure and
  technology, routing/archive health, and last-generation time.
- **Config:** population, episode ticks, clans per arena, repeats, arena size,
  arena trees/neutrals, mutation rate & strength, elite count.
- **Seed best brain → live world:** inject the current champion as a new clan so
  you can watch the evolved leader play.
- A **fitness-over-generations** graph (best + average).
