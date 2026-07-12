const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const vm = require("vm");
const { Worker } = require("worker_threads");

const ROOT = __dirname;
const PORT = Number(process.env.PORT) || 8787;
const CPU_COUNT = os.availableParallelism ? os.availableParallelism() : (os.cpus().length || 4);
const ENV_WORKER_COUNT = Number(process.env.LIFE_WORKERS);
const DEFAULT_WORKER_COUNT = Number.isFinite(ENV_WORKER_COUNT) && ENV_WORKER_COUNT > 0 ? ENV_WORKER_COUNT : 8;
const MAX_WORKER_COUNT = Math.max(8, Math.min(64, CPU_COUNT * 2));
const WORKER_PATH = path.join(ROOT, "training-worker.js");
const SIM_FILES = [
  "pellet.js",
  "tree.js",
  "resource.js",
  "stockpile.js",
  "task.js",
  "brain.js",
  "clan.js",
  "entity.js",
  "world.js",
  "trainer.js",
];

const context = vm.createContext({ console, Math, Date, Set, Map, JSON });
for (const file of SIM_FILES) {
  const source = fs.readFileSync(path.join(ROOT, "js", file), "utf8");
  vm.runInContext(source, context, { filename: file });
}
vm.runInContext(`
  globalThis.LIFE = {
    Pellet, Tree, Resource, Stockpile, Task, Clan, Entity, World, LeaderBrain, TrainingArena,
    TREE_DEFAULT_INTERVAL, INITIAL_CLAIM_RADIUS, CLAN_RECRUIT_RADIUS
  };
`, context);

const LIFE = context.LIFE;
let world = new LIFE.World(256);
let trainer = new LIFE.TrainingArena();

const runtime = {
  simRunning: false,
  trainingRunning: false,
  ticksPerBatch: 2000,
  trainTicksPerBatch: 5000,
  workerCount: Math.min(DEFAULT_WORKER_COUNT, MAX_WORKER_COUNT),
  parallelTraining: true,
  parallelGenerationActive: false,
  parallelCompleted: 0,
  parallelEpisodes: 0,
  parallelLastMs: 0,
  parallelError: "",
  tps: 0,
  trainTicksPerSecond: 0,
  trainingTicksTotal: 0,
  lastMeterAt: Date.now(),
  lastWorldTick: 0,
  lastTrainingTicksTotal: 0,
};

let trainingJob = null;
let trainingVersion = 0;
const workerPool = [];

