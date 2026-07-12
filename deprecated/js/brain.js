// LeaderBrain: a tiny feed-forward policy network used by clan leaders.
// It maps town state -> order scores, then Clan.leaderThink applies the best
// valid order to the existing task system.
const LEADER_BRAIN_INPUTS = [
  { key: "people", label: "people" },
  { key: "followers", label: "followers" },
  { key: "food", label: "food reserve" },
  { key: "foodSecurity", label: "food security" },
  { key: "resources", label: "resource reserve" },
  { key: "territory", label: "territory" },
  { key: "stockpiles", label: "stockpiles" },
  { key: "phase", label: "village phase" },
  { key: "hunger", label: "hunger pressure" },
  { key: "foodNearby", label: "food nearby" },
  { key: "resourceNearby", label: "resource nearby" },
  { key: "enemyNearby", label: "enemy nearby" },
  { key: "neutralNearby", label: "neutral nearby" },
  { key: "contestedFood", label: "contested food" },
  { key: "vision", label: "vision" },
  { key: "trees", label: "owned trees" },
  { key: "dominance", label: "dominance" },
  { key: "militaryBalance", label: "military balance" },
  { key: "neutralFriendly", label: "neutral friendly" },
  { key: "neutralAnimosity", label: "neutral animosity" },
  { key: "pendingOrders", label: "pending orders" },
  { key: "expansionReady", label: "expansion ready" },
  { key: "timeCycle", label: "time cycle" },
];

const LEADER_BRAIN_OUTPUTS = [
  { key: "claim", label: "claim land" },
  { key: "defend", label: "defend" },
  { key: "harvest", label: "harvest" },
  { key: "expand", label: "expand" },
  { key: "recruit", label: "recruit" },
  { key: "scout", label: "scout" },
  { key: "fight", label: "fight entity" },
  { key: "fightArea", label: "fight area" },
  { key: "hunt", label: "hunt" },
  { key: "store", label: "store" },
  { key: "consolidate", label: "consolidate" },
  { key: "reserve", label: "reserve food" },
  { key: "friendliness", label: "friendliness" },
  { key: "animosity", label: "animosity" },
];

const LEADER_BRAIN_ACTIONS = [
  "claim",
  "defend",
  "harvest",
  "expand",
  "recruit",
  "scout",
  "fight",
  "fightArea",
  "hunt",
  "store",
  "consolidate",
];

function brainClamp01(v) {
  if (!isFinite(v)) return 0;
  return Math.max(0, Math.min(1, v));
}

function brainClampWeight(v) {
  return Math.max(-4, Math.min(4, v));
}

function brainSigmoid(v) {
  return 1 / (1 + Math.exp(-v));
}

function brainRandomWeight() {
  return Math.random() * 2 - 1;
}

function brainRandomMatrix(rows, cols) {
  const matrix = [];
  for (let y = 0; y < rows; y++) {
    const row = [];
    for (let x = 0; x < cols; x++) row.push(brainRandomWeight());
    matrix.push(row);
  }
  return matrix;
}

function brainResizeMatrix(matrix, rows, cols) {
  const out = [];
  for (let y = 0; y < rows; y++) {
    const source = matrix && matrix[y] ? matrix[y] : [];
    const row = [];
    for (let x = 0; x < cols; x++) {
      row.push(source[x] !== undefined ? source[x] : brainRandomWeight());
    }
    out.push(row);
  }
  return out;
}

function brainResizeArray(values, length) {
  const out = [];
  for (let i = 0; i < length; i++) {
    out.push(values && values[i] !== undefined ? values[i] : brainRandomWeight());
  }
  return out;
}

function brainCloneMatrix(matrix) {
  return matrix.map(row => row.slice());
}

function brainGaussian() {
  const u = Math.max(Number.EPSILON, Math.random());
  const v = Math.max(Number.EPSILON, Math.random());
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}

function brainDistanceSignal(world, x, y, target) {
  if (!target) return 0;
  const dist = Math.sqrt((target.x - x) ** 2 + (target.y - y) ** 2);
  return brainClamp01(1 - dist / Math.max(1, world.size * 0.55));
}

class LeaderBrain {
  static nextId = 1;
  static hiddenCount = 10;

  constructor(data = {}) {
    this.id = data.id !== undefined ? data.id : LeaderBrain.nextId++;
    LeaderBrain.nextId = Math.max(LeaderBrain.nextId, this.id + 1);
    this.name = data.name || `B-${String(this.id).padStart(3, "0")}`;
    this.generation = data.generation || 0;
    this.parents = (data.parents || []).slice();
    this.lastScore = data.lastScore || 0;

    const inputCount = LEADER_BRAIN_INPUTS.length;
    const hiddenCount = data.hiddenCount || LeaderBrain.hiddenCount;
    const outputCount = LEADER_BRAIN_OUTPUTS.length;

    this.inputWeights = data.inputWeights
      ? brainResizeMatrix(data.inputWeights, hiddenCount, inputCount)
      : brainRandomMatrix(hiddenCount, inputCount);
    this.hiddenBias = data.hiddenBias
      ? brainResizeArray(data.hiddenBias, hiddenCount)
      : Array.from({ length: hiddenCount }, brainRandomWeight);
    this.outputWeights = data.outputWeights
      ? brainResizeMatrix(data.outputWeights, outputCount, hiddenCount)
      : brainRandomMatrix(outputCount, hiddenCount);
    this.outputBias = data.outputBias
      ? brainResizeArray(data.outputBias, outputCount)
      : Array.from({ length: outputCount }, brainRandomWeight);

    this.lastDecision = null;
  }

