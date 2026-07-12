// World: the 256x256 grid and everything living on it.
const VISION_RADIUS = 15;
const VISION_MEMORY_TICKS = 2000;
const FOOD_FIGHT_RANGE = 2;
const CLAN_GRACE_TICKS = 1000;
const VISION_REFRESH_TICKS = 100;
const VISION_SCAN_INTERVAL = 20;
const VISION_PRUNE_INTERVAL = 100;
const VISION_OFFSETS = [];
for (let dy = -VISION_RADIUS; dy <= VISION_RADIUS; dy++) {
  for (let dx = -VISION_RADIUS; dx <= VISION_RADIUS; dx++) {
    if (dx * dx + dy * dy <= VISION_RADIUS * VISION_RADIUS) {
      VISION_OFFSETS.push([dx, dy]);
    }
  }
}

class World {
  constructor(size = 256) {
    this.size = size;
    this.tick = 0;
    this.entities = [];
    this.resources = [];
    this.trees = [];
    this.clans = [];
    this.pellets = new Map(); // "x,y" -> Pellet, for O(1) lookup by cell
    this.maxPellets = Math.floor(size * size * 0.04);
  }

  key(x, y) {
    return x + "," + y;
  }

  clamp(v) {
    return Math.max(0, Math.min(this.size - 1, v));
  }

  inBounds(x, y) {
    return x >= 0 && x < this.size && y >= 0 && y < this.size;
  }

  randomCell() {
    return {
      x: Math.floor(Math.random() * this.size),
      y: Math.floor(Math.random() * this.size),
    };
  }

  // --- pellets ---
  addPellet(pellet) {
    this.pellets.set(this.key(pellet.x, pellet.y), pellet);
  }

  pelletAt(x, y) {
    return this.pellets.get(this.key(x, y));
  }

  removePellet(pellet) {
    this.pellets.delete(this.key(pellet.x, pellet.y));
  }

  nearestPellet(x, y, filter) {
    let best = null;
    let bestDist = Infinity;
    for (const p of this.pellets.values()) {
      if (filter && !filter(p)) continue;
      const d = (p.x - x) ** 2 + (p.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = p;
      }
    }
    return best;
  }

  nearestPelletInKeys(x, y, keys, best = null, bestDist = Infinity) {
    for (const key of keys) {
      const p = this.pellets.get(key);
      if (!p) continue;
      const d = (p.x - x) ** 2 + (p.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = p;
      }
    }
    return { pellet: best, dist: bestDist };
  }

  nearestPelletInRadius(x, y, radius, best = null, bestDist = Infinity) {
    for (let yy = y - radius; yy <= y + radius; yy++) {
      if (yy < 0 || yy >= this.size) continue;
      for (let xx = x - radius; xx <= x + radius; xx++) {
        if (xx < 0 || xx >= this.size) continue;
        const p = this.pelletAt(xx, yy);
        if (!p) continue;
        const d = (p.x - x) ** 2 + (p.y - y) ** 2;
        if (d < bestDist) {
          bestDist = d;
          best = p;
        }
      }
    }
    return { pellet: best, dist: bestDist };
  }

  canGatherPellet(entity, pellet) {
    if (!pellet || entity.dead) return false;
    const key = this.key(pellet.x, pellet.y);
    if (entity.clan) return entity.clan.claimed.has(key);
    return entity.claimed.has(key);
  }

  nearestGatherablePellet(entity) {
    if (entity.clan) {
      return this.nearestPelletInKeys(entity.x, entity.y, entity.clan.claimed).pellet;
    }
    return this.nearestPelletInKeys(entity.x, entity.y, entity.claimed).pellet;
  }

  nearestVisiblePellet(entity) {
    if (entity.clan) {
      let found = this.nearestPelletInKeys(entity.x, entity.y, entity.clan.claimed);
      found = this.nearestPelletInKeys(entity.x, entity.y, entity.clan.vision.keys(), found.pellet, found.dist);
      return found.pellet;
    }
    let found = this.nearestPelletInKeys(entity.x, entity.y, entity.claimed);
    found = this.nearestPelletInRadius(entity.x, entity.y, VISION_RADIUS, found.pellet, found.dist);
    return found.pellet;
  }

  // --- resources ---
  addResource(resource) {
    this.resources.push(resource);
  }

  resourceAt(x, y) {
    return this.resources.find(r => r.x === x && r.y === y);
  }

  nearestResource(x, y, type) {
    let best = null;
    let bestDist = Infinity;
    for (const r of this.resources) {
      if (r.depleted) continue;
      if (type && r.type !== type) continue;
      const d = (r.x - x) ** 2 + (r.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = r;
      }
    }
    return best;
  }

