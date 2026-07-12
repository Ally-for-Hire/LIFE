// Clan: a leader and their followers, sharing a color. With at least one
// stockpile the clan is a town: the leader issues orders (Tasks) and members
// are assigned to them, falling back to the standing "collect" job.
const CLAN_RECRUIT_RADIUS = 3;      // entities join when within 3 cells of the leader
const TOWN_FOUNDING_FOLLOWERS = 3;  // followers needed before the first stockpile
const STOCKPILE_PLACE_RADIUS = 5;   // first stockpile lands within this range of the leader
const INITIAL_CLAIM_RADIUS = 2;     // first stockpile seeds a contiguous 5x5 claim
const LEADER_THINK_INTERVAL = 200;  // ticks between leader decisions
const REL_NEUTRAL = "neutral";
const FOOD_CRISIS_PER_PERSON = 2;
const CLAN_STATS_INTERVAL = 100;
const CLAN_RECRUIT_INTERVAL = 20;
const CLAN_ORDER_INTERVAL = 20;
const ORDER_MAX_AGE = {
  claim: 900,
  recruit: 900,
  scout: 1200,
  defend: 1400,
  fightEntity: 900,
  fightArea: 1000,
  hunt: 1200,
  moveBuilding: 1200,
  harvest: 1800,
  store: 600,
  putResources: 1000,
  putPellets: 1200,
};

class Clan {
  static nextId = 1;

  constructor(leader, color, brain) {
    this.id = Clan.nextId++;
    this.leader = leader;
    this.color = color;
    this.brain = brain || leader.brain || LeaderBrain.random();
    this.leader.brain = this.brain;
    this.lastDecision = null;
    this.members = [];       // followers; the leader is not in this list
    this.stockpiles = [];    // one or more once the clan is a town
    this.claimed = new Set();// "x,y" cells claimed as town territory
    this.vision = new Map(); // "x,y" -> expiry tick; claimed cells are always visible
    this.relationships = {}; // "neutral" or "clan:id" -> { friendliness, animosity }
    this.orders = [];        // standing + one-shot Tasks issued by the leader
    this.minPerPerson = 0;   // food each townsperson keeps; set by the brain
    this.stats = {
      foundedTick: null,
      recruits: 0,
      kills: 0,
      losses: 0,
      peakPeople: 1,
      peakFood: 0,
      peakResources: 0,
      peakTerritory: 0,
      peakStockpiles: 0,
      peakTrees: 0,
    };
    this.disbanded = false;
  }

  get isTown() {
    return this.stockpiles.length > 0;
  }

  everyone() {
    return [this.leader, ...this.members];
  }

  phase(world) {
    if (!this.isTown) return "band";
    const people = this.everyone().filter(e => !e.dead).length;
    const resources = this.totalResources();
    const trees = world ? this.ownedTreeCount(world) : this.stats.peakTrees;
    if (people >= 18 && this.stockpiles.length >= 4 && resources >= 140 && trees >= 5) return "city";
    if (people >= 10 && this.stockpiles.length >= 2 && resources >= 55 && trees >= 3) return "town";
    return "village";
  }

  update(world) {
    if (this.disbanded) return;
    this.ensureRelationships(world);
    if (world.tick % CLAN_RECRUIT_INTERVAL === 0) this.recruit(world);
    if (!this.isTown && this.members.length >= TOWN_FOUNDING_FOLLOWERS) {
      this.foundTown(world);
    }
    if (!this.isTown) {
      if (world.tick % LEADER_THINK_INTERVAL === 0) this.leaderThink(world);
      if (world.tick % CLAN_ORDER_INTERVAL === 0) {
        this.pruneOrders(world);
        this.assignTasks();
      }
      if (world.tick % CLAN_STATS_INTERVAL === 0) this.recordStats(world);
      return;
    }
    if (this.isTown) {
      if (world.tick % LEADER_THINK_INTERVAL === 0) this.leaderThink(world);
      if (world.tick % CLAN_ORDER_INTERVAL === 0) {
        this.pruneOrders(world);
        this.assignTasks();
      }
    }
    if (world.tick % CLAN_STATS_INTERVAL === 0) this.recordStats(world);
  }

  recruit(world) {
    for (const e of world.entities) {
      if (e.clan || e.isLeader || e.dead) continue;
      const dist = Math.max(Math.abs(e.x - this.leader.x), Math.abs(e.y - this.leader.y));
      if (dist <= CLAN_RECRUIT_RADIUS) this.addMember(e);
    }
  }

