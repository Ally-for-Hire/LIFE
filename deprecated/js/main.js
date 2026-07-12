// Setup, render loop, and UI wiring.
const COLORS = {
  background: "#1a1a1a",
  pellet: "#3fbf3f",
  tree: "#2f9f4f",
  entity: "#e8e8e8",
  stockpile: "#ffd700",
  brush: "rgba(255, 255, 255, 0.6)",
};

const SAVE_KEY = "life-world";

const canvas = document.getElementById("world");
const ctx = canvas.getContext("2d");
const hud = document.getElementById("hud");

// The canvas is 256x256 (1 pixel per cell), scaled up with CSS.
canvas.style.width = "768px";
canvas.style.height = "768px";

const world = new World(256);
const editor = new Editor(world, canvas);
editor.onInspect = text => { document.getElementById("inspector").textContent = text; };

const trainer = new TrainingArena();
const trainingPreview = document.getElementById("training-preview");
const trainingPreviewCtx = trainingPreview.getContext("2d");
const fitnessChart = document.getElementById("fitness-chart");
const fitnessCtx = fitnessChart.getContext("2d");

let paused = false;
let ticksPerFrame = 1;
let trainingActive = false;
let selectedBrainId = null;

// --- populate / clear ---
function populate() {
  const trees = Number(document.getElementById("pop-pellets").value) || 0;
  const entities = Number(document.getElementById("pop-entities").value) || 0;
  const leaders = Number(document.getElementById("pop-leaders").value) || 0;
  const resources = Number(document.getElementById("pop-resources").value) || 0;
  const woodPct = Number(document.getElementById("wood-pct").value) / 100;

  for (let i = 0; i < resources; i++) {
    const { x, y } = world.randomCell();
    const type = Math.random() < woodPct ? "wood" : "stone";
    world.addResource(new Resource(x, y, type));
  }

  // Spawn leaders with a tiny seed group plus visible nearby neutral candidates
  // so early clans can found towns and still train active recruitment.
  const spawned = [];
  const leaderCount = Math.min(leaders, entities);
  let remainingEntities = Math.max(0, entities - leaderCount);
  let remainingTrees = trees;

  for (let i = 0; i < leaderCount; i++) {
    const { x, y } = world.randomCell();
    const leader = new Entity(x, y, true);
    world.addEntity(leader);
    const clan = leader.clan;

    const localTrees = Math.min(2, remainingTrees);
    for (let n = 0; n < localTrees; n++) {
      const pos = randomNear(world, x, y, INITIAL_CLAIM_RADIUS);
      world.addTree(new Tree(pos.x, pos.y, { lastSpawnTick: -Math.floor(Math.random() * TREE_DEFAULT_INTERVAL) }));
      remainingTrees--;
    }

    const startingFollowers = Math.min(2, remainingEntities);
    for (let n = 0; n < startingFollowers; n++) {
      const pos = randomNear(world, x, y, 2);
      const follower = new Entity(pos.x, pos.y, false);
      world.addEntity(follower);
      clan.addMember(follower, "seed");
      remainingEntities--;
    }

    const localCandidates = Math.min(4, remainingEntities);
    for (let n = 0; n < localCandidates; n++) {
      let pos = randomNear(world, x, y, 8);
      if (Math.max(Math.abs(pos.x - x), Math.abs(pos.y - y)) <= CLAN_RECRUIT_RADIUS) {
        pos = {
          x: world.clamp(x + (pos.x >= x ? CLAN_RECRUIT_RADIUS + 2 : -CLAN_RECRUIT_RADIUS - 2)),
          y: world.clamp(y + (pos.y >= y ? CLAN_RECRUIT_RADIUS + 2 : -CLAN_RECRUIT_RADIUS - 2)),
        };
      }
      spawned.push(new Entity(pos.x, pos.y, false));
      remainingEntities--;
    }
  }

  for (let i = 0; i < remainingEntities; i++) {
    const { x, y } = world.randomCell();
    spawned.push(new Entity(x, y, false));
  }
  for (let i = 0; i < remainingTrees; i++) {
    const { x, y } = world.randomCell();
    world.addTree(new Tree(x, y, { lastSpawnTick: -Math.floor(Math.random() * TREE_DEFAULT_INTERVAL) }));
  }
  for (const e of spawned) world.addEntity(e);
}