function randomNear(worldRef, cx, cy, radius) {
  return {
    x: worldRef.clamp(cx + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
    y: worldRef.clamp(cy + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
  };
}

function populateWorld(options = {}) {
  if (options.clear !== false) world.clear();

  const trees = Number(options.trees ?? 55) || 0;
  const entities = Number(options.entities ?? 40) || 0;
  const leaders = Number(options.leaders ?? 4) || 0;
  const resources = Number(options.resources ?? 60) || 0;
  const woodPct = Number(options.woodPct ?? 0.5);

  for (let i = 0; i < resources; i++) {
    const { x, y } = world.randomCell();
    world.addResource(new LIFE.Resource(x, y, Math.random() < woodPct ? "wood" : "stone"));
  }

  const spawned = [];
  const leaderCount = Math.min(leaders, entities);
  let remainingEntities = Math.max(0, entities - leaderCount);
  let remainingTrees = trees;

  for (let i = 0; i < leaderCount; i++) {
    const { x, y } = world.randomCell();
    const leader = new LIFE.Entity(x, y, true);
    world.addEntity(leader);
    const clan = leader.clan;

    const localTrees = Math.min(2, remainingTrees);
    for (let n = 0; n < localTrees; n++) {
      const pos = randomNear(world, x, y, LIFE.INITIAL_CLAIM_RADIUS);
      world.addTree(new LIFE.Tree(pos.x, pos.y, {
        lastSpawnTick: -Math.floor(Math.random() * LIFE.TREE_DEFAULT_INTERVAL),
      }));
      remainingTrees--;
    }

    const startingFollowers = Math.min(2, remainingEntities);
    for (let n = 0; n < startingFollowers; n++) {
      const pos = randomNear(world, x, y, 2);
      const follower = new LIFE.Entity(pos.x, pos.y, false);
      world.addEntity(follower);
      clan.addMember(follower, "seed");
      remainingEntities--;
    }

    const localCandidates = Math.min(4, remainingEntities);
    for (let n = 0; n < localCandidates; n++) {
      let pos = randomNear(world, x, y, 8);
      if (Math.max(Math.abs(pos.x - x), Math.abs(pos.y - y)) <= LIFE.CLAN_RECRUIT_RADIUS) {
        pos = {
          x: world.clamp(x + (pos.x >= x ? LIFE.CLAN_RECRUIT_RADIUS + 2 : -LIFE.CLAN_RECRUIT_RADIUS - 2)),
          y: world.clamp(y + (pos.y >= y ? LIFE.CLAN_RECRUIT_RADIUS + 2 : -LIFE.CLAN_RECRUIT_RADIUS - 2)),
        };
      }
      spawned.push(new LIFE.Entity(pos.x, pos.y, false));
      remainingEntities--;
    }
  }

  for (let i = 0; i < remainingEntities; i++) {
    const { x, y } = world.randomCell();
    spawned.push(new LIFE.Entity(x, y, false));
  }
  for (let i = 0; i < remainingTrees; i++) {
    const { x, y } = world.randomCell();
    world.addTree(new LIFE.Tree(x, y, {
      lastSpawnTick: -Math.floor(Math.random() * LIFE.TREE_DEFAULT_INTERVAL),
    }));
  }
  for (const entity of spawned) world.addEntity(entity);
}

function seedBestBrain() {
  const champion = trainer.champion();
  const { x, y } = world.randomCell();
  for (let i = 0; i < 6; i++) {
    const pos = randomNear(world, x, y, LIFE.INITIAL_CLAIM_RADIUS);
    world.addTree(new LIFE.Tree(pos.x, pos.y, { lastSpawnTick: -LIFE.TREE_DEFAULT_INTERVAL }));
  }
  for (let i = 0; i < 8; i++) {
    const pos = randomNear(world, x, y, 12);
    world.addResource(new LIFE.Resource(pos.x, pos.y, Math.random() < 0.55 ? "wood" : "stone"));
  }

  const leader = new LIFE.Entity(x, y, true);
  leader.brain = champion.clone({ preserveId: false, name: `${champion.name} live` });
  world.addEntity(leader);
  const clan = leader.clan;

  for (let i = 0; i < 10; i++) {
    const pos = randomNear(world, x, y, 3);
    const follower = new LIFE.Entity(pos.x, pos.y, false);
    world.addEntity(follower);
    clan.addMember(follower, "seed");
  }
  for (let i = 0; i < 5; i++) {
    const pos = randomNear(world, x, y, 10);
    world.addEntity(new LIFE.Entity(pos.x, pos.y, false));
  }
  if (!clan.isTown && clan.members.length >= 3) clan.foundTown(world);
  const home = clan.stockpiles[0];
  if (home) {
    home.food += 45;
    home.resources.wood = (home.resources.wood || 0) + 25;
    home.resources.stone = (home.resources.stone || 0) + 18;
  }
  clan.revealArea(world, x, y, 15);
  return clan.id;
}

function stepWorld(count) {
  for (let i = 0; i < count; i++) world.step();
}

function clampInt(value, min, max, fallback) {
  const n = Math.floor(Number(value));
  if (!Number.isFinite(n)) return fallback;
  return Math.max(min, Math.min(max, n));
}

function createSeededRandom(seed) {
  let state = seed >>> 0 || 1;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 4294967296;
  };
}

function withSeededRandom(seed, fn) {
  const previousRandom = Math.random;
  Math.random = createSeededRandom(seed);
  try {
    return fn();
  } finally {
    Math.random = previousRandom;
  }
}

function serializePopulation() {
  return trainer.population.map(brain => brain.serialize());
}

function rejectWorkerPending(slot, err) {
  if (!slot || !slot.pending) return;
  const pending = slot.pending;
  slot.pending = null;
  pending.reject(err);
}

function createWorkerSlot(index) {
  const worker = new Worker(WORKER_PATH);
  const slot = { index, worker, pending: null };

  worker.on("message", message => {
    if (!slot.pending) return;
    const pending = slot.pending;
    slot.pending = null;
    if (!message || !message.ok) {
      pending.reject(new Error(message && message.error ? message.error : "training worker failed"));
      return;
    }
    pending.resolve(message.result);
  });
  worker.on("error", err => {
    rejectWorkerPending(slot, err);
    if (workerPool[index] === slot) workerPool[index] = null;
  });
  worker.on("exit", code => {
    if (code !== 0) rejectWorkerPending(slot, new Error(`training worker exited with code ${code}`));
    if (workerPool[index] === slot) workerPool[index] = null;
  });

  return slot;
}

function workerSlot(index) {
  let slot = workerPool[index];
  if (!slot) {
    slot = createWorkerSlot(index);
    workerPool[index] = slot;
  }
  return slot;
}

function trimWorkerPool(count) {
  for (let i = count; i < workerPool.length; i++) {
    const slot = workerPool[i];
    if (!slot) continue;
    if (slot.pending) continue;
    slot.worker.terminate().catch(() => {});
    workerPool[i] = null;
  }
  while (workerPool.length > count && !workerPool[workerPool.length - 1]) workerPool.pop();
}

function runWorkerEpisode(payload) {
  const slot = workerSlot(payload.episodeIndex);
  if (slot.pending) return Promise.reject(new Error(`training worker ${payload.episodeIndex} is already busy`));

  return new Promise((resolve, reject) => {
    slot.pending = { resolve, reject };
    slot.worker.postMessage(payload);
  });
}

function averageRounded(sum, count, digits = 1) {
  const factor = 10 ** digits;
  return Math.round((sum / Math.max(1, count)) * factor) / factor;
}

function aggregateParallelScores(results) {
  const byBrain = new Map();
  for (const result of results) {
    for (const score of result.scores) {
      let record = byBrain.get(score.brainId);
      if (!record) {
        record = {
          brainId: score.brainId,
          name: score.name,
          generation: score.generation,
          runs: 0,
          scoreSum: 0,
          peopleSum: 0,
          foodSum: 0,
          resourcesSum: 0,
          territorySum: 0,
          stockpilesSum: 0,
          visionSum: 0,
          treesSum: 0,
          dominanceSum: 0,
          foodSecuritySum: 0,
          killsSum: 0,
          lossesSum: 0,
          recruitsSum: 0,
          best: score,
        };
        byBrain.set(score.brainId, record);
      }
      record.runs++;
      record.scoreSum += score.score || 0;
      record.peopleSum += score.people || 0;
      record.foodSum += score.food || 0;
      record.resourcesSum += score.resources || 0;
      record.territorySum += score.territory || 0;
      record.stockpilesSum += score.stockpiles || 0;
      record.visionSum += score.vision || 0;
      record.treesSum += score.trees || 0;
      record.dominanceSum += score.dominance || 0;
      record.foodSecuritySum += score.foodSecurity || 0;
      record.killsSum += score.kills || 0;
      record.lossesSum += score.losses || 0;
      record.recruitsSum += score.recruits || 0;
      if ((score.score || 0) > (record.best.score || 0)) record.best = score;
    }
  }

  return trainer.population.map(brain => {
    const record = byBrain.get(brain.id);
    if (!record) {
      return {
        brainId: brain.id,
        name: brain.name,
        generation: brain.generation,
        brain,
        score: brain.lastScore || 0,
        alive: false,
        people: 0,
        food: 0,
        resources: 0,
        territory: 0,
        stockpiles: 0,
        vision: 0,
        trees: 0,
        dominance: 0,
        foodSecurity: 0,
        kills: 0,
        losses: 0,
        recruits: 0,
        phase: "lost",
        action: "lost",
      };
    }

    const best = record.best;
    const score = {
      brainId: brain.id,
      name: brain.name,
      generation: brain.generation,
      brain,
      score: averageRounded(record.scoreSum, record.runs),
      alive: best.alive,
      clanId: best.clanId,
      people: Math.round(record.peopleSum / record.runs),
      food: Math.round(record.foodSum / record.runs),
      resources: Math.round(record.resourcesSum / record.runs),
      territory: Math.round(record.territorySum / record.runs),
      stockpiles: Math.round(record.stockpilesSum / record.runs),
      vision: Math.round(record.visionSum / record.runs),
      trees: Math.round(record.treesSum / record.runs),
      dominance: averageRounded(record.dominanceSum, record.runs, 3),
      foodSecurity: averageRounded(record.foodSecuritySum, record.runs, 3),
      kills: Math.round(record.killsSum / record.runs),
      losses: Math.round(record.lossesSum / record.runs),
      recruits: Math.round(record.recruitsSum / record.runs),
      phase: best.phase,
      action: best.action,
    };
    brain.lastScore = score.score;
    return score;
  }).sort((a, b) => b.score - a.score);
}

function finishParallelGeneration(results) {
  const scores = aggregateParallelScores(results);
  trainer.latestScores = scores;
  const avg = scores.reduce((sum, r) => sum + r.score, 0) / Math.max(1, scores.length);
  const best = scores[0] || null;

  if (best) {
    const bestBrain = trainer.population.find(brain => brain.id === best.brainId);
    if (bestBrain && (!trainer.bestBrain || best.score >= trainer.bestScore)) {
      trainer.bestScore = best.score;
      trainer.bestBrain = bestBrain.clone({ preserveId: true });
    }
    trainer.history.push({
      generation: trainer.generation,
      best: best.score,
      avg,
      bestName: best.name,
    });
    if (trainer.history.length > 80) trainer.history.shift();
  }

  withSeededRandom(trainer.options.seed + trainer.generation * 31337, () => trainer.evolve(scores));
  trainer.state = "idle";
  trainer.world = null;
  trainer.rng = null;
  trainer.ticksRun = trainer.options.episodeTicks;
}

function startParallelGeneration() {
  if (trainingJob) return trainingJob;

  const version = trainingVersion;
  const startedAt = Date.now();
  const generation = trainer.generation + 1;
  const population = serializePopulation();
  const options = { ...trainer.options };
  const episodes = clampInt(runtime.workerCount, 1, MAX_WORKER_COUNT, 1);
  trimWorkerPool(episodes);

  trainer.generation = generation;
  trainer.ticksRun = 0;
  trainer.state = "running";
  runtime.parallelTraining = episodes > 1;
  runtime.parallelGenerationActive = true;
  runtime.parallelCompleted = 0;
  runtime.parallelEpisodes = episodes;
  runtime.parallelError = "";

  const tasks = Array.from({ length: episodes }, (_, episodeIndex) => {
    const payload = { options, population, generation, episodeIndex };
    return runWorkerEpisode(payload).then(result => {
      runtime.parallelCompleted++;
      runtime.trainingTicksTotal += result.ticksRun || options.episodeTicks;
      trainer.ticksRun = Math.floor(options.episodeTicks * runtime.parallelCompleted / episodes);
      return result;
    });
  });

  trainingJob = Promise.all(tasks)
    .then(results => {
      if (version !== trainingVersion) return;
      finishParallelGeneration(results);
      runtime.parallelLastMs = Date.now() - startedAt;
    })
    .catch(err => {
      runtime.trainingRunning = false;
      runtime.parallelError = err && err.message ? err.message : String(err);
      trainer.state = "idle";
    })
    .finally(() => {
      if (version === trainingVersion) {
        runtime.parallelGenerationActive = false;
        runtime.parallelCompleted = 0;
        runtime.parallelEpisodes = 0;
        trimWorkerPool(runtime.workerCount > 1 ? runtime.workerCount : 0);
      }
      trainingJob = null;
      if (runtime.trainingRunning) setImmediate(pump);
    });

  return trainingJob;
}

async function runOneTrainingGeneration() {
  if (runtime.workerCount > 1) {
    await startParallelGeneration();
    return;
  }
  trainer.runGeneration();
  runtime.trainingTicksTotal += trainer.options.episodeTicks;
}

function pump() {
  if (runtime.simRunning) stepWorld(runtime.ticksPerBatch);
  if (runtime.trainingRunning) {
    if (runtime.workerCount > 1) {
      startParallelGeneration();
    } else if (!trainingJob) {
      trainer.advance(runtime.trainTicksPerBatch);
      runtime.trainingTicksTotal += runtime.trainTicksPerBatch;
    }
  }

  const now = Date.now();
  if (now - runtime.lastMeterAt >= 1000) {
    const elapsed = (now - runtime.lastMeterAt) / 1000;
    runtime.tps = Math.round((world.tick - runtime.lastWorldTick) / elapsed);
    runtime.trainTicksPerSecond = Math.round((runtime.trainingTicksTotal - runtime.lastTrainingTicksTotal) / elapsed);
    runtime.lastMeterAt = now;
    runtime.lastWorldTick = world.tick;
    runtime.lastTrainingTicksTotal = runtime.trainingTicksTotal;
  }

  if (runtime.simRunning || (runtime.trainingRunning && runtime.workerCount <= 1)) setImmediate(pump);
  else setTimeout(pump, 50);
}

function clanSnapshot(clan) {
  const neutral = clan.relationshipWithNeutral();
  return {
    id: clan.id,
    color: clan.color,
    leaderId: clan.leader ? clan.leader.id : null,
    people: clan.everyone().filter(e => !e.dead).length,
    members: clan.members.length,
    phase: clan.phase(world),
    stockpiles: clan.stockpiles.map(s => [s.x, s.y, s.food, s.totalResources()]),
    claimed: [...clan.claimed],
    visionSize: clan.vision.size,
    lastAction: clan.lastDecision ? clan.lastDecision.action : "none",
    lastReason: clan.lastDecision ? clan.lastDecision.reason : "",
    score: clan.brain ? clan.brain.lastScore || 0 : 0,
    kills: clan.stats.kills,
    losses: clan.stats.losses,
    recruits: clan.stats.recruits,
    neutralFriendliness: neutral.friendliness,
    neutralAnimosity: neutral.animosity,
  };
}

function worldSnapshot() {
  const scores = trainer.state === "running" || !trainer.latestScores.length
    ? trainer.snapshotScores()
    : trainer.latestScores;
  return {
    runtime,
    world: {
      size: world.size,
      tick: world.tick,
      graceRemaining: world.clanGraceRemaining(),
      pellets: [...world.pellets.values()].map(p => [p.x, p.y]),
      trees: world.trees.map(t => [t.x, t.y, t.health]),
      resources: world.resources.map(r => [r.x, r.y, r.type]),
      entities: world.entities.map(e => [
        e.id,
        e.x,
        e.y,
        e.clan ? e.clan.id : 0,
        e.isLeader ? 1 : 0,
        Math.max(0, Math.round(e.health * 10) / 10),
        e.task ? e.task.type : "",
      ]),
      neutralClaims: world.entities
        .filter(e => !e.clan && e.claimed.size)
        .map(e => ({ id: e.id, claimed: [...e.claimed] })),
      clans: world.clans.map(clanSnapshot),
      counts: {
        entities: world.entities.length,
        clans: world.clans.length,
        towns: world.clans.filter(c => c.isTown).length,
        pellets: world.pellets.size,
        trees: world.trees.length,
        resources: world.resources.length,
      },
    },
    trainer: {
      generation: trainer.generation,
      ticksRun: trainer.ticksRun,
      episodeTicks: trainer.episodeTicks,
      state: trainer.state,
      history: trainer.history,
      scores: scores.slice(0, 16).map(r => ({
        brainId: r.brainId,
        name: r.name,
        score: r.score,
        people: r.people,
        trees: r.trees || 0,
        dominance: r.dominance || 0,
        foodSecurity: r.foodSecurity || 0,
        kills: r.kills || 0,
        losses: r.losses || 0,
        phase: r.phase || "lost",
        action: r.action,
      })),
    },
  };
}

function sendJson(res, status, data) {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  res.end(body);
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.on("data", chunk => {
      body += chunk;
      if (body.length > 1_000_000) {
        reject(new Error("body too large"));
        req.destroy();
      }
    });
    req.on("end", () => {
      if (!body) return resolve({});
      try {
        resolve(JSON.parse(body));
      } catch (err) {
        reject(err);
      }
    });
  });
}