  addMember(entity, source = "recruit") {
    entity.clan = this;
    entity.claimed.clear();
    this.members.push(entity);
    if (source !== "seed") this.stats.recruits++;
  }

  removeMember(entity) {
    this.members = this.members.filter(m => m !== entity);
    entity.clan = null;
    entity.task = null;
  }

  // --- town founding ---
  foundTown(world) {
    const spot = this.findStockpileSpot(world);
    this.stockpiles.push(new Stockpile(spot.x, spot.y));
    this.claimArea(world, spot.x, spot.y, INITIAL_CLAIM_RADIUS);
    this.rollMinPerPerson();
    if (this.stats.foundedTick === null) this.stats.foundedTick = world.tick;
    this.issueOrder("collect"); // the standing default job
    this.recordStats(world);
  }

  findStockpileSpot(world) {
    if (!world.resourceAt(this.leader.x, this.leader.y) && !world.clanWithStockpileAt(this.leader.x, this.leader.y)) {
      return { x: this.leader.x, y: this.leader.y };
    }
    for (let i = 0; i < 20; i++) {
      const x = world.clamp(this.leader.x + Math.floor(Math.random() * (STOCKPILE_PLACE_RADIUS * 2 + 1)) - STOCKPILE_PLACE_RADIUS);
      const y = world.clamp(this.leader.y + Math.floor(Math.random() * (STOCKPILE_PLACE_RADIUS * 2 + 1)) - STOCKPILE_PLACE_RADIUS);
      if (!world.resourceAt(x, y) && !world.clanWithStockpileAt(x, y)) return { x, y };
    }
    return { x: this.leader.x, y: this.leader.y };
  }

  // --- orders ---
  issueOrder(type, params = {}) {
    const task = new Task(type, params);
    task.params.startedTick = task.params.startedTick || 0;
    this.orders.push(task);
    return task;
  }

  workersOf(order) {
    return this.everyone().filter(e => e.task === order).length;
  }

  // The leader's decision pass: sense the town, run the policy network, then
  // issue the highest-scoring valid order.
  leaderThink(world) {
    const home = this.stockpiles[0];

    const decision = this.brain.decide(this, world);
    this.lastDecision = decision;
    this.minPerPerson = decision.outputs.reserve >= 0.55 ? 1 : 0;
    this.adjustRelationships(world, decision.outputs);

    if (!home) {
      const recruitScore = decision.outputs.recruit || 0;
      let orderType = recruitScore >= 0.2 ? this.tryBrainAction(world, "recruit", recruitScore, decision.outputs) : null;
      let action = orderType ? "recruit" : null;
      let score = orderType ? recruitScore : 0;
      if (!orderType) {
        const scoutScore = Math.max(0.35, decision.outputs.scout || 0);
        orderType = this.tryBrainAction(world, "scout", scoutScore, decision.outputs);
        action = orderType ? "scout" : "wait";
        score = orderType ? scoutScore : 0;
      }
      decision.action = action || "wait";
      decision.reason = orderType ? "issued" : "no town yet";
      decision.orderType = orderType;
      decision.score = score;
      this.brain.lastDecision = decision;
      return;
    }

    const issued = this.issueBrainOrder(world, decision);
    decision.action = issued.action;
    decision.reason = issued.reason;
    decision.orderType = issued.orderType || null;
    decision.score = issued.score || 0;
    this.brain.lastDecision = decision;
  }

  hasOpenOrder(type) {
    return this.orders.some(o => o.type === type && !o.done);
  }

  issueBrainOrder(world, decision) {
    for (const candidate of decision.ranked) {
      if (!LEADER_BRAIN_ACTIONS.includes(candidate.key)) continue;
      if (candidate.value < 0.38) break;
      const orderType = this.tryBrainAction(world, candidate.key, candidate.value, decision.outputs);
      if (orderType) {
        return {
          action: candidate.key,
          orderType,
          score: candidate.value,
          reason: "issued",
        };
      }
    }
    return { action: "wait", reason: "no valid high-scoring order" };
  }