  // --- trees ---
  addTree(tree) {
    this.trees.push(tree);
  }

  treeAt(x, y) {
    return this.trees.find(t => !t.destroyed && t.x === x && t.y === y);
  }

  nearestTree(x, y, filter) {
    let best = null;
    let bestDist = Infinity;
    for (const t of this.trees) {
      if (t.destroyed) continue;
      if (filter && !filter(t)) continue;
      const d = (t.x - x) ** 2 + (t.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = t;
      }
    }
    return best;
  }

  // --- entities ---
  addEntity(entity) {
    this.entities.push(entity);
    if (entity.isLeader && !entity.clan) this.createClan(entity);
  }

  entityAt(x, y) {
    return this.entities.find(e => e.x === x && e.y === y);
  }

  entitiesAt(x, y) {
    return this.entities.filter(e => e.x === x && e.y === y);
  }

  entityById(id) {
    return this.entities.find(e => e.id === id && !e.dead);
  }

  // --- clans ---
  createClan(leader) {
    const color = `hsl(${Math.floor(Math.random() * 360)}, 85%, 60%)`;
    const clan = new Clan(leader, color);
    leader.clan = clan;
    this.clans.push(clan);
    for (const c of this.clans) c.ensureRelationships(this);
    return clan;
  }

  clanWithStockpileAt(x, y) {
    return this.clans.find(c => c.stockpiles.some(s => s.x === x && s.y === y));
  }

  // --- vision, ownership, combat ---
  clanGraceRemaining() {
    return Math.max(0, CLAN_GRACE_TICKS - this.tick);
  }

  isClanGraceActive() {
    return this.clanGraceRemaining() > 0;
  }

  isProtectedClanPair(a, b) {
    return this.isClanGraceActive() && a && b && a.clan && b.clan && a.clan !== b.clan;
  }

  canEntitySee(entity, x, y) {
    if (!entity || entity.dead) return false;
    if (entity.clan) return entity.clan.canSee(this, x, y);
    const dist = Math.max(Math.abs(entity.x - x), Math.abs(entity.y - y));
    return dist <= VISION_RADIUS || entity.claimed.has(this.key(x, y));
  }

  updateVision() {
    if (this.tick % VISION_PRUNE_INTERVAL === 0) {
      for (const clan of this.clans) clan.pruneVision(this.tick);
    }
    if (this.tick % VISION_SCAN_INTERVAL !== 0) return;
    for (const e of this.entities) {
      if (e.dead || !e.clan) continue;
      if (e._visionX === e.x && e._visionY === e.y && e._visionClan === e.clan.id &&
        this.tick - (e._visionTick || 0) < VISION_REFRESH_TICKS) {
        continue;
      }
      e.clan.revealArea(this, e.x, e.y, VISION_RADIUS);
      e._visionX = e.x;
      e._visionY = e.y;
      e._visionClan = e.clan.id;
      e._visionTick = this.tick;
    }
  }

  visibleEntitiesForClan(clan, filter) {
    return this.entities.filter(e => {
      if (e.dead || e.clan === clan) return false;
      if (!clan.canSee(this, e.x, e.y)) return false;
      return !filter || filter(e);
    });
  }

  nearestVisibleNeutral(clan) {
    let best = null;
    let bestDist = Infinity;
    for (const e of this.visibleEntitiesForClan(clan, e => !e.clan && !e.isLeader)) {
      const d = (e.x - clan.leader.x) ** 2 + (e.y - clan.leader.y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = e;
      }
    }
    return best;
  }

  nearestVisibleEnemy(clan, x = clan.leader.x, y = clan.leader.y, filter) {
    let best = null;
    let bestDist = Infinity;
    for (const e of this.visibleEntitiesForClan(clan, e => clan.isHostileToEntity(e) && (!filter || filter(e)))) {
      const d = (e.x - x) ** 2 + (e.y - y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = e;
      }
    }
    return best;
  }

  ownerKey(entity) {
    return entity.clan ? `clan:${entity.clan.id}` : `neutral:${entity.id}`;
  }

  sameFaction(a, b) {
    if (!a || !b || a === b) return true;
    if (a.clan && b.clan) return a.clan === b.clan;
    return false;
  }

  shouldFight(a, b, forced = false) {
    if (!a || !b || a.dead || b.dead || this.sameFaction(a, b)) return false;
    if (this.isProtectedClanPair(a, b)) return false;
    if (forced) return true;
    if (a.clan && !a.clan.isHostileToEntity(b)) return false;
    if (b.clan && !b.clan.isHostileToEntity(a)) return false;
    return true;
  }

