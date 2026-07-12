// Headless smoke test: runs the sim in Node (no DOM) and checks the new
// behaviors actually happen. Run with: node test/smoke.js
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const ctx = vm.createContext({ Math, console });
for (const f of ["pellet.js", "tree.js", "resource.js", "stockpile.js", "task.js", "brain.js", "clan.js", "entity.js", "world.js", "trainer.js"]) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, "..", "js", f), "utf8"), ctx, { filename: f });
}

const result = vm.runInContext(`
  const world = new World(256);

  for (let i = 0; i < 80; i++) {
    const { x, y } = world.randomCell();
    world.addTree(new Tree(x, y, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
  }
  for (let i = 0; i < 100; i++) {
    const { x, y } = world.randomCell();
    world.addResource(new Resource(x, y, Math.random() < 0.5 ? "wood" : "stone"));
  }
  const starts = [
    { x: 48, y: 48 },
    { x: 132, y: 132 },
    { x: 204, y: 58 },
  ];
  for (const start of starts) {
    const leader = new Entity(start.x, start.y, true);
    world.addEntity(leader);
    for (let i = 0; i < 7; i++) {
      const follower = new Entity(start.x + (i % 3) - 1, start.y + Math.floor(i / 3) - 1, false);
      world.addEntity(follower);
      leader.clan.addMember(follower, "seed");
    }
    leader.clan.foundTown(world);
    world.addTree(new Tree(start.x + 1, start.y + 1, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
    world.addTree(new Tree(start.x - 1, start.y - 1, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
    world.addTree(new Tree(start.x + 2, start.y - 1, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
    world.addTree(new Tree(start.x - 2, start.y + 1, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
  }
  for (let i = 0; i < 170; i++) {
    const { x, y } = world.randomCell();
    world.addEntity(new Entity(x, y, false));
  }

  const startLeaders = world.entities.filter(e => e.isLeader).length;
  const speeds = world.entities.map(e => e.speed);
  const speedRangeOk = speeds.every(s => s >= 0.25 && s <= 0.5);
  const healthOk =
    world.entities.filter(e => e.isLeader).every(e => e.health === 15 && e.maxHealth === 15) &&
    world.entities.filter(e => !e.isLeader).every(e => e.health === 10 && e.maxHealth === 10);

  const graceWorld = new World(32);
  const graceA = new Entity(5, 5, true);
  const graceB = new Entity(6, 5, true);
  graceWorld.addEntity(graceA);
  graceWorld.addEntity(graceB);
  const graceBefore = graceB.health;
  const blockedDuringGrace = graceA.attack(graceB, graceWorld) === false && graceB.health === graceBefore;
  graceWorld.tick = CLAN_GRACE_TICKS;
  graceA.attackCooldown = 0;
  const allowedAfterGrace = graceA.attack(graceB, graceWorld) === true && graceB.health < graceBefore;
  const graceOk = blockedDuringGrace && allowedAfterGrace;

  // track one entity's movement rate to confirm the cost works
  const probe = world.entities[10];
  let probeMoves = 0;
  let probeAliveTicks = 0;
  let probePrev = { x: probe.x, y: probe.y };

  let died = 0;
  let prevCount = world.entities.length;
  let maxTowns = 0;
  let maxClanMembers = 0;
  let peakStockpile = 0;
  let maxStockpiles = 0;
  let peakClaimed = 0;
  let peakStoredResources = 0;
  let healthEverInvalid = false;
  let brainDecisionSeen = false;
  let successionOk = false;
  let treePelletsSeen = false;
  const initialClaimOk = world.clans.every(c => c.claimed.size >= 25);
  const claimBefore = world.clans[1].claimed.size;
  world.clans[1].claimArea(world, 250, 250, 0);
  const adjacentClaimOk = world.clans[1].claimed.size === claimBefore;
  const orderTypesSeen = new Set();
  let reloadOk = null;

  const successionClan = world.clans[0];
  const oldLeaderId = successionClan.leader.id;
  successionClan.leader.dead = true;
  world.step();
  successionOk = successionClan.leader.id !== oldLeaderId && successionClan.leader.isLeader && !successionClan.disbanded;

  for (let t = 0; t < 6000; t++) {
    world.step();
    if (!probe.dead) {
      probeAliveTicks++;
      if (probe.x !== probePrev.x || probe.y !== probePrev.y) probeMoves++;
      probePrev = { x: probe.x, y: probe.y };
    }
    died += prevCount - world.entities.length;
    prevCount = world.entities.length;

    maxTowns = Math.max(maxTowns, world.clans.filter(c => c.isTown).length);
    treePelletsSeen = treePelletsSeen || world.pellets.size > 0;
    maxClanMembers = Math.max(maxClanMembers, ...world.clans.map(c => c.members.length), 0);
    for (const c of world.clans) {
      maxStockpiles = Math.max(maxStockpiles, c.stockpiles.length);
      peakClaimed = Math.max(peakClaimed, c.claimed.size);
      if (c.lastDecision && c.lastDecision.outputs && c.brain) brainDecisionSeen = true;
      for (const o of c.orders) orderTypesSeen.add(o.type);
      for (const s of c.stockpiles) {
        peakStockpile = Math.max(peakStockpile, s.food);
        peakStoredResources = Math.max(peakStoredResources, s.totalResources());
      }
    }
    for (const e of world.entities) {
      if (e.health <= 0 || e.health > e.maxHealth) healthEverInvalid = true;
    }

    // round-trip the save format mid-run, while the world is busy
    if (t === 1500) {
      const snap = JSON.stringify(world.serialize());
      const w2 = new World(256);
      w2.deserialize(JSON.parse(snap));
      w2.step();
      reloadOk = w2.entities.length > 0 &&
        w2.clans.length === world.clans.length &&
        w2.entities.every(e => e.speed >= 0.25 && e.speed <= 0.5) &&
        w2.clans.every(c => c.brain && c.brain.inputWeights.length > 0 && c.relationships.neutral) &&
        w2.trees.length > 0;
    }
  }

  const arena = new TrainingArena({
    populationSize: 6,
    episodeTicks: 800,
    ticksPerFrame: 200,
    worldSize: 96,
    trees: 30,
    resources: 40,
    neutralEntities: 6,
  });
  arena.runGeneration();
  arena.runGeneration();
  const trainingOk =
    arena.generation === 2 &&
    arena.history.length === 2 &&
    arena.latestScores.length === 6 &&
    arena.bestBrain &&
    arena.latestScores[0].score > 0;

  const towns = world.clans.filter(c => c.isTown);
  ({
    startLeaders,
    speedRangeOk,
    healthOk,
    graceOk,
    healthEverInvalid,
    probeSpeed: Number(probe.speed.toFixed(3)),
    probeMovesPerAliveTick: Number((probeMoves / Math.max(1, probeAliveTicks)).toFixed(3)),
    maxTowns,
    maxClanMembers,
    maxStockpiles,
    peakStockpile,
    peakStoredResources,
    peakClaimed,
    brainDecisionSeen,
    successionOk,
    treePelletsSeen,
    initialClaimOk,
    adjacentClaimOk,
    orderTypesSeen: [...orderTypesSeen].sort(),
    endTowns: towns.length,
    endClans: world.clans.length,
    died,
    alive: world.entities.length,
    pelletsLeft: world.pellets.size,
    reloadOk,
    trainingOk,
  });
`, ctx);

console.log(JSON.stringify(result, null, 2));