  tryBrainAction(world, action, score, outputs) {
    const home = this.stockpiles[0];
    const people = this.members.length + 1;
    const totalFood = this.stockpiles.reduce((sum, s) => sum + s.food, 0) +
      this.everyone().reduce((sum, e) => sum + e.food, 0);
    const foodCrisis = this.isTown && totalFood < people * FOOD_CRISIS_PER_PERSON;

    if (foodCrisis && ["scout", "claim", "harvest", "expand", "defend", "fight", "fightArea", "hunt"].includes(action)) {
      return null;
    }

    if ((action === "fight" || action === "fightArea" || action === "hunt") &&
      world.isClanGraceActive()) {
      return null;
    }

    if ((action === "fight" || action === "fightArea" || action === "hunt") &&
      (people < 7 || totalFood < people * 4 || score < 0.72)) {
      return null;
    }

    if (action === "recruit") {
      if (this.hasOpenOrder("recruit")) return null;
      const rel = this.relationshipWithNeutral();
      if (rel.friendliness < rel.animosity && score < 0.7) return null;
      const target = world.nearestVisibleNeutral(this);
      if (!target) return null;
      this.issueOrder("recruit", { targetId: target.id, lastX: target.x, lastY: target.y });
      return "recruit";
    }

    if (action === "scout") {
      if (this.orders.filter(o => o.type === "scout" && !o.done).length >= 2) return null;
      const base = this.stockpiles[Math.floor(Math.random() * this.stockpiles.length)] || this.leader;
      const angle = Math.random() * Math.PI * 2;
      const dist = 12 + score * 25;
      this.issueOrder("scout", {
        x: world.clamp(Math.round(base.x + Math.cos(angle) * dist)),
        y: world.clamp(Math.round(base.y + Math.sin(angle) * dist)),
        radius: VISION_RADIUS,
      });
      return "scout";
    }

    if (action === "fight") {
      if (this.hasOpenOrder("fightEntity")) return null;
      const enemy = world.nearestVisibleEnemy(this);
      if (!enemy) return null;
      this.issueOrder("fightEntity", { targetId: enemy.id, lastX: enemy.x, lastY: enemy.y });
      return "fightEntity";
    }

    if (action === "fightArea") {
      if (this.hasOpenOrder("fightArea")) return null;
      const enemy = world.nearestVisibleEnemy(this);
      if (!enemy) return null;
      const span = 8 + Math.round(score * 12);
      const half = Math.floor(span / 2);
      this.issueOrder("fightArea", {
        x: world.clamp(enemy.x - half),
        y: world.clamp(enemy.y - half),
        w: span,
        h: span,
      });
      return "fightArea";
    }

    if (action === "hunt") {
      if (this.hasOpenOrder("hunt")) return null;
      const enemy = world.nearestVisibleEnemy(this);
      if (!enemy) return null;
      this.issueOrder("hunt", { targetId: enemy.id, lastX: enemy.x, lastY: enemy.y });
      return "hunt";
    }

    if (action === "claim") {
      if (this.hasOpenOrder("claim")) return null;
      const goal = Math.max(9, Math.round(people * (5 + outputs.claim * 7)));
      if (this.claimed.size >= goal) return null;

      const target = this.findClaimTarget(world);
      if (target) {
        this.issueOrder("claim", target);
        return "claim";
      }
      return null;
    }

    if (action === "defend") {
      if (this.hasOpenOrder("defend") || this.members.length < 2) return null;
      const span = 9 + Math.round(outputs.defend * 14);
      const half = Math.floor(span / 2);
      this.issueOrder("defend", {
        x: world.clamp(home.x - half),
        y: world.clamp(home.y - half),
        w: span,
        h: span,
      });
      return "defend";
    }

    if (action === "harvest") {
      if (this.hasOpenOrder("harvest")) return null;
      const node = world.nearestResource(home.x, home.y);
      if (!node) return null;
      const maxRange = 18 + outputs.harvest * 60;
      if ((node.x - home.x) ** 2 + (node.y - home.y) ** 2 > maxRange ** 2) return null;
      this.issueOrder("harvest", {});
      return "harvest";
    }

    if (action === "expand") {
      if (this.hasOpenOrder("putPellets")) return null;
      if (this.members.length < 5 || this.stockpiles.length >= 6) return null;
      const totalFood = this.stockpiles.reduce((sum, s) => sum + s.food, 0);
      const neededFood = Math.max(8, Math.round(30 - outputs.expand * 18));
      if (totalFood < neededFood) return null;

      const angle = Math.random() * Math.PI * 2;
      const dist = 8 + outputs.expand * 12;
      this.issueOrder("putPellets", {
        x: world.clamp(Math.round(home.x + Math.cos(angle) * dist)),
        y: world.clamp(Math.round(home.y + Math.sin(angle) * dist)),
      });
      return "putPellets";
    }

    if (action === "store") {
      if (this.hasOpenOrder("store")) return null;
      if (!this.everyone().some(e => e.carriedResources() > 0)) return null;
      this.issueOrder("store");
      return "store";
    }

    if (action === "consolidate") {
      if (this.hasOpenOrder("putResources") || this.stockpiles.length < 2) return null;
      const candidates = this.stockpiles
        .map(s => ({ stockpile: s, load: s.food + s.totalResources() }))
        .sort((a, b) => b.load - a.load);
      if (!candidates.length || candidates[0].load <= 0) return null;
      this.issueOrder("putResources", { x: candidates[0].stockpile.x, y: candidates[0].stockpile.y });
      return "putResources";
    }

    return null;
  }

