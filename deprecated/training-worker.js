const fs = require("fs");
const path = require("path");
const vm = require("vm");
const { parentPort } = require("worker_threads");

const ROOT = __dirname;
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
    Pellet, Tree, Resource, Stockpile, Task, Clan, Entity, World, LeaderBrain, TrainingArena
  };
`, context);

const LIFE = context.LIFE;

function scoreSnapshot(record) {
  return {
    brainId: record.brainId,
    name: record.name,
    generation: record.generation,
    score: record.score,
    alive: record.alive,
    clanId: record.clanId,
    people: record.people,
    food: record.food,
    resources: record.resources,
    territory: record.territory,
    stockpiles: record.stockpiles,
    vision: record.vision,
    trees: record.trees,
    dominance: record.dominance,
    foodSecurity: record.foodSecurity,
    kills: record.kills,
    losses: record.losses,
    recruits: record.recruits,
    phase: record.phase,
    action: record.action,
  };
}

function evaluateTournament(message) {
  const options = {
    ...message.options,
    seed: (Number(message.options.seed) || 1) + (message.episodeIndex + 1) * 1_000_003,
  };
  const arena = new LIFE.TrainingArena(options);
  arena.population = message.population.map(data => LIFE.LeaderBrain.fromJSON(data));
  arena.generation = Math.max(0, Number(message.generation) - 1);
  arena.startGeneration();
  arena.withRandom(() => {
    for (let i = 0; i < arena.options.episodeTicks; i++) arena.world.step();
  });
  arena.ticksRun = arena.options.episodeTicks;
  const scores = arena.snapshotScores()
    .sort((a, b) => b.score - a.score)
    .map(scoreSnapshot);

  return {
    generation: message.generation,
    episodeIndex: message.episodeIndex,
    ticksRun: arena.options.episodeTicks,
    scores,
  };
}

parentPort.on("message", message => {
  try {
    parentPort.postMessage({ ok: true, result: evaluateTournament(message) });
  } catch (err) {
    parentPort.postMessage({ ok: false, error: err && err.stack ? err.stack : String(err) });
  }
});
