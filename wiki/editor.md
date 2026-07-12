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
- Terrain is the base layer; clan territory tints over it; bright cells are food;
  yellow cells are stockpiles; leaders are lightened.

## Controls panel (left)

- **Presets:** Gentle (default) · Balanced · Buffet · Famine — one click rebuilds
  the whole economy.
- **Simulation:** run/step, the speed slider.
- **Populate:** world size, NPC count, trees, starting clans, and a "maintain"
  population floor → **Populate fresh**.
- **World Parameters** (collapsible groups, all live): Food/trees, Hunger/health,
  Movement/perception, Clans/combat, **Farming/seasons**, Growth/expansion, and
  Terrain. **reset** restores defaults; your tuning survives Populate. See
  [Parameters](save-format.md).
- **View:** reset the camera; a legend of NPC/terrain colors.

## Inspector panel (right)

- The selected NPC's **idea** (current goal), health and hunger bars, carried
  food, speed, and position.
- If it belongs to a clan: the clan's **order (mode)**, members, stockpile food,
  territory, aggression, K/L/recruited, the **master → sub-mind routing** (which
  expert the leader is delegating to), and the **blended action utilities**.
- A live **clan list** (color, mode, people, food, K/L).

## Graphs panel (bottom)

Rolling plots of **population & leaders & clans**, **food on the map**, and
**starvation vs combat deaths**.

## Training window (floating)

- **Start / Stop** evolution (runs on a background thread across all CPU cores).
- **Stats:** generation, best/avg/best-ever fitness, last-gen time.
- **Config:** population, episode ticks, clans per arena, repeats, arena size,
  arena trees/neutrals, mutation rate & strength, elite count.
- **Seed best brain → live world:** inject the current champion as a new clan so
  you can watch the evolved leader play.
- A **fitness-over-generations** graph (best + average).
