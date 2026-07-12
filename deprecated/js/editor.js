// Editor: mouse-driven world editing — paint pellets/resources, place
// entities, erase, and inspect cells. Owns tool + brush state; main.js
// wires it to the sidebar UI.
class Editor {
  constructor(world, canvas) {
    this.world = world;
    this.canvas = canvas;
    this.tool = "pellet"; // pellet | entity | tree | wood | stone | eraser | inspect
    this.brushSize = 1;
    this.hover = null;      // {x, y} cell under the cursor, for the brush preview
    this.painting = false;
    this.erasing = false;   // true while right-button drag
    this.onInspect = null;  // callback(text) -> main.js shows it in the panel

    canvas.addEventListener("contextmenu", e => e.preventDefault());

    canvas.addEventListener("pointerdown", e => {
      e.preventDefault();
      canvas.setPointerCapture(e.pointerId);
      this.painting = true;
      this.erasing = e.button === 2;
      this.handle(e);
    });

    canvas.addEventListener("pointermove", e => {
      this.hover = this.cellFromEvent(e);
      if (this.painting) this.handle(e);
    });

    canvas.addEventListener("pointerup", () => {
      this.painting = false;
      this.erasing = false;
    });

    canvas.addEventListener("pointerout", () => {
      if (!this.painting) this.hover = null;
    });
  }

  cellFromEvent(e) {
    const rect = this.canvas.getBoundingClientRect();
    const x = Math.floor((e.clientX - rect.left) / rect.width * this.world.size);
    const y = Math.floor((e.clientY - rect.top) / rect.height * this.world.size);
    return { x: this.world.clamp(x), y: this.world.clamp(y) };
  }

  // Cells covered by the square brush centered on (cx, cy), clipped to the grid.
  brushCells(cx, cy) {
    const s = this.brushSize;
    const o = Math.floor((s - 1) / 2);
    const cells = [];
    for (let y = cy - o; y < cy - o + s; y++) {
      for (let x = cx - o; x < cx - o + s; x++) {
        if (this.world.inBounds(x, y)) cells.push({ x, y });
      }
    }
    return cells;
  }

  // Bounding box of the brush, for the hover preview.
  brushBox(cx, cy) {
    const s = this.brushSize;
    const o = Math.floor((s - 1) / 2);
    return { x: cx - o, y: cy - o, w: s, h: s };
  }

  handle(e) {
    const { x, y } = this.cellFromEvent(e);
    const tool = this.erasing ? "eraser" : this.tool;

    if (tool === "inspect") {
      this.inspect(x, y);
      return;
    }

    // Entities are placed one at a time regardless of brush size.
    if (tool === "entity") {
      if (!this.world.entityAt(x, y)) this.world.addEntity(new Entity(x, y));
      return;
    }

    for (const cell of this.brushCells(x, y)) {
      switch (tool) {
        case "pellet":
          this.world.addPellet(new Pellet(cell.x, cell.y));
          break;
        case "tree":
          if (!this.world.treeAt(cell.x, cell.y)) {
            this.world.addTree(new Tree(cell.x, cell.y, { lastSpawnTick: this.world.tick }));
          }
          break;
        case "wood":
        case "stone":
          if (!this.world.resourceAt(cell.x, cell.y)) {
            this.world.addResource(new Resource(cell.x, cell.y, tool));
          }
          break;
        case "eraser":
          this.world.clearCell(cell.x, cell.y);
          break;
      }
    }
  }

  inspect(x, y) {
    const lines = [`cell (${x}, ${y})`];

    const pellet = this.world.pelletAt(x, y);
    if (pellet) lines.push(`pellet: ${pellet.energy} energy`);

    const resource = this.world.resourceAt(x, y);
    if (resource) lines.push(`resource: ${resource.type} (${resource.amount} left)`);

    const tree = this.world.treeAt(x, y);
    if (tree) {
      lines.push(`tree #${tree.id}: hp ${tree.health.toFixed(1)}, drops ${tree.pelletsPerCycle} pellets/${tree.interval} ticks within r${tree.radius}`);
    }

    const town = this.world.clanWithStockpileAt(x, y);
    if (town) {
      const sp = town.stockpiles.find(s => s.x === x && s.y === y);
      const res = Object.entries(sp.resources)
        .map(([type, n]) => `${type} x${n}`)
        .join(", ") || "none";
      lines.push(`stockpile of clan #${town.id}: ${sp.food} food, resources: ${res} (min/person: ${town.minPerPerson})`);
    }

    const claimer = this.world.clans.find(c => c.claimed.has(x + "," + y));
    if (claimer) lines.push(`territory of clan #${claimer.id}`);

    const neutralClaimers = this.world.entities.filter(e => !e.clan && e.claimed.has(x + "," + y));
    for (const e of neutralClaimers) lines.push(`neutral claim of entity #${e.id}`);

    this.world.entitiesAt(x, y).forEach((e, i) => {
      const inv = Object.entries(e.inventory)
        .map(([type, n]) => `${type} x${n}`)
        .join(", ") || "empty";
      const role = e.isLeader ? "leader" : e.clan ? "follower" : "loner";
      const clanTag = e.clan ? ` of clan #${e.clan.id}` : "";
      lines.push(`entity #${i + 1}: ${role}${clanTag}`);
      lines.push(`  hp: ${e.health.toFixed(1)}/${e.maxHealth}  speed: ${e.speed.toFixed(2)} cells/tick`);
      lines.push(`  hunger: ${Math.round(e.hunger * 100)}% (seeks food at ${Math.round(e.hungerThreshold * 100)}%)`);
      lines.push(`  food: ${e.food}  task: ${e.task ? e.task.type : "none"}  inv: ${inv}`);
      lines.push(`  claims: ${e.claimed.size}  target: ${e.foodTargetKey || "none"}`);
      if (e.isLeader && e.clan && e.clan.brain) {
        const decision = e.clan.lastDecision;
        const action = decision ? `${decision.action} (${Math.round(decision.score * 100)}%)` : "none yet";
        lines.push(`  brain: ${e.clan.brain.name} gen ${e.clan.brain.generation}  last: ${action}`);
        const neutral = e.clan.relationshipWithNeutral();
        lines.push(`  vision: ${e.clan.vision.size} cells  relations neutral f:${neutral.friendliness.toFixed(2)} a:${neutral.animosity.toFixed(2)}`);
      }
    });

    if (lines.length === 1) lines.push("(empty)");
    if (this.onInspect) this.onInspect(lines.join("\n"));
  }
}