  static random() {
    return new LeaderBrain();
  }

  static fromJSON(data) {
    return data ? new LeaderBrain(data) : LeaderBrain.random();
  }

  serialize() {
    return {
      id: this.id,
      name: this.name,
      generation: this.generation,
      parents: this.parents.slice(),
      lastScore: this.lastScore,
      hiddenCount: this.inputWeights.length,
      inputWeights: brainCloneMatrix(this.inputWeights),
      hiddenBias: this.hiddenBias.slice(),
      outputWeights: brainCloneMatrix(this.outputWeights),
      outputBias: this.outputBias.slice(),
    };
  }

  clone(options = {}) {
    const data = this.serialize();
    if (!options.preserveId) {
      delete data.id;
      data.name = options.name || `${this.name} copy`;
      data.parents = [this.id];
    }
    if (options.name) data.name = options.name;
    const cloned = new LeaderBrain(data);
    cloned.lastDecision = this.lastDecision ? JSON.parse(JSON.stringify(this.lastDecision)) : null;
    return cloned;
  }

  crossover(other) {
    const data = this.serialize();
    delete data.id;
    delete data.name;
    data.generation = Math.max(this.generation, other.generation) + 1;
    data.parents = [this.id, other.id];
    data.lastScore = 0;

    for (let y = 0; y < data.inputWeights.length; y++) {
      for (let x = 0; x < data.inputWeights[y].length; x++) {
        data.inputWeights[y][x] = Math.random() < 0.5
          ? this.inputWeights[y][x]
          : other.inputWeights[y][x];
      }
    }
    for (let i = 0; i < data.hiddenBias.length; i++) {
      data.hiddenBias[i] = Math.random() < 0.5 ? this.hiddenBias[i] : other.hiddenBias[i];
    }
    for (let y = 0; y < data.outputWeights.length; y++) {
      for (let x = 0; x < data.outputWeights[y].length; x++) {
        data.outputWeights[y][x] = Math.random() < 0.5
          ? this.outputWeights[y][x]
          : other.outputWeights[y][x];
      }
    }
    for (let i = 0; i < data.outputBias.length; i++) {
      data.outputBias[i] = Math.random() < 0.5 ? this.outputBias[i] : other.outputBias[i];
    }

    return new LeaderBrain(data);
  }

  mutate(rate = 0.08, strength = 0.35) {
    const mutateValue = value => {
      if (Math.random() > rate) return value;
      return brainClampWeight(value + brainGaussian() * strength);
    };

    for (const row of this.inputWeights) {
      for (let i = 0; i < row.length; i++) row[i] = mutateValue(row[i]);
    }
    for (let i = 0; i < this.hiddenBias.length; i++) this.hiddenBias[i] = mutateValue(this.hiddenBias[i]);
    for (const row of this.outputWeights) {
      for (let i = 0; i < row.length; i++) row[i] = mutateValue(row[i]);
    }
    for (let i = 0; i < this.outputBias.length; i++) this.outputBias[i] = mutateValue(this.outputBias[i]);
    return this;
  }

  evaluate(inputs) {
    const hidden = this.inputWeights.map((row, i) => {
      let sum = this.hiddenBias[i] || 0;
      for (let x = 0; x < row.length; x++) sum += row[x] * (inputs[x] || 0);
      return Math.tanh(sum);
    });

    const values = {};
    for (let y = 0; y < this.outputWeights.length; y++) {
      let sum = this.outputBias[y] || 0;
      for (let x = 0; x < this.outputWeights[y].length; x++) sum += this.outputWeights[y][x] * hidden[x];
      values[LEADER_BRAIN_OUTPUTS[y].key] = brainSigmoid(sum);
    }
    return values;
  }

  decide(clan, world) {
    const inputs = LeaderBrain.sense(clan, world);
    const outputs = this.evaluate(inputs.values);
    const ranked = LEADER_BRAIN_OUTPUTS
      .map(o => ({ key: o.key, label: o.label, value: outputs[o.key] || 0 }))
      .sort((a, b) => b.value - a.value);

    const decision = {
      tick: world.tick,
      inputs,
      outputs,
      ranked,
      action: ranked[0] ? ranked[0].key : "wait",
      confidence: ranked[0] ? ranked[0].value : 0,
    };
    this.lastDecision = decision;
    return decision;
  }

