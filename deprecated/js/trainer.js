// TrainingArena: runs repeated headless tournaments and evolves leader brains.
const TRAINING_DEFAULTS = {
  populationSize: 16,
  episodeTicks: 6000,
  ticksPerFrame: 500,
  worldSize: 160,
  startingFollowers: 4,
  localNeutrals: 6,
  neutralEntities: 24,
  pellets: 0,
  trees: 128,
  resources: 130,
  startingFood: 32,
  startingWood: 12,
  startingStone: 8,
  eliteCount: 3,
  mutationRate: 0.07,
  mutationStrength: 0.28,
  rivalryFriendliness: 0.4,
  rivalryAnimosity: 0.32,
  seed: 7429,
};

class SeededRandom {
  constructor(seed) {
    this.state = seed >>> 0 || 1;
  }

  next() {
    this.state = (this.state * 1664525 + 1013904223) >>> 0;
    return this.state / 4294967296;
  }
}

class TrainingArena {
  constructor(options = {}) {
    this.options = { ...TRAINING_DEFAULTS, ...options };
    this.population = [];
    this.generation = 0;
    this.world = null;
    this.rng = null;
    this.state = "idle";
    this.ticksRun = 0;
    this.latestScores = [];
    this.history = [];
    this.bestBrain = null;
    this.bestScore = 0;
    this.resetPopulation();
  }

  get populationSize() { return this.options.populationSize; }
  get episodeTicks() { return this.options.episodeTicks; }
  get ticksPerFrame() { return this.options.ticksPerFrame; }

  configure(options = {}) {
    const oldPopulationSize = this.options.populationSize;
    this.options = { ...this.options, ...options };
    if (oldPopulationSize !== this.options.populationSize) this.resetPopulation();
  }

  resetPopulation() {
    this.population = [];
    for (let i = 0; i < this.options.populationSize; i++) {
      this.population.push(LeaderBrain.random());
    }
    this.generation = 0;
    this.world = null;
    this.rng = null;
    this.state = "idle";
    this.ticksRun = 0;
    this.latestScores = [];
    this.history = [];
    this.bestBrain = null;
    this.bestScore = 0;
  }

  withRandom(fn) {
    const previousRandom = Math.random;
    Math.random = () => this.rng.next();
    try {
      return fn();
    } finally {
      Math.random = previousRandom;
    }
  }

  startGeneration() {
    this.generation++;
    this.ticksRun = 0;
    this.rng = new SeededRandom(this.options.seed + this.generation * 9973);
    this.world = this.withRandom(() => this.createWorld(this.population));
    this.state = "running";
    this.latestScores = this.snapshotScores();
  }

  advance(ticks) {
    let completed = 0;
    let budget = Math.max(0, Math.floor(ticks));

    while (budget > 0) {
      if (this.state !== "running") this.startGeneration();
      const remaining = this.options.episodeTicks - this.ticksRun;
      const stepCount = Math.min(budget, remaining);

      this.withRandom(() => {
        for (let i = 0; i < stepCount; i++) this.world.step();
      });

      this.ticksRun += stepCount;
      budget -= stepCount;

      if (this.ticksRun >= this.options.episodeTicks) {
        this.finishGeneration();
        completed++;
      }
    }

    return completed;
  }

  runGeneration() {
    this.startGeneration();
    this.advance(this.options.episodeTicks);
  }

  finishGeneration() {
    const scores = this.snapshotScores().sort((a, b) => b.score - a.score);
    this.latestScores = scores;
    const avg = scores.reduce((sum, r) => sum + r.score, 0) / Math.max(1, scores.length);
    const best = scores[0] || null;

    if (best) {
      if (!this.bestBrain || best.score >= this.bestScore) {
        this.bestScore = best.score;
        this.bestBrain = best.brain.clone({ preserveId: true });
      }
      this.history.push({
        generation: this.generation,
        best: best.score,
        avg,
        bestName: best.name,
      });
      if (this.history.length > 80) this.history.shift();
    }

    this.withRandom(() => this.evolve(scores));
    this.state = "idle";
  }