  pruneOrders(world) {
    for (const order of this.orders) {
      if (order.done) continue;
      if (!order.params.startedTick) order.params.startedTick = world.tick;
      const maxAge = ORDER_MAX_AGE[order.type];
      if (maxAge && world.tick - order.params.startedTick > maxAge) {
        order.done = true;
        continue;
      }
      if (order.type === "recruit") {
        const target = world.entityById(order.params.targetId);
        if (!target || target.clan || target.dead) order.done = true;
      }
      if (order.type === "fightEntity" || order.type === "hunt") {
        const target = world.entityById(order.params.targetId);
        if (!target || target.dead) order.done = true;
      }
      // harvest: complete when the relevant resource type is gone
      if (order.type === "harvest") {
        const node = world.nearestResource(this.leader.x, this.leader.y, order.params.type);
        if (!node) order.done = true;
      }
      // store: complete once no assignee is carrying anything
      if (order.type === "store") {
        const carrying = this.everyone().some(e => e.task === order && e.carriedResources() > 0);
        if (!carrying) order.done = true;
      }
      // putResources: complete once the source stockpiles are drained
      if (order.type === "putResources") {
        const target = this.nearestStockpile(order.params.x, order.params.y);
        const carrying = this.everyone().some(e => e.task === order && e.carriedResources() > 0);
        const sourcesEmpty = this.stockpiles.every(s => s === target || s.totalResources() === 0);
        if (!target || (sourcesEmpty && !carrying)) order.done = true;
      }
    }
    for (const e of this.everyone()) {
      if (e.task && e.task.done) e.task = null;
    }
    this.orders = this.orders.filter(o => !o.done);
  }

  assignTasks() {
    const collectOrder = this.orders.find(o => o.type === "collect");
    const people = this.everyone();
    const storedFood = this.stockpiles.reduce((sum, s) => sum + s.food, 0);
    const carriedFood = people.reduce((sum, e) => sum + e.food, 0);
    const foodPressure = this.isTown && storedFood + carriedFood < people.length * FOOD_CRISIS_PER_PERSON;
    const minCollectors = foodPressure
      ? Math.min(people.length, Math.max(2, Math.ceil(people.length * 0.55)))
      : 0;

    if (foodPressure && collectOrder) {
      let collectors = people.filter(e => e.task === collectOrder || !e.task).length;
      for (const e of people) {
        if (collectors >= minCollectors) break;
        if (!e.task || e.task === collectOrder || (e.task.def && e.task.def.leaderOnly)) continue;
        e.task = collectOrder;
        collectors++;
      }
    }

    // fill bounded orders first, pulling workers off the default collect job
    for (const order of this.orders) {
      if (!isFinite(order.def.maxWorkers)) continue;
      let workers = this.workersOf(order);
      if (order.def.leaderOnly) {
        if (workers < order.def.maxWorkers && (!this.leader.task || this.leader.task === collectOrder)) {
          this.leader.task = order;
        }
        continue;
      }
      for (const m of this.members) {
        if (workers >= order.def.maxWorkers) break;
        if (foodPressure && collectOrder && people.filter(e => e.task === collectOrder || !e.task).length <= minCollectors) break;
        if (!m.task || m.task === collectOrder) {
          m.task = order;
          workers++;
        }
      }
      if (order.def.leaderCanWork && workers < order.def.maxWorkers && (!this.leader.task || this.leader.task === collectOrder)) {
        this.leader.task = order;
      }
    }

    // everyone idle (the leader included) falls back to collecting
    if (collectOrder) {
      for (const e of this.everyone()) {
        if (!e.task) e.task = collectOrder;
      }
    }
  }