  static sense(clan, world) {
    const peopleList = clan.everyone().filter(e => !e.dead);
    const people = Math.max(1, peopleList.length);
    const leader = clan.leader;
    const foodStored = clan.stockpiles.reduce((sum, s) => sum + s.food, 0);
    const foodCarried = peopleList.reduce((sum, e) => sum + e.food, 0);
    const resources = clan.stockpiles.reduce((sum, s) => sum + s.totalResources(), 0) +
      peopleList.reduce((sum, e) => sum + e.carriedResources(), 0);
    const hunger = peopleList.reduce((sum, e) => sum + brainClamp01(e.hunger), 0) / people;
    const nearestFood = world.nearestGatherablePellet(leader) || world.nearestVisiblePellet(leader);
    const nearestResource = world.nearestResource(leader.x, leader.y);
    const visibleNeutral = world.visibleEntitiesForClan(clan, e => !e.clan && !e.isLeader).length;
    const visibleEnemies = world.visibleEntitiesForClan(clan, e => clan.isHostileToEntity(e)).length;
    const contestedFood = world.countContestedFoodForClan(clan);
    const ownedTrees = world.trees.filter(t => !t.destroyed && clan.claimed.has(world.key(t.x, t.y))).length;
    const visibleTrees = world.trees.filter(t => !t.destroyed && clan.canSee(world, t.x, t.y)).length;
    const neutralRel = clan.relationshipWithNeutral();
    const clans = world.clans.filter(c => !c.disbanded);
    const totalPeople = clans.reduce((sum, c) => sum + c.everyone().filter(e => !e.dead).length, 0);
    const totalTerritory = clans.reduce((sum, c) => sum + c.claimed.size, 0);
    const totalTrees = world.trees.filter(t => !t.destroyed).length;
    const largestRivalPeople = clans
      .filter(c => c !== clan)
      .reduce((max, c) => Math.max(max, c.everyone().filter(e => !e.dead).length), 0);
    const dominance =
      (totalPeople ? people / totalPeople : 0) * 0.45 +
      (totalTerritory ? clan.claimed.size / totalTerritory : 0) * 0.35 +
      (totalTrees ? ownedTrees / totalTrees : 0) * 0.2;
    const phaseScores = { band: 0, village: 0.35, town: 0.7, city: 1 };

    let enemyNearby = 0;
    for (const e of world.visibleEntitiesForClan(clan, e => clan.isHostileToEntity(e))) {
      const dist = Math.max(Math.abs(e.x - leader.x), Math.abs(e.y - leader.y));
      if (dist <= VISION_RADIUS) enemyNearby++;
    }

    const pendingOrders = clan.orders.filter(o => o.type !== "collect" && !o.done).length;
    const home = clan.stockpiles[0];
    const expansionReady = home && clan.members.length >= 5 && home.food >= 12 && clan.stockpiles.length < 4;
    const claimedGoal = people * 8;
    const valuesByKey = {
      people: brainClamp01(people / 24),
      followers: brainClamp01(clan.members.length / 20),
      food: brainClamp01((foodStored + foodCarried) / (people * 6)),
      foodSecurity: brainClamp01((foodStored + foodCarried + ownedTrees * 12) / (people * 10)),
      resources: brainClamp01(resources / (people * 4)),
      territory: brainClamp01(clan.claimed.size / Math.max(1, claimedGoal)),
      stockpiles: brainClamp01(clan.stockpiles.length / 4),
      phase: phaseScores[clan.phase(world)] || 0,
      hunger: brainClamp01(hunger),
      foodNearby: brainDistanceSignal(world, leader.x, leader.y, nearestFood),
      resourceNearby: brainDistanceSignal(world, leader.x, leader.y, nearestResource),
      enemyNearby: brainClamp01((enemyNearby + visibleEnemies * 0.5) / Math.max(1, people * 2)),
      neutralNearby: brainClamp01(visibleNeutral / Math.max(1, people * 2)),
      contestedFood: brainClamp01(contestedFood / Math.max(1, people)),
      vision: brainClamp01((clan.vision.size + clan.claimed.size) / Math.max(1, people * VISION_RADIUS * VISION_RADIUS)),
      trees: brainClamp01((ownedTrees + visibleTrees * 0.35) / Math.max(1, people)),
      dominance: brainClamp01(dominance),
      militaryBalance: brainClamp01(people / Math.max(1, people + largestRivalPeople)),
      neutralFriendly: brainClamp01(neutralRel.friendliness),
      neutralAnimosity: brainClamp01(neutralRel.animosity),
      pendingOrders: brainClamp01(pendingOrders / 6),
      expansionReady: expansionReady ? 1 : 0,
      timeCycle: (Math.sin(world.tick / 500) + 1) / 2,
    };

    return {
      keys: LEADER_BRAIN_INPUTS.map(i => i.key),
      values: LEADER_BRAIN_INPUTS.map(i => valuesByKey[i.key]),
      byKey: valuesByKey,
    };
  }
}