// --- rendering ---
function drawWorld(targetCtx, targetWorld, options = {}) {
  const scaleX = targetCtx.canvas.width / targetWorld.size;
  const scaleY = targetCtx.canvas.height / targetWorld.size;
  targetCtx.save();
  targetCtx.setTransform(scaleX, 0, 0, scaleY, 0, 0);
  targetCtx.fillStyle = COLORS.background;
  targetCtx.fillRect(0, 0, targetWorld.size, targetWorld.size);

  // remembered clan vision as a very faint wash
  targetCtx.globalAlpha = 0.05;
  for (const clan of targetWorld.clans) {
    targetCtx.fillStyle = clan.color;
    for (const key of clan.vision.keys()) {
      if (clan.claimed.has(key)) continue;
      const [cx, cy] = key.split(",").map(Number);
      targetCtx.fillRect(cx, cy, 1, 1);
    }
  }
  targetCtx.globalAlpha = 1;

  // neutral per-entity claims
  targetCtx.globalAlpha = 0.12;
  targetCtx.fillStyle = "#b0b0b0";
  for (const entity of targetWorld.entities) {
    if (entity.clan || !entity.claimed.size) continue;
    for (const key of entity.claimed) {
      const [cx, cy] = key.split(",").map(Number);
      targetCtx.fillRect(cx, cy, 1, 1);
    }
  }
  targetCtx.globalAlpha = 1;

  // claimed town territory as a faint tint
  targetCtx.globalAlpha = 0.15;
  for (const clan of targetWorld.clans) {
    if (!clan.claimed.size) continue;
    targetCtx.fillStyle = clan.color;
    for (const key of clan.claimed) {
      const [cx, cy] = key.split(",").map(Number);
      targetCtx.fillRect(cx, cy, 1, 1);
    }
  }
  targetCtx.globalAlpha = 1;

  targetCtx.fillStyle = COLORS.pellet;
  for (const pellet of targetWorld.pellets.values()) {
    targetCtx.fillRect(pellet.x, pellet.y, 1, 1);
  }
  for (const tree of targetWorld.trees) {
    targetCtx.fillStyle = COLORS.tree;
    targetCtx.fillRect(tree.x - 1, tree.y - 1, 3, 3);
    targetCtx.fillStyle = "#74c66d";
    targetCtx.fillRect(tree.x, tree.y, 1, 1);
  }
  for (const resource of targetWorld.resources) {
    targetCtx.fillStyle = RESOURCE_TYPES[resource.type] ? RESOURCE_TYPES[resource.type].color : "#fff";
    targetCtx.fillRect(resource.x, resource.y, 1, 1);
  }
  targetCtx.lineWidth = 0.5;
  for (const clan of targetWorld.clans) {
    for (const sp of clan.stockpiles) {
      targetCtx.fillStyle = COLORS.stockpile;
      targetCtx.fillRect(sp.x, sp.y, 1, 1);
      targetCtx.strokeStyle = clan.color;
      targetCtx.strokeRect(sp.x - 1, sp.y - 1, 3, 3);
    }
  }

  for (const entity of targetWorld.entities) {
    targetCtx.fillStyle = entity.clan ? entity.clan.color : COLORS.entity;
    targetCtx.fillRect(entity.x, entity.y, 1, 1);
    if (entity.isLeader && entity.clan) {
      targetCtx.strokeStyle = entity.clan.color;
      targetCtx.strokeRect(entity.x - 1, entity.y - 1, 3, 3); // halo marks the leader
    }
  }

  if (options.showBrush && editor.hover) {
    const b = editor.brushBox(editor.hover.x, editor.hover.y);
    targetCtx.strokeStyle = COLORS.brush;
    targetCtx.lineWidth = 0.5;
    targetCtx.strokeRect(b.x, b.y, b.w, b.h);
  }
  targetCtx.restore();
}