  // --- territory & stockpiles ---
  claimArea(world, cx, cy, radius) {
    const pending = new Set();
    const candidates = [];
    for (let y = cy - radius; y <= cy + radius; y++) {
      for (let x = cx - radius; x <= cx + radius; x++) {
        if (world.inBounds(x, y)) candidates.push({ x, y });
      }
    }

    let changed = true;
    while (changed) {
      changed = false;
      for (const cell of candidates) {
        const key = world.key(cell.x, cell.y);
        if (this.claimed.has(key) || pending.has(key)) continue;
        if (this.canExtendClaimTo(world, cell.x, cell.y, pending)) {
          pending.add(key);
          changed = true;
        }
      }
    }

    for (const key of pending) this.claimed.add(key);
    this.revealArea(world, cx, cy, Math.max(radius, 1));
  }

  canExtendClaimTo(world, x, y, pending = new Set()) {
    if (this.claimed.size === 0 && pending.size === 0) return true;
    for (let yy = y - 1; yy <= y + 1; yy++) {
      for (let xx = x - 1; xx <= x + 1; xx++) {
        if (xx === x && yy === y) continue;
        const key = world.key(xx, yy);
        if (this.claimed.has(key) || pending.has(key)) return true;
      }
    }
    return false;
  }

  findClaimTarget(world) {
    if (this.claimed.size === 0) {
      const home = this.stockpiles[0] || this.leader;
      return { x: home.x, y: home.y };
    }

    const candidates = [];
    for (const key of this.claimed) {
      const [cx, cy] = key.split(",").map(Number);
      for (let y = cy - 1; y <= cy + 1; y++) {
        for (let x = cx - 1; x <= cx + 1; x++) {
          if (!world.inBounds(x, y)) continue;
          const nextKey = world.key(x, y);
          if (!this.claimed.has(nextKey) && this.canExtendClaimTo(world, x, y)) {
            candidates.push({ x, y });
          }
        }
      }
    }
    if (!candidates.length) return null;

    const tree = world.nearestTree(this.leader.x, this.leader.y, t => this.canSee(world, t.x, t.y));
    if (tree) {
      candidates.sort((a, b) =>
        ((a.x - tree.x) ** 2 + (a.y - tree.y) ** 2) -
        ((b.x - tree.x) ** 2 + (b.y - tree.y) ** 2));
      return candidates[0];
    }
    return candidates[Math.floor(Math.random() * candidates.length)];
  }

  revealArea(world, cx, cy, radius) {
    const expires = world.tick + VISION_MEMORY_TICKS;
    if (radius === VISION_RADIUS && typeof VISION_OFFSETS !== "undefined") {
      for (const [dx, dy] of VISION_OFFSETS) {
        const x = cx + dx;
        const y = cy + dy;
        if (world.inBounds(x, y)) this.vision.set(world.key(x, y), expires);
      }
      return;
    }
    for (let y = cy - radius; y <= cy + radius; y++) {
      for (let x = cx - radius; x <= cx + radius; x++) {
        if (!world.inBounds(x, y)) continue;
        if ((x - cx) ** 2 + (y - cy) ** 2 <= radius ** 2) {
          this.vision.set(world.key(x, y), expires);
        }
      }
    }
  }

  pruneVision(tick) {
    for (const [key, expires] of this.vision.entries()) {
      if (expires <= tick && !this.claimed.has(key)) this.vision.delete(key);
    }
  }

  canSee(world, x, y) {
    const key = world.key(x, y);
    if (this.claimed.has(key)) return true;
    return (this.vision.get(key) || 0) > world.tick;
  }

  nearestStockpile(x, y, filter) {
    let best = null;
    let bestDist = Infinity;
    for (const s of this.stockpiles) {
      if (filter && !filter(s)) continue;
      const d = (s.x - x) ** 2 + (s.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = s;
      }
    }
    return best;
  }

  totalFood() {
    return this.stockpiles.reduce((sum, s) => sum + s.food, 0) +
      this.everyone().reduce((sum, e) => sum + e.food, 0);
  }

  totalResources() {
    return this.stockpiles.reduce((sum, s) => sum + s.totalResources(), 0) +
      this.everyone().reduce((sum, e) => sum + e.carriedResources(), 0);
  }

  ownedTreeCount(world) {
    return world.trees.filter(t => !t.destroyed && this.claimed.has(world.key(t.x, t.y))).length;
  }