function serveFile(res, filePath, type) {
  fs.readFile(filePath, (err, data) => {
    if (err) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    res.writeHead(200, { "content-type": type, "cache-control": "no-store" });
    res.end(data);
  });
}

const routes = {
  "GET /api/state": (_req, res) => sendJson(res, 200, worldSnapshot()),
  "POST /api/control": async (req, res) => {
    const body = await readJson(req);
    if (body.simRunning !== undefined) runtime.simRunning = !!body.simRunning;
    if (body.trainingRunning !== undefined) runtime.trainingRunning = !!body.trainingRunning;
    if (body.ticksPerBatch !== undefined) runtime.ticksPerBatch = Math.max(1, Math.min(50000, Number(body.ticksPerBatch) || 1));
    if (body.trainTicksPerBatch !== undefined) runtime.trainTicksPerBatch = Math.max(1, Math.min(50000, Number(body.trainTicksPerBatch) || 1));
    if (body.workerCount !== undefined) {
      runtime.workerCount = clampInt(body.workerCount, 1, MAX_WORKER_COUNT, runtime.workerCount);
      runtime.parallelTraining = runtime.workerCount > 1;
      if (!runtime.parallelGenerationActive) trimWorkerPool(runtime.workerCount > 1 ? runtime.workerCount : 0);
    }
    sendJson(res, 200, worldSnapshot());
  },
  "POST /api/step": async (req, res) => {
    const body = await readJson(req);
    stepWorld(Math.max(1, Math.min(100000, Number(body.ticks) || 1)));
    sendJson(res, 200, worldSnapshot());
  },
  "POST /api/populate": async (req, res) => {
    const body = await readJson(req);
    populateWorld(body);
    sendJson(res, 200, worldSnapshot());
  },
  "POST /api/training/reset": (_req, res) => {
    trainingVersion++;
    runtime.trainingRunning = false;
    runtime.parallelGenerationActive = false;
    runtime.parallelCompleted = 0;
    runtime.parallelEpisodes = 0;
    runtime.parallelError = "";
    runtime.trainingTicksTotal = 0;
    runtime.lastTrainingTicksTotal = 0;
    trainer = new LIFE.TrainingArena();
    sendJson(res, 200, worldSnapshot());
  },
  "POST /api/training/generation": async (_req, res) => {
    await runOneTrainingGeneration();
    sendJson(res, 200, worldSnapshot());
  },
  "POST /api/seed-best": (_req, res) => {
    const clanId = seedBestBrain();
    sendJson(res, 200, { clanId, snapshot: worldSnapshot() });
  },
};

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  const key = `${req.method} ${url.pathname}`;
  try {
    if (routes[key]) {
      await routes[key](req, res);
      return;
    }
    if (req.method === "GET" && (url.pathname === "/" || url.pathname === "/fast.html")) {
      serveFile(res, path.join(ROOT, "fast.html"), "text/html; charset=utf-8");
      return;
    }
    if (req.method === "GET" && url.pathname === "/js/fast-client.js") {
      serveFile(res, path.join(ROOT, "js", "fast-client.js"), "text/javascript; charset=utf-8");
      return;
    }
    res.writeHead(404);
    res.end("not found");
  } catch (err) {
    sendJson(res, 500, { error: err.message });
  }
});

populateWorld({ clear: true });
pump();

server.listen(PORT, "127.0.0.1", () => {
  console.log(`LIFE fast runner listening at http://127.0.0.1:${PORT}/`);
});