  shouldFightForFood(a, b) {
    if (!a.foodTargetKey || a.foodTargetKey !== b.foodTargetKey) return false;
    if (this.sameFaction(a, b)) return false;
    if (this.isProtectedClanPair(a, b)) return false;
    if (a.clan && b.clan) {
      const ar = a.clan.relationshipWithClan(b.clan);
      const br = b.clan.relationshipWithClan(a.clan);
      const friendly = (ar.friendliness + br.friendliness) * 0.5;
      const hostile = (ar.animosity + br.animosity) * 0.5;
      return hostile > friendly + 0.1 || hostile > 0.6;
    }
    return true;
  }

  resolveFoodFights() {
    const byTarget = new Map();
    for (const e of this.entities) {
      if (e.dead || !e.foodTargetKey) continue;
      if (!byTarget.has(e.foodTargetKey)) byTarget.set(e.foodTargetKey, []);
      byTarget.get(e.foodTargetKey).push(e);
    }

    for (const contenders of byTarget.values()) {
      if (contenders.length < 2) continue;
      for (const e of contenders) {
        let target = null;
        let bestDist = Infinity;
        for (const other of contenders) {
          if (e === other || !this.shouldFightForFood(e, other)) continue;
          const dist = Math.max(Math.abs(e.x - other.x), Math.abs(e.y - other.y));
          if (dist <= FOOD_FIGHT_RANGE && dist < bestDist) {
            bestDist = dist;
            target = other;
          }
        }
        if (target) e.attack(target, this);
      }
    }
  }

  countContestedFoodForClan(clan) {
    const targets = new Set(clan.everyone().map(e => e.foodTargetKey).filter(Boolean));
    if (!targets.size) return 0;
    let contested = 0;
    for (const e of this.entities) {
      if (e.dead || e.clan === clan || !e.foodTargetKey || !targets.has(e.foodTargetKey)) continue;
      contested++;
    }
    return contested;
  }

  // Detach a dying/erased entity from its clan before removal.
  detachEntity(entity) {
    entity.dead = true;
    entity.claimed.clear();
    if (!entity.clan) return;
    const clan = entity.clan;
    if (!entity._lossCounted) {
      clan.stats.losses++;
      entity._lossCounted = true;
    }
    if (clan.leader === entity) clan.handleLeaderDeath(entity, this);
    else clan.removeMember(entity);
  }

  // --- editing ---
  clearCell(x, y) {
    this.pellets.delete(this.key(x, y));
    this.resources = this.resources.filter(r => !(r.x === x && r.y === y));
    this.trees = this.trees.filter(t => !(t.x === x && t.y === y));

    for (const e of this.entities) {
      if (e.x === x && e.y === y) this.detachEntity(e);
    }
    this.entities = this.entities.filter(e => !e.dead);
    this.clans = this.clans.filter(c => !c.disbanded);

    // erasing a clan's last stockpile reverts the town back to a plain clan
    for (const clan of this.clans) {
      const before = clan.stockpiles.length;
      clan.stockpiles = clan.stockpiles.filter(s => !(s.x === x && s.y === y));
      if (before > 0 && clan.stockpiles.length === 0) {
        clan.orders = [];
        for (const e of clan.everyone()) e.task = null;
      }
    }
  }

  clear() {
    this.tick = 0;
    this.entities = [];
    this.resources = [];
    this.trees = [];
    this.clans = [];
    this.pellets.clear();
  }

  // --- simulation ---
  step() {
    this.tick++;

    for (const tree of this.trees) tree.update(this);
    this.updateVision();

    for (const entity of this.entities) {
      entity.update(this);
    }

    this.resolveFoodFights();

    // starvation deaths
    for (const e of this.entities) {
      if (e.dead) this.detachEntity(e);
    }
    this.entities = this.entities.filter(e => !e.dead);

    // recruiting + town founding
    for (const clan of this.clans) {
      clan.update(this);
    }
    this.clans = this.clans.filter(c => !c.disbanded);

    // Legacy fallback: neural leaders set this in leaderThink via their
    // reserve-food output, so only brainless towns use the old random roll.
    if (this.tick % 1000 === 0) {
      for (const clan of this.clans) {
        if (clan.isTown && !clan.brain) clan.rollMinPerPerson();
      }
    }

    this.resources = this.resources.filter(r => !r.depleted);
    this.trees = this.trees.filter(t => !t.destroyed);
  }