  createWorld(brains) {
    const world = new World(this.options.worldSize);
    world.trainingRoster = brains.map(brain => ({ brainId: brain.id, name: brain.name, generation: brain.generation }));

    for (let i = 0; i < this.options.pellets; i++) {
      const { x, y } = world.randomCell();
      world.addPellet(new Pellet(x, y));
    }
    for (let i = 0; i < this.options.trees; i++) {
      const { x, y } = world.randomCell();
      world.addTree(new Tree(x, y));
    }
    for (let i = 0; i < this.options.resources; i++) {
      const { x, y } = world.randomCell();
      world.addResource(new Resource(x, y, Math.random() < 0.55 ? "wood" : "stone"));
    }

    const center = (world.size - 1) / 2;
    const ring = world.size * 0.34;
    for (let i = 0; i < brains.length; i++) {
      const angle = (i / brains.length) * Math.PI * 2;
      const jitter = () => Math.floor(Math.random() * 9) - 4;
      const x = world.clamp(Math.round(center + Math.cos(angle) * ring) + jitter());
      const y = world.clamp(Math.round(center + Math.sin(angle) * ring) + jitter());

      this.seedLocalFood(world, x, y);
      const leader = new Entity(x, y, true);
      leader.brain = brains[i].clone({ preserveId: true });
      world.addEntity(leader);
      const clan = leader.clan;

      for (let n = 0; n < this.options.startingFollowers; n++) {
        const pos = this.near(world, x, y, 3);
        const follower = new Entity(pos.x, pos.y, false);
        world.addEntity(follower);
        clan.addMember(follower, "seed");
      }
      for (let n = 0; n < this.options.localNeutrals; n++) {
        const pos = this.near(world, x, y, 8);
        world.addEntity(new Entity(pos.x, pos.y, false));
      }
      if (!clan.isTown && clan.members.length >= TOWN_FOUNDING_FOLLOWERS) clan.foundTown(world);
      const home = clan.stockpiles[0];
      if (home) {
        home.food += this.options.startingFood;
        home.resources.wood = (home.resources.wood || 0) + this.options.startingWood;
        home.resources.stone = (home.resources.stone || 0) + this.options.startingStone;
      }
    }

    for (let i = 0; i < this.options.neutralEntities; i++) {
      const { x, y } = world.randomCell();
      world.addEntity(new Entity(x, y, false));
    }

    this.seedRivalries(world);
    return world;
  }

  seedRivalries(world) {
    for (const clan of world.clans) {
      clan.ensureRelationships(world);
      for (const other of world.clans) {
        if (other === clan) continue;
        const rel = clan.relationshipWithClan(other);
        rel.friendliness = this.options.rivalryFriendliness;
        rel.animosity = this.options.rivalryAnimosity;
      }
    }
  }

  seedLocalFood(world, cx, cy) {
    for (let i = 0; i < 6; i++) {
      const pos = this.near(world, cx, cy, INITIAL_CLAIM_RADIUS);
      world.addTree(new Tree(pos.x, pos.y, { lastSpawnTick: -TREE_DEFAULT_INTERVAL }));
    }
    for (let i = 0; i < 6; i++) {
      const pos = this.near(world, cx, cy, 18);
      world.addResource(new Resource(pos.x, pos.y, Math.random() < 0.5 ? "wood" : "stone"));
    }
  }