function render() {
  drawWorld(ctx, world, { showBrush: true });

  const towns = world.clans.filter(c => c.isTown).length;
  hud.textContent =
    `tick: ${world.tick}  entities: ${world.entities.length}  ` +
    `grace: ${world.clanGraceRemaining()}  ` +
    `pellets: ${world.pellets.size}  trees: ${world.trees.length}  resources: ${world.resources.length}  ` +
    `clans: ${world.clans.length}  towns: ${towns}` +
    (paused ? "  [PAUSED]" : "");

  renderTrainingPreview();
  renderFitnessChart();
  updateTrainingUI();
}

function loop() {
  if (trainingActive) trainer.advance(trainer.ticksPerFrame);
  if (!paused) {
    for (let i = 0; i < ticksPerFrame; i++) world.step();
  }
  render();
  requestAnimationFrame(loop);
}

// --- UI wiring ---
function status(msg) {
  const el = document.getElementById("status");
  el.textContent = msg;
  clearTimeout(status.timer);
  status.timer = setTimeout(() => { el.textContent = ""; }, 2500);
}

function applyTrainingSettings() {
  const populationSize = Math.max(2, Math.min(32, Number(document.getElementById("train-pop").value) || TRAINING_DEFAULTS.populationSize));
  const episodeTicks = Math.max(500, Number(document.getElementById("train-episode").value) || TRAINING_DEFAULTS.episodeTicks);
  const trainingTicks = Math.max(50, Number(document.getElementById("train-speed").value) || TRAINING_DEFAULTS.ticksPerFrame);

  document.getElementById("train-pop").value = populationSize;
  document.getElementById("train-episode").value = episodeTicks;
  document.getElementById("train-speed").value = trainingTicks;
  document.getElementById("train-speed-label").textContent = trainingTicks;

  trainer.configure({
    populationSize,
    episodeTicks,
    ticksPerFrame: trainingTicks,
  });
}

function renderTrainingPreview() {
  if (trainer.world) {
    drawWorld(trainingPreviewCtx, trainer.world);
    return;
  }

  trainingPreviewCtx.setTransform(1, 0, 0, 1, 0, 0);
  trainingPreviewCtx.fillStyle = "#101010";
  trainingPreviewCtx.fillRect(0, 0, trainingPreview.width, trainingPreview.height);
  trainingPreviewCtx.fillStyle = "#555";
  trainingPreviewCtx.font = "10px monospace";
  trainingPreviewCtx.fillText("arena idle", 34, 66);
}

function renderFitnessChart() {
  const w = fitnessChart.width;
  const h = fitnessChart.height;
  const history = trainer.history;
  fitnessCtx.setTransform(1, 0, 0, 1, 0, 0);
  fitnessCtx.fillStyle = "#101010";
  fitnessCtx.fillRect(0, 0, w, h);
  fitnessCtx.strokeStyle = "#292929";
  fitnessCtx.beginPath();
  fitnessCtx.moveTo(0, h - 14);
  fitnessCtx.lineTo(w, h - 14);
  fitnessCtx.stroke();

  if (history.length < 2) {
    fitnessCtx.fillStyle = "#555";
    fitnessCtx.font = "10px monospace";
    fitnessCtx.fillText("fitness history", 8, 18);
    return;
  }

  const maxScore = Math.max(1, ...history.flatMap(p => [p.best, p.avg]));
  const drawLine = (key, color) => {
    fitnessCtx.strokeStyle = color;
    fitnessCtx.beginPath();
    history.forEach((p, i) => {
      const x = history.length === 1 ? 0 : i / (history.length - 1) * (w - 8) + 4;
      const y = h - 16 - (p[key] / maxScore) * (h - 24);
      if (i === 0) fitnessCtx.moveTo(x, y);
      else fitnessCtx.lineTo(x, y);
    });
    fitnessCtx.stroke();
  };
  drawLine("avg", "#777");
  drawLine("best", "#3fbf3f");
}