  // --- save / load ---
  serialize() {
    return {
      version: 7,
      size: this.size,
      tick: this.tick,
      pellets: [...this.pellets.values()].map(p => ({ x: p.x, y: p.y, energy: p.energy })),
      resources: this.resources.map(r => ({ x: r.x, y: r.y, type: r.type, amount: r.amount })),
      trees: this.trees.map(t => t.serialize()),
      entities: this.entities.map(e => ({
        id: e.id,
        x: e.x,
        y: e.y,
        isLeader: e.isLeader,
        health: e.health,
        speed: e.speed,
        food: e.food,
        ticksSinceFood: e.ticksSinceFood,
        hungerThreshold: e.hungerThreshold,
        inventory: e.inventory,
        claimed: [...e.claimed],
        attackCooldown: e.attackCooldown,
      })),
      clans: this.clans.map(c => ({
        id: c.id,
        leaderId: c.leader.id,
        memberIds: c.members.map(m => m.id),
        color: c.color,
        minPerPerson: c.minPerPerson,
        stats: c.stats,
        brain: c.brain.serialize(),
        relationships: c.serializeRelationships(),
        vision: [...c.vision.entries()],
        claimed: [...c.claimed],
        orders: c.orders.map(o => ({ type: o.type, params: o.params })),
        stockpiles: c.stockpiles.map(s => ({ x: s.x, y: s.y, food: s.food, resources: s.resources })),
      })),
    };
  }

  deserialize(data) {
    this.clear();
    this.tick = data.tick || 0;

    for (const p of data.pellets || []) {
      this.addPellet(new Pellet(p.x, p.y, p.energy));
    }
    for (const r of data.resources || []) {
      this.addResource(new Resource(r.x, r.y, r.type, r.amount));
    }
    for (const t of data.trees || []) {
      this.addTree(new Tree(t.x, t.y, t));
    }

    const byId = new Map();
    for (const d of data.entities || []) {
      const e = new Entity(d.x, d.y);
      // v1 saves predate leaders; keep the constructor's fresh roll for those
      if (d.isLeader !== undefined) e.isLeader = !!d.isLeader;
      if (d.id !== undefined) e.id = d.id;
      if (d.speed !== undefined) e.speed = d.speed; // pre-v3 saves keep the fresh roll
      e.health = d.health !== undefined ? d.health : e.maxHealth;
      e.food = d.food || 0;
      e.ticksSinceFood = d.ticksSinceFood || 0;
      if (d.hungerThreshold) e.hungerThreshold = d.hungerThreshold;
      e.inventory = d.inventory || {};
      e.claimed = new Set(d.claimed || []);
      e.attackCooldown = d.attackCooldown || 0;
      this.entities.push(e); // bypass addEntity: clans are rebuilt below
      byId.set(e.id, e);
      Entity.nextId = Math.max(Entity.nextId, e.id + 1);
    }

    for (const c of data.clans || []) {
      const leader = byId.get(c.leaderId);
      if (!leader) continue;
      const brain = c.brain ? LeaderBrain.fromJSON(c.brain) : LeaderBrain.random();
      const clan = new Clan(leader, c.color, brain);
      if (c.id !== undefined) clan.id = c.id;
      Clan.nextId = Math.max(Clan.nextId, clan.id + 1);
      leader.clan = clan;
      clan.minPerPerson = c.minPerPerson !== undefined ? c.minPerPerson : 1;
      clan.stats = { ...clan.stats, ...(c.stats || {}) };
      clan.claimed = new Set(c.claimed || []);
      clan.vision = new Map(c.vision || []);
      clan.deserializeRelationships(c.relationships || {});

      // v4 stores a stockpile list; v2/v3 stored a single `stockpile`
      const stockpiles = c.stockpiles || (c.stockpile ? [c.stockpile] : []);
      for (const s of stockpiles) {
        const sp = new Stockpile(s.x, s.y);
        sp.food = s.food || 0;
        sp.resources = s.resources || {};
        clan.stockpiles.push(sp);
      }

      for (const o of c.orders || []) {
        clan.orders.push(new Task(o.type, o.params || {}));
      }
      // pre-v4 towns had an implicit collect job; make it a real order
      if (clan.isTown && !clan.orders.some(o => o.type === "collect")) {
        clan.issueOrder("collect");
      }

      for (const id of c.memberIds || []) {
        const m = byId.get(id);
        if (m) {
          m.clan = clan;
          clan.members.push(m);
        }
      }
      this.clans.push(clan); // tasks are re-assigned by assignTasks on the next step
    }

    // leaders that lost their clan record (or fresh v1 rolls) get one now
    for (const e of this.entities) {
      if (e.isLeader && !e.clan) this.createClan(e);
    }
    for (const clan of this.clans) clan.ensureRelationships(this);
  }
}