  recordStats(world) {
    const people = this.everyone().filter(e => !e.dead).length;
    const food = this.totalFood();
    const resources = this.totalResources();
    this.stats.peakPeople = Math.max(this.stats.peakPeople || 0, people);
    this.stats.peakFood = Math.max(this.stats.peakFood || 0, food);
    this.stats.peakResources = Math.max(this.stats.peakResources || 0, resources);
    this.stats.peakTerritory = Math.max(this.stats.peakTerritory || 0, this.claimed.size);
    this.stats.peakStockpiles = Math.max(this.stats.peakStockpiles || 0, this.stockpiles.length);
    this.stats.peakTrees = Math.max(this.stats.peakTrees || 0, this.ownedTreeCount(world));
  }

  rollMinPerPerson() {
    this.minPerPerson = Math.random() < 0.5 ? 0 : 1;
  }

  relationshipKeyForClan(clan) {
    return `clan:${clan.id}`;
  }

  ensureRelationship(key, friendliness = 0.4, animosity = 0.35) {
    if (!this.relationships[key]) {
      this.relationships[key] = { friendliness, animosity };
    }
    return this.relationships[key];
  }

  ensureRelationships(world) {
    this.ensureRelationship(REL_NEUTRAL, 0.55, 0.25);
    for (const clan of world.clans) {
      if (clan === this || clan.disbanded) continue;
      this.ensureRelationship(this.relationshipKeyForClan(clan), 0.45, 0.25);
    }
  }

  relationshipWithNeutral() {
    return this.ensureRelationship(REL_NEUTRAL, 0.55, 0.25);
  }

  relationshipWithClan(clan) {
    return this.ensureRelationship(this.relationshipKeyForClan(clan), 0.45, 0.25);
  }

  relationshipWithEntity(entity) {
    return entity.clan ? this.relationshipWithClan(entity.clan) : this.relationshipWithNeutral();
  }

  isHostileToEntity(entity) {
    if (!entity || entity.dead || entity.clan === this) return false;
    const rel = this.relationshipWithEntity(entity);
    return rel.animosity > rel.friendliness + 0.12 || rel.animosity > 0.68;
  }

  adjustRelationships(world, outputs) {
    const neutral = this.relationshipWithNeutral();
    neutral.friendliness = neutral.friendliness * 0.85 + brainClamp01(outputs.friendliness || 0) * 0.15;
    neutral.animosity = neutral.animosity * 0.85 + brainClamp01(outputs.animosity || 0) * 0.15;

    for (const other of world.clans) {
      if (other === this || other.disbanded) continue;
      const rel = this.relationshipWithClan(other);
      const enemyVisible = this.canSee(world, other.leader.x, other.leader.y) ? 0.1 : 0;
      rel.friendliness = rel.friendliness * 0.9 + brainClamp01(outputs.friendliness || 0) * 0.1;
      rel.animosity = rel.animosity * 0.9 + brainClamp01((outputs.animosity || 0) + enemyVisible) * 0.1;
    }
  }

  serializeRelationships() {
    const out = {};
    for (const [key, value] of Object.entries(this.relationships)) {
      out[key] = {
        friendliness: value.friendliness,
        animosity: value.animosity,
      };
    }
    return out;
  }

  deserializeRelationships(data) {
    this.relationships = {};
    for (const [key, value] of Object.entries(data || {})) {
      this.relationships[key] = {
        friendliness: value.friendliness !== undefined ? value.friendliness : (key === REL_NEUTRAL ? 0.55 : 0.45),
        animosity: value.animosity !== undefined ? value.animosity : 0.25,
      };
    }
  }

  handleLeaderDeath(oldLeader, world) {
    oldLeader.clan = null;
    oldLeader.task = null;
    const successor = this.members.find(m => !m.dead);
    if (!successor) {
      this.disband();
      return;
    }

    this.members = this.members.filter(m => m !== successor);
    successor.promoteToLeader();
    successor.clan = this;
    successor.task = null;
    successor.brain = this.brain;
    this.leader = successor;
    this.brain.lastDecision = null;
    this.revealArea(world, successor.x, successor.y, VISION_RADIUS);
  }

  disband() {
    for (const m of this.members) {
      m.clan = null;
      m.task = null;
    }
    this.members = [];
    if (this.leader) {
      this.leader.clan = null;
      this.leader.task = null;
    }
    this.orders = [];
    this.claimed.clear();
    this.vision.clear();
    this.disbanded = true;
  }
}