function valueBar(v) {
  const filled = Math.round(brainClamp01(v) * 10);
  return "[" + "#".repeat(filled) + "-".repeat(10 - filled) + "] " + Math.round(v * 100) + "%";
}

function renderBrainInspector(record) {
  const el = document.getElementById("brain-inspector");
  if (!record) {
    el.textContent = "run training to inspect brain inputs, outputs, and selected actions";
    return;
  }

  const clan = trainer.world && trainer.world.clans.find(c => c.brain.id === record.brainId);
  const brain = clan ? clan.brain : record.brain;
  const decision = clan && clan.lastDecision ? clan.lastDecision : brain.lastDecision;
  const lines = [
    `${record.name}  id ${record.brainId}  gen ${record.generation}`,
    `score ${record.score}  clan ${record.clanId || "-"}  phase ${record.phase || "lost"}  people ${record.people}`,
    `dominance ${Math.round((record.dominance || 0) * 100)}%  food-sec ${Math.round((record.foodSecurity || 0) * 100)}%  K/L ${record.kills || 0}/${record.losses || 0}`,
    `food ${record.food}  resources ${record.resources}  territory ${record.territory}  trees ${record.trees || 0}  stockpiles ${record.stockpiles}`,
  ];

  if (!decision) {
    lines.push("", "no decision yet; leaders think every 200 ticks");
    el.textContent = lines.join("\n");
    return;
  }

  lines.push(
    "",
    `last action: ${decision.action}  ${Math.round((decision.score || decision.confidence || 0) * 100)}%`,
    `reason: ${decision.reason || "thinking"}`,
    "",
    "inputs"
  );
  for (const input of LEADER_BRAIN_INPUTS) {
    lines.push(`${input.label.padEnd(17)} ${valueBar(decision.inputs.byKey[input.key])}`);
  }
  lines.push("", "outputs");
  for (const output of LEADER_BRAIN_OUTPUTS) {
    lines.push(`${output.label.padEnd(17)} ${valueBar(decision.outputs[output.key])}`);
  }
  el.textContent = lines.join("\n");
}

function updateTrainingUI() {
  const scores = trainer.state === "running" || !trainer.latestScores.length
    ? trainer.snapshotScores()
    : trainer.latestScores;
  if (!selectedBrainId && scores[0]) selectedBrainId = scores[0].brainId;
  const selected = scores.find(r => r.brainId === selectedBrainId) || scores[0] || null;
  if (selected) selectedBrainId = selected.brainId;

  const progress = trainer.state === "running"
    ? trainer.ticksRun / Math.max(1, trainer.episodeTicks)
    : 0;
  document.getElementById("train-progress").style.width = `${Math.round(progress * 100)}%`;
  const toggle = document.getElementById("btn-train-toggle");
  toggle.textContent = trainingActive ? "Pause Train" : "Train";
  toggle.classList.toggle("active", trainingActive);

  const best = scores[0] || null;
  const avg = scores.reduce((sum, r) => sum + r.score, 0) / Math.max(1, scores.length);
  const stats = document.getElementById("train-stats");
  stats.innerHTML = "";
  const statPairs = [
    ["gen", trainer.generation],
    ["tick", `${trainer.ticksRun}/${trainer.episodeTicks}`],
    ["best", best ? best.score : 0],
    ["avg", Math.round(avg * 10) / 10],
    ["dom", best ? `${Math.round((best.dominance || 0) * 100)}%` : "0%"],
    ["phase", best ? best.phase || "-" : "-"],
    ["leader", best ? best.name : "-"],
    ["state", trainingActive ? "training" : trainer.state],
  ];
  for (const [k, v] of statPairs) {
    const d = document.createElement("div");
    d.textContent = `${k}: ${v}`;
    stats.appendChild(d);
  }

  const list = document.getElementById("league-list");
  list.innerHTML = "";
  const maxScore = Math.max(1, ...scores.map(r => r.score));
  for (const record of scores.slice(0, 12)) {
    const row = document.createElement("div");
    row.className = "league-row";
    row.classList.toggle("active", record.brainId === selectedBrainId);
    row.dataset.brainId = record.brainId;

    const score = document.createElement("div");
    score.textContent = Math.round(record.score);
    const name = document.createElement("div");
    name.textContent = record.name;
    const action = document.createElement("div");
    action.textContent = `${record.phase || "lost"} ${record.people}`;
    const bar = document.createElement("div");
    bar.className = "score-bar";
    const fill = document.createElement("span");
    fill.style.width = `${Math.round(record.score / maxScore * 100)}%`;
    bar.appendChild(fill);

    row.appendChild(score);
    row.appendChild(name);
    row.appendChild(action);
    row.appendChild(bar);
    list.appendChild(row);
  }

  renderBrainInspector(selected);
}