  near(world, cx, cy, radius) {
    return {
      x: world.clamp(cx + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
      y: world.clamp(cy + Math.floor(Math.random() * (radius * 2 + 1)) - radius),
    };
  }

  snapshotScores() {
    if (!this.world) {
      return this.population.map(brain => ({
        brainId: brain.id,
        name: brain.name,
        generation: brain.generation,
        brain,
        score: brain.lastScore || 0,
        alive: false,
        clanId: null,
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
        action: "not run",
      }));
    }

    const byBrain = new Map();
    for (const brain of this.population) {
      byBrain.set(brain.id, {
        brainId: brain.id,
        name: brain.name,
        generation: brain.generation,
        brain,
        score: 0,
        alive: false,
        clanId: null,
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
      });
    }

    const liveClans = this.world.clans.filter(c => !c.disbanded && c.leader && !c.leader.dead);
    const initialClans = Math.max(1, this.world.trainingRoster ? this.world.trainingRoster.length : this.population.length);
    const totalPeople = liveClans.reduce((sum, c) => sum + c.everyone().filter(e => !e.dead).length, 0);
    const totalFood = liveClans.reduce((sum, c) => sum + c.totalFood(), 0);
    const totalResources = liveClans.reduce((sum, c) => sum + c.totalResources(), 0);
    const totalTerritory = liveClans.reduce((sum, c) => sum + c.claimed.size, 0);
    const totalOwnedTrees = Math.max(1, liveClans.reduce((sum, c) => sum + c.ownedTreeCount(this.world), 0));
    const phaseBonus = { band: 0, village: 80, town: 220, city: 420 };

    for (const clan of this.world.clans) {
      const record = byBrain.get(clan.brain.id);
      if (!record) continue;
      const people = clan.everyone().filter(e => !e.dead).length;
      const food = clan.totalFood();
      const resources = clan.totalResources();
      const stockpiles = clan.stockpiles.length;
      const territory = clan.claimed.size;
      const vision = clan.vision.size;
      const trees = clan.ownedTreeCount(this.world);
      const action = clan.lastDecision ? clan.lastDecision.action : "thinking";
      const rivalsAlive = liveClans.filter(c => c !== clan).length;
      const eliminatedRivals = Math.max(0, initialClans - 1 - rivalsAlive);
      const dominance =
        (totalPeople ? people / totalPeople : 0) * 0.42 +
        (totalTerritory ? territory / totalTerritory : 0) * 0.25 +
        (totalFood ? food / totalFood : 0) * 0.13 +
        (totalResources ? resources / totalResources : 0) * 0.1 +
        (trees / totalOwnedTrees) * 0.1;
      const hunger = clan.everyone().reduce((sum, e) => sum + brainClamp01(e.hunger), 0) / Math.max(1, people);
      const foodSecurity = brainClamp01((food + trees * 12) / Math.max(1, people * 10));
      const netKills = Math.max(0, clan.stats.kills - clan.stats.losses);
      const phase = clan.phase(this.world);
      const stableDominance = dominance * (0.35 + foodSecurity * 0.65);
      const stableEliminations = foodSecurity >= 0.45 ? eliminatedRivals : 0;
      let score =
        people * 55 +
        (clan.stats.peakPeople || people) * 16 +
        (clan.leader && !clan.leader.dead ? 180 : 0) +
        (clan.isTown ? 160 : 0) +
        (phaseBonus[phase] || 0) +
        stockpiles * 90 +
        Math.min(food, people * 14) * 5 +
        Math.max(0, food - people * 14) * 1 +
        resources * 3.5 +
        territory * 1.1 +
        trees * 120 +
        vision * 0.035 +
        clan.stats.recruits * 45 +
        stableEliminations * 300 +
        stableDominance * 900 +
        netKills * 70 +
        clan.stats.kills * 15 -
        clan.stats.losses * 120 -
        hunger * 400 -
        (foodSecurity < 0.25 ? 700 : 0) -
        (foodSecurity < 0.5 ? 250 : 0);
      if (foodSecurity < 0.25) score *= 0.35;
      else if (foodSecurity < 0.5) score *= 0.7;

      record.brain = clan.brain;
      record.score = Math.round(score * 10) / 10;
      record.alive = !!(clan.leader && !clan.leader.dead);
      record.clanId = clan.id;
      record.people = people;
      record.food = food;
      record.resources = resources;
      record.territory = territory;
      record.stockpiles = stockpiles;
      record.vision = vision;
      record.trees = trees;
      record.dominance = Math.round(dominance * 1000) / 1000;
      record.foodSecurity = Math.round(foodSecurity * 1000) / 1000;
      record.kills = clan.stats.kills;
      record.losses = clan.stats.losses;
      record.recruits = clan.stats.recruits;
      record.phase = phase;
      record.action = action;
      clan.brain.lastScore = record.score;

      const populationBrain = this.population.find(b => b.id === clan.brain.id);
      if (populationBrain) {
        populationBrain.lastScore = record.score;
        populationBrain.lastDecision = clan.brain.lastDecision;
      }
    }

    return [...byBrain.values()].sort((a, b) => b.score - a.score);
  }

  evolve(scores) {
    if (!scores.length) return;
    const ranked = scores.slice().sort((a, b) => b.score - a.score);
    const eliteCount = Math.max(1, Math.min(this.options.eliteCount, ranked.length));
    const parentCount = Math.max(eliteCount, Math.ceil(ranked.length / 2));
    const parents = ranked.slice(0, parentCount)
      .map(r => this.population.find(b => b.id === r.brainId))
      .filter(Boolean);
    const next = [];

    for (let i = 0; i < eliteCount; i++) {
      const elite = this.population.find(b => b.id === ranked[i].brainId);
      if (elite) next.push(elite.clone({ preserveId: true }));
    }

    while (next.length < this.options.populationSize) {
      const a = this.weightedParent(parents);
      const b = this.weightedParent(parents);
      const child = a.crossover(b).mutate(this.options.mutationRate, this.options.mutationStrength);
      next.push(child);
    }

    this.population = next;
  }

  weightedParent(parents) {
    if (parents.length === 1) return parents[0];
    const index = Math.min(parents.length - 1, Math.floor(Math.random() ** 2 * parents.length));
    return parents[index];
  }

  champion() {
    if (this.bestBrain) return this.bestBrain.clone({ preserveId: true });
    const scored = this.population.slice().sort((a, b) => (b.lastScore || 0) - (a.lastScore || 0));
    return scored[0] ? scored[0].clone({ preserveId: true }) : LeaderBrain.random();
  }
}
