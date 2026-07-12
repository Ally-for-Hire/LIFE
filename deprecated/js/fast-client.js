const canvas = document.getElementById("world");
const ctx = canvas.getContext("2d");

let latest = null;
let polling = false;

const colors = {
  bg: "#111418",
  pellet: "#48b868",
  tree: "#2f9f4f",
  treeCore: "#8bd36d",
  resourceWood: "#9a6738",
  resourceStone: "#9ba3aa",
  entity: "#e2e6eb",
  leader: "#ffffff",
  stockpile: "#d7b84c",
  neutralClaim: "rgba(180, 186, 195, 0.12)",
  vision: 0.045,
};

async function api(path, body) {
  const options = body === undefined
    ? {}
    : {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      };
  const res = await fetch(path, options);
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

function stat(container, pairs) {
  container.innerHTML = "";
  for (const [k, v, cls] of pairs) {
    const el = document.createElement("div");
    const a = document.createElement("span");
    const b = document.createElement("span");
    a.textContent = k;
    b.textContent = v;
    if (cls) b.className = cls;
    el.appendChild(a);
    el.appendChild(b);
    container.appendChild(el);
  }
}

function parseKey(key) {
  const i = key.indexOf(",");
  return [Number(key.slice(0, i)), Number(key.slice(i + 1))];
}

function syncInputValue(id, value) {
  const el = document.getElementById(id);
  if (document.activeElement !== el) el.value = value;
}

function draw() {
  if (!latest) return;
  const world = latest.world;
  const scale = canvas.width / world.size;

  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.setTransform(scale, 0, 0, scale, 0, 0);

  for (const neutral of world.neutralClaims) {
    ctx.fillStyle = colors.neutralClaim;
    for (const key of neutral.claimed) {
      const [x, y] = parseKey(key);
      ctx.fillRect(x, y, 1, 1);
    }
  }

  for (const clan of world.clans) {
    ctx.globalAlpha = 0.15;
    ctx.fillStyle = clan.color;
    for (const key of clan.claimed) {
      const [x, y] = parseKey(key);
      ctx.fillRect(x, y, 1, 1);
    }
    ctx.globalAlpha = 1;
  }

  ctx.fillStyle = colors.pellet;
  for (const [x, y] of world.pellets) ctx.fillRect(x, y, 1, 1);

  for (const [x, y] of world.trees) {
    ctx.fillStyle = colors.tree;
    ctx.fillRect(x - 1, y - 1, 3, 3);
    ctx.fillStyle = colors.treeCore;
    ctx.fillRect(x, y, 1, 1);
  }

  for (const [x, y, type] of world.resources) {
    ctx.fillStyle = type === "wood" ? colors.resourceWood : colors.resourceStone;
    ctx.fillRect(x, y, 1, 1);
  }

  for (const clan of world.clans) {
    for (const [x, y] of clan.stockpiles) {
      ctx.fillStyle = colors.stockpile;
      ctx.fillRect(x, y, 1, 1);
      ctx.strokeStyle = clan.color;
      ctx.lineWidth = 0.5;
      ctx.strokeRect(x - 1, y - 1, 3, 3);
    }
  }

  for (const entity of world.entities) {
    const [, x, y, clanId, isLeader, health] = entity;
    const clan = clanId ? world.clans.find(c => c.id === clanId) : null;
    ctx.fillStyle = clan ? clan.color : colors.entity;
    ctx.fillRect(x, y, 1, 1);
    if (isLeader) {
      ctx.strokeStyle = health <= 3 ? "#d86161" : colors.leader;
      ctx.lineWidth = 0.5;
      ctx.strokeRect(x - 1, y - 1, 3, 3);
    }
  }
}

function renderLists() {
  const world = latest.world;
  const clans = document.getElementById("clan-list");
  clans.innerHTML = "";
  for (const clan of world.clans.slice().sort((a, b) => b.people - a.people)) {
    const food = clan.stockpiles.reduce((sum, s) => sum + s[2], 0);
    const el = document.createElement("div");
    el.className = "item";
    el.innerHTML = `
      <span class="num">#${clan.id}</span>
      <span><i class="swatch" style="background:${clan.color}"></i>${clan.phase} ${clan.people}</span>
      <span class="num">${food} food</span>
      <span class="num">K${clan.kills}/L${clan.losses}</span>
      <span>${clan.lastAction}</span>
    `;
    clans.appendChild(el);
  }
  if (!world.clans.length) clans.innerHTML = `<span class="muted">no active clans</span>`;

  const league = document.getElementById("league-list");
  league.innerHTML = "";
  for (const record of latest.trainer.scores) {
    const el = document.createElement("div");
    el.className = "item";
    el.innerHTML = `
      <span class="num">${Math.round(record.score)}</span>
      <span>${record.name}</span>
      <span class="num">${record.phase} ${record.people}</span>
      <span class="num">${Math.round((record.dominance || 0) * 100)}%</span>
      <span>${record.action}</span>
    `;
    league.appendChild(el);
  }
}

function renderStats() {
  const world = latest.world;
  const runtime = latest.runtime;
  const trainer = latest.trainer;
  const stockpileFood = world.clans.reduce((sum, clan) =>
    sum + clan.stockpiles.reduce((s, stockpile) => s + stockpile[2], 0), 0);

  document.getElementById("summary").innerHTML = `
    <span class="pill">tick ${world.tick.toLocaleString()}</span>
    <span class="pill">grace ${world.graceRemaining.toLocaleString()}</span>
    <span class="pill">${runtime.tps.toLocaleString()} ticks/s</span>
    <span class="pill">${world.counts.entities} entities</span>
    <span class="pill">${world.counts.clans} clans</span>
    <span class="pill">${world.counts.towns} towns</span>
  `;

  stat(document.getElementById("sim-stats"), [
    ["state", runtime.simRunning ? "running" : "paused", runtime.simRunning ? "good" : ""],
    ["tick", world.tick.toLocaleString()],
    ["grace", world.graceRemaining.toLocaleString()],
    ["ticks/s", runtime.tps.toLocaleString(), runtime.tps > 0 ? "good" : ""],
    ["batch", runtime.ticksPerBatch.toLocaleString()],
    ["entities", world.counts.entities],
    ["towns", world.counts.towns],
    ["stored food", stockpileFood],
    ["pellets", world.counts.pellets],
    ["trees", world.counts.trees],
    ["resources", world.counts.resources],
  ]);

  const trainingState = runtime.parallelGenerationActive
    ? `parallel ${runtime.parallelCompleted}/${runtime.parallelEpisodes}`
    : (runtime.trainingRunning ? "training" : trainer.state);
  stat(document.getElementById("train-stats"), [
    ["state", trainingState, runtime.trainingRunning || runtime.parallelGenerationActive ? "good" : ""],
    ["generation", trainer.generation],
    ["episode", `${trainer.ticksRun.toLocaleString()} / ${trainer.episodeTicks.toLocaleString()}`],
    ["train ticks/s", runtime.trainTicksPerSecond.toLocaleString()],
    ["workers", runtime.workerCount],
    ["batch", runtime.trainTicksPerBatch.toLocaleString()],
    ["last gen", runtime.parallelLastMs ? `${runtime.parallelLastMs.toLocaleString()} ms` : "-"],
    ["best", trainer.scores[0] ? Math.round(trainer.scores[0].score) : 0],
    ["best pop", trainer.scores[0] ? trainer.scores[0].people : 0],
    ["best dom", trainer.scores[0] ? `${Math.round((trainer.scores[0].dominance || 0) * 100)}%` : "0%"],
    ["error", runtime.parallelError || "-", runtime.parallelError ? "bad" : ""],
  ]);

  syncInputValue("ticks-batch", runtime.ticksPerBatch);
  syncInputValue("train-batch", runtime.trainTicksPerBatch);
  syncInputValue("worker-count", runtime.workerCount);

  document.getElementById("sim-toggle").textContent = runtime.simRunning ? "Pause" : "Run";
  document.getElementById("sim-toggle").classList.toggle("active", runtime.simRunning);
  document.getElementById("train-toggle").textContent = runtime.trainingRunning ? "Pause Train" : "Train";
  document.getElementById("train-toggle").classList.toggle("active", runtime.trainingRunning);
}

function render() {
  if (!latest) return;
  draw();
  renderStats();
  renderLists();
}

async function poll() {
  if (polling) return;
  polling = true;
  try {
    latest = await api("/api/state");
    render();
  } catch (err) {
    document.getElementById("summary").textContent = err.message;
  } finally {
    polling = false;
  }
}

async function syncControl(changes) {
  latest = await api("/api/control", {
    ticksPerBatch: Number(document.getElementById("ticks-batch").value),
    trainTicksPerBatch: Number(document.getElementById("train-batch").value),
    workerCount: Number(document.getElementById("worker-count").value),
    ...changes,
  });
  render();
}

document.getElementById("sim-toggle").addEventListener("click", () => {
  syncControl({ simRunning: !(latest && latest.runtime.simRunning) });
});

document.getElementById("train-toggle").addEventListener("click", () => {
  syncControl({ trainingRunning: !(latest && latest.runtime.trainingRunning) });
});

document.getElementById("ticks-batch").addEventListener("change", () => syncControl({}));
document.getElementById("train-batch").addEventListener("change", () => syncControl({}));
document.getElementById("worker-count").addEventListener("change", () => syncControl({}));

document.getElementById("step-1k").addEventListener("click", async () => {
  latest = await api("/api/step", { ticks: 1000 });
  render();
});

document.getElementById("step-10k").addEventListener("click", async () => {
  latest = await api("/api/step", { ticks: 10000 });
  render();
});

document.getElementById("populate").addEventListener("click", async () => {
  latest = await api("/api/populate", {
    clear: true,
    trees: Number(document.getElementById("pop-trees").value),
    entities: Number(document.getElementById("pop-entities").value),
    leaders: Number(document.getElementById("pop-leaders").value),
    resources: Number(document.getElementById("pop-resources").value),
  });
  render();
});

document.getElementById("reset-training").addEventListener("click", async () => {
  latest = await api("/api/training/reset", {});
  render();
});

document.getElementById("run-generation").addEventListener("click", async () => {
  latest = await api("/api/training/generation", {});
  render();
});

document.getElementById("seed-best").addEventListener("click", async () => {
  const result = await api("/api/seed-best", {});
  latest = result.snapshot;
  render();
});

poll();
setInterval(poll, 250);