function selectTool(tool) {
  editor.tool = tool;
  for (const btn of document.querySelectorAll("#tools button")) {
    btn.classList.toggle("active", btn.dataset.tool === tool);
  }
}

function setBrushSize(size) {
  const slider = document.getElementById("brush-size");
  size = Math.max(Number(slider.min), Math.min(Number(slider.max), size));
  editor.brushSize = size;
  slider.value = size;
  document.getElementById("brush-label").textContent = size;
}

function setPaused(value) {
  paused = value;
  document.getElementById("btn-pause").textContent = paused ? "Resume" : "Pause";
}

function randomNear(worldRef, cx, cy, radius) {
  return {
    x: worldRef.clamp(cx + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
    y: worldRef.clamp(cy + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
  };
}

function seedBestBrainIntoWorld() {
  const champion = trainer.champion();
  const { x, y } = world.randomCell();
  for (let i = 0; i < 6; i++) {
    const pos = randomNear(world, x, y, INITIAL_CLAIM_RADIUS);
    world.addTree(new Tree(pos.x, pos.y, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
  }
  for (let i = 0; i < 8; i++) {
    const pos = randomNear(world, x, y, 12);
    world.addResource(new Resource(pos.x, pos.y, Math.random() < 0.55 ? "wood" : "stone"));
  }

  const leader = new Entity(x, y, true);
  leader.brain = champion.clone({ preserveId: false, name: `${champion.name} live` });
  world.addEntity(leader);
  const clan = leader.clan;

  for (let i = 0; i < 10; i++) {
    const pos = randomNear(world, x, y, 3);
    const follower = new Entity(pos.x, pos.y, false);
    world.addEntity(follower);
    clan.addMember(follower, "seed");
  }
  for (let i = 0; i < 5; i++) {
    const pos = randomNear(world, x, y, 10);
    world.addEntity(new Entity(pos.x, pos.y, false));
  }
  if (!clan.isTown && clan.members.length >= TOWN_FOUNDING_FOLLOWERS) clan.foundTown(world);
  const home = clan.stockpiles[0];
  if (home) {
    home.food += 45;
    home.resources.wood = (home.resources.wood || 0) + 25;
    home.resources.stone = (home.resources.stone || 0) + 18;
  }
  clan.revealArea(world, x, y, VISION_RADIUS);
  status(`seeded ${champion.name} into clan #${clan.id}`);
}

document.getElementById("tools").addEventListener("click", e => {
  if (e.target.dataset.tool) selectTool(e.target.dataset.tool);
});

document.getElementById("brush-size").addEventListener("input", e => {
  setBrushSize(Number(e.target.value));
});

document.getElementById("speed").addEventListener("input", e => {
  ticksPerFrame = Number(e.target.value);
  document.getElementById("speed-label").textContent = ticksPerFrame;
});

document.getElementById("train-speed").addEventListener("input", e => {
  document.getElementById("train-speed-label").textContent = e.target.value;
  trainer.configure({ ticksPerFrame: Number(e.target.value) });
});

document.getElementById("train-pop").addEventListener("change", () => {
  applyTrainingSettings();
  selectedBrainId = null;
});

document.getElementById("train-episode").addEventListener("change", applyTrainingSettings);

document.getElementById("wood-pct").addEventListener("input", e => {
  document.getElementById("wood-pct-label").textContent = e.target.value;
});

document.getElementById("btn-pause").addEventListener("click", () => setPaused(!paused));

document.getElementById("btn-step").addEventListener("click", () => {
  setPaused(true);
  world.step();
});

document.getElementById("btn-train-toggle").addEventListener("click", () => {
  applyTrainingSettings();
  trainingActive = !trainingActive;
});

document.getElementById("btn-train-step").addEventListener("click", () => {
  applyTrainingSettings();
  trainingActive = false;
  trainer.runGeneration();
  updateTrainingUI();
});

document.getElementById("btn-train-reset").addEventListener("click", () => {
  applyTrainingSettings();
  trainer.resetPopulation();
  selectedBrainId = null;
  status("training league reset");
});

document.getElementById("btn-seed-best").addEventListener("click", seedBestBrainIntoWorld);

document.getElementById("league-list").addEventListener("click", e => {
  const row = e.target.closest(".league-row");
  if (!row) return;
  selectedBrainId = Number(row.dataset.brainId);
  updateTrainingUI();
});

document.getElementById("btn-populate").addEventListener("click", populate);

document.getElementById("btn-clear").addEventListener("click", () => {
  world.clear();
  status("world cleared");
});

document.getElementById("btn-save").addEventListener("click", () => {
  localStorage.setItem(SAVE_KEY, JSON.stringify(world.serialize()));
  status("saved to browser storage");
});

document.getElementById("btn-load").addEventListener("click", () => {
  const raw = localStorage.getItem(SAVE_KEY);
  if (!raw) {
    status("no save found");
    return;
  }
  world.deserialize(JSON.parse(raw));
  status("loaded");
});

document.getElementById("btn-export").addEventListener("click", () => {
  const blob = new Blob([JSON.stringify(world.serialize(), null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "life-world.json";
  a.click();
  URL.revokeObjectURL(a.href);
  status("exported life-world.json");
});

document.getElementById("btn-import").addEventListener("click", () => {
  document.getElementById("import-file").click();
});

document.getElementById("import-file").addEventListener("change", e => {
  const file = e.target.files[0];
  if (!file) return;
  file.text().then(text => {
    try {
      world.deserialize(JSON.parse(text));
      status(`imported ${file.name}`);
    } catch (err) {
      status("import failed: not a valid world file");
    }
  });
  e.target.value = "";
});

const TOOL_KEYS = {
  Digit1: "pellet",
  Digit2: "entity",
  Digit3: "tree",
  Digit4: "wood",
  Digit5: "stone",
  Digit6: "eraser",
  Digit7: "inspect",
};

document.addEventListener("keydown", e => {
  if (e.target.tagName === "INPUT") return;
  if (e.code === "Space") {
    setPaused(!paused);
    e.preventDefault();
  } else if (TOOL_KEYS[e.code]) {
    selectTool(TOOL_KEYS[e.code]);
  } else if (e.code === "BracketLeft") {
    setBrushSize(editor.brushSize - 1);
  } else if (e.code === "BracketRight") {
    setBrushSize(editor.brushSize + 1);
  }
});

applyTrainingSettings();
populate();
updateTrainingUI();
loop();
