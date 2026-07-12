// Entity: an NPC. Runs one priority behaviour (eat when hungry), otherwise
// works its assigned task, otherwise wanders. Starvation drains health;
// eating well heals it back.
const STARVE_TICKS = 900;   // ticks without eating before health starts draining
const LEADER_CHANCE = 0.02; // 2% of entities are born leaders
const MIN_SPEED = 0.25;     // cells per tick
const MAX_SPEED = 0.5;
const BASE_HEALTH = 10;
const LEADER_HEALTH = 15;
const STARVE_DAMAGE = 0.1;  // health lost per tick while starving
const HEAL_RATE = 0.05;     // health regained per tick while fed
const CARRY_LIMIT = 5;      // resource units carried before storing
const ATTACK_DAMAGE = 0.45;
const ATTACK_COOLDOWN_TICKS = 20;
const ATTACK_RANGE = 1;

class Entity {
  static nextId = 1;

  // isLeader: omit for the natural 2% roll, pass true/false to force it
  constructor(x, y, isLeader) {
    this.id = Entity.nextId++;
    this.x = x;
    this.y = y;
    this.isLeader = isLeader !== undefined ? isLeader : Math.random() < LEADER_CHANCE;
    this.health = this.maxHealth;
    this.speed = MIN_SPEED + Math.random() * (MAX_SPEED - MIN_SPEED);
    this.moveBudget = 0; // accrues `speed` per tick; moving a cell costs 1
    this.clan = null;
    this.task = null;     // Task reference, assigned by the clan
    this.food = 0;        // carried food units
    this.inventory = {};  // carried resources, e.g. { wood: 3 }
    this.claimed = new Set(); // neutral per-entity territory; clans use Clan.claimed
    this.ticksSinceFood = 0;
    this.hungerThreshold = this.rollHungerThreshold();
    this.attackCooldown = 0;
    this.foodTargetKey = null;
    this.dead = false;
  }

  get maxHealth() {
    return this.isLeader ? LEADER_HEALTH : BASE_HEALTH;
  }

  promoteToLeader() {
    this.isLeader = true;
    this.health = this.maxHealth;
  }

  // Each entity seeks food at its own point between 30% and 70% hunger,
  // re-rolled after every meal.
  rollHungerThreshold() {
    return 0.3 + Math.random() * 0.4;
  }

  get hunger() {
    return this.ticksSinceFood / STARVE_TICKS; // 1.0+ = starving
  }

  get starving() {
    return this.ticksSinceFood >= STARVE_TICKS;
  }

  resetHunger() {
    this.ticksSinceFood = 0;
    this.hungerThreshold = this.rollHungerThreshold();
  }

  update(world) {
    this.ticksSinceFood++;
    if (this.starving) {
      this.health -= STARVE_DAMAGE;
      if (this.health <= 0) {
        this.dead = true;
        return;
      }
    } else if (this.health < this.maxHealth) {
      this.health = Math.min(this.maxHealth, this.health + HEAL_RATE);
    }

    this.moveBudget += this.speed;
    this.attackCooldown = Math.max(0, this.attackCooldown - 1);
    this.foodTargetKey = null;

    // priority behaviour: eating always preempts assigned tasks
    if (this.hunger >= this.hungerThreshold) {
      this.findFood(world);
      return;
    }

    if (this.task && !this.task.done) {
      this.runTask(world);
      return;
    }

    this.randomWalk(world);
  }

  // --- priority behaviour: find food ---
  findFood(world) {
    const stockpile = this.clan && this.clan.nearestStockpile(this.x, this.y);
    if (stockpile && this.food > this.clan.minPerPerson && !this.starving && this.hunger <= 0.65) {
      this.moveToward(world, stockpile.x, stockpile.y);
      if (this.x === stockpile.x && this.y === stockpile.y) {
        stockpile.food += this.food - this.clan.minPerPerson;
        this.food = this.clan.minPerPerson;
      }
      return;
    }

    // carried food first
    if (this.food > 0) {
      this.food--;
      this.resetHunger();
      this.randomWalk(world);
      return;
    }

    const emergencyStockpile = this.clan && this.clan.nearestStockpile(this.x, this.y, s => s.food > 0);
    if (emergencyStockpile && (this.starving || this.hunger > 0.65)) {
      this.foodTargetKey = `stockpile:${this.clan.id}:${emergencyStockpile.x},${emergencyStockpile.y}`;
      this.moveToward(world, emergencyStockpile.x, emergencyStockpile.y);
      if (this.x === emergencyStockpile.x && this.y === emergencyStockpile.y && emergencyStockpile.food > 0) {
        emergencyStockpile.food--;
        this.resetHunger();
      }
      return;
    }

    // then the nearest pellet that can legally be gathered from territory
    let pellet = world.nearestGatherablePellet(this);
    if (!pellet && !this.clan) pellet = world.nearestVisiblePellet(this);
    if (pellet) {
      this.foodTargetKey = `pellet:${pellet.x},${pellet.y}`;
      this.moveToward(world, pellet.x, pellet.y);
      if (!this.clan) this.claimNeutralArea(world, this.x, this.y, 1);
      const here = world.pelletAt(this.x, this.y);
      if (here) {
        if (!this.clan && !world.canGatherPellet(this, here)) this.claimNeutralArea(world, here.x, here.y, 1);
        if (world.canGatherPellet(this, here)) {
          world.removePellet(here);
          this.resetHunger();
        }
      }
      return;
    }

    // no pellets anywhere: fall back to the nearest town stockpile with food
    const foodStockpile = this.clan && this.clan.nearestStockpile(this.x, this.y, s => s.food > 0);
    if (foodStockpile) {
      this.foodTargetKey = `stockpile:${this.clan.id}:${foodStockpile.x},${foodStockpile.y}`;
      this.moveToward(world, foodStockpile.x, foodStockpile.y);
      if (this.x === foodStockpile.x && this.y === foodStockpile.y && foodStockpile.food > 0) {
        foodStockpile.food--;
        this.resetHunger();
      }
      return;
    }

    this.randomWalk(world);
  }

  // --- assigned tasks ---
  runTask(world) {
    switch (this.task.type) {
      case "collect":      return this.taskCollect(world);
      case "defend":       return this.taskDefend(world, this.task);
      case "claim":        return this.taskClaim(world, this.task);
      case "recruit":      return this.taskRecruit(world, this.task);
      case "scout":        return this.taskScout(world, this.task);
      case "fightEntity":  return this.taskFightEntity(world, this.task);
      case "fightArea":    return this.taskFightArea(world, this.task);
      case "hunt":         return this.taskHunt(world, this.task);
      case "moveBuilding": return this.taskMoveBuilding(world, this.task);
      case "harvest":      return this.taskHarvest(world, this.task);
      case "store":        return this.taskStore(world, this.task);
      case "putResources": return this.taskPutResources(world, this.task);
      case "putPellets":   return this.taskPutPellets(world, this.task);
      default:             return this.randomWalk(world);
    }
  }

  // gather pellets as carried food; haul excess to the nearest stockpile
  taskCollect(world) {
    const stockpile = this.clan && this.clan.nearestStockpile(this.x, this.y);
    if (!stockpile) return this.randomWalk(world);

    if (this.food > this.clan.minPerPerson) {
      this.moveToward(world, stockpile.x, stockpile.y);
      if (this.x === stockpile.x && this.y === stockpile.y) {
        stockpile.food += this.food - this.clan.minPerPerson;
        this.food = this.clan.minPerPerson;
      }
      return;
    }

    const pellet = world.nearestGatherablePellet(this);
    if (pellet) {
      this.foodTargetKey = `pellet:${pellet.x},${pellet.y}`;
      this.moveToward(world, pellet.x, pellet.y);
      const here = world.pelletAt(this.x, this.y);
      if (here && world.canGatherPellet(this, here)) {
        world.removePellet(here);
        this.food++;
        if (this.starving) {
          this.food--;
          this.resetHunger();
        }
      }
      return;
    }

    this.randomWalk(world);
  }

  // patrol inside the ordered area and attack hostiles that enter it
  taskDefend(world, task) {
    const { x, y, w, h } = task.params;
    const enemy = this.nearestEnemyInArea(world, x, y, w, h);
    if (enemy) return this.engageTarget(world, enemy, false);

    const inside = this.x >= x && this.x < x + w && this.y >= y && this.y < y + h;
    if (!inside) {
      this.moveToward(world, x + Math.floor(w / 2), y + Math.floor(h / 2));
      return;
    }
    this.randomWalk(world);
  }

  // walk to the cell and claim a 3x3 around it as town territory
  taskClaim(world, task) {
    this.moveToward(world, task.params.x, task.params.y);
    if (this.x === task.params.x && this.y === task.params.y) {
      if (this.clan) this.clan.claimArea(world, task.params.x, task.params.y, 1);
      else this.claimNeutralArea(world, task.params.x, task.params.y, 1);
      task.done = true;
    }
  }

  // leader-only: walk to a neutral entity so normal proximity recruitment fires
  taskRecruit(world, task) {
    if (!this.clan || this.clan.leader !== this) {
      task.done = true;
      return;
    }
    const target = world.entityById(task.params.targetId);
    if (!target || target.clan || target.dead || target.isLeader) {
      task.done = true;
      return;
    }
    task.params.lastX = target.x;
    task.params.lastY = target.y;
    this.moveToward(world, target.x, target.y);
    const dist = Math.max(Math.abs(this.x - target.x), Math.abs(this.y - target.y));
    if (dist <= CLAN_RECRUIT_RADIUS) {
      this.clan.addMember(target);
      task.done = true;
    }
  }

  taskScout(world, task) {
    const radius = task.params.radius || VISION_RADIUS;
    this.moveToward(world, task.params.x, task.params.y);
    if (this.clan) this.clan.revealArea(world, this.x, this.y, radius);
    if (this.x === task.params.x && this.y === task.params.y) task.done = true;
  }

  taskFightEntity(world, task) {
    const target = world.entityById(task.params.targetId);
    if (!target || target.dead) {
      task.done = true;
      return;
    }
    task.params.lastX = target.x;
    task.params.lastY = target.y;
    this.engageTarget(world, target, true);
  }

  taskFightArea(world, task) {
    const { x, y, w, h } = task.params;
    const enemy = this.nearestEnemyInArea(world, x, y, w, h);
    if (enemy) return this.engageTarget(world, enemy, true);

    const inside = this.x >= x && this.x < x + w && this.y >= y && this.y < y + h;
    if (!inside) return this.moveToward(world, x + Math.floor(w / 2), y + Math.floor(h / 2));
    this.randomWalk(world);
  }

  taskHunt(world, task) {
    const target = world.entityById(task.params.targetId);
    if (!target || target.dead) {
      task.done = true;
      return;
    }
    if (this.clan && this.clan.canSee(world, target.x, target.y)) {
      task.params.lastX = target.x;
      task.params.lastY = target.y;
      return this.engageTarget(world, target, true);
    }
    if (task.params.lastX !== undefined && task.params.lastY !== undefined) {
      this.moveToward(world, task.params.lastX, task.params.lastY);
      return;
    }
    this.randomWalk(world);
  }

  // walk to a stockpile, then carry it (contents and all) to the target cell
  taskMoveBuilding(world, task) {
    const { fromX, fromY, toX, toY } = task.params;
    if (!task.stockpile) {
      const sp = this.clan.stockpiles.find(s => s.x === fromX && s.y === fromY);
      if (!sp) {
        task.done = true;
        return;
      }
      this.moveToward(world, sp.x, sp.y);
      if (this.x === sp.x && this.y === sp.y) task.stockpile = sp;
      return;
    }
    this.moveToward(world, toX, toY);
    task.stockpile.x = this.x;
    task.stockpile.y = this.y;
    if (this.x === toX && this.y === toY) {
      task.stockpile = null;
      task.done = true;
    }
  }

  // mine/chop the nearest resource node, store loads at the nearest stockpile
  taskHarvest(world, task) {
    if (this.carriedResources() >= CARRY_LIMIT) return this.storeCarried(world);

    const node = world.nearestResource(this.x, this.y, task.params.type);
    if (!node) return this.randomWalk(world);

    this.moveToward(world, node.x, node.y);
    if (this.x === node.x && this.y === node.y) {
      this.addToInventory(node.type, node.harvest(1));
    }
  }

  // deposit whatever resources you're carrying (clan completes the order
  // once nobody assigned is carrying anything)
  taskStore(world, task) {
    if (this.carriedResources() === 0) return this.randomWalk(world);
    this.storeCarried(world);
  }

  // drain other stockpiles into the one nearest the ordered X,Y
  taskPutResources(world, task) {
    const target = this.clan.nearestStockpile(task.params.x, task.params.y);
    if (!target) {
      task.done = true;
      return;
    }

    if (this.carriedResources() > 0) {
      this.moveToward(world, target.x, target.y);
      if (this.x === target.x && this.y === target.y) this.depositResources(target);
      return;
    }

    const source = this.clan.nearestStockpile(this.x, this.y,
      s => s !== target && s.totalResources() > 0);
    if (!source) return this.randomWalk(world); // clan marks the order done

    this.moveToward(world, source.x, source.y);
    if (this.x === source.x && this.y === source.y) {
      let space = CARRY_LIMIT;
      for (const [type, n] of Object.entries(source.resources)) {
        if (space <= 0) break;
        const take = Math.min(n, space);
        source.resources[type] -= take;
        if (source.resources[type] <= 0) delete source.resources[type];
        this.addToInventory(type, take);
        space -= take;
      }
    }
  }

  // walk to X,Y and build a new stockpile there
  taskPutPellets(world, task) {
    const { x, y } = task.params;
    this.moveToward(world, x, y);
    if (this.x === x && this.y === y) {
      if (!world.clanWithStockpileAt(x, y)) {
        this.clan.stockpiles.push(new Stockpile(x, y));
        this.clan.claimArea(world, x, y, 1);
      }
      task.done = true;
    }
  }

  // --- helpers ---
  nearestEnemyInArea(world, x, y, w, h) {
    if (!this.clan) return null;
    let best = null;
    let bestDist = Infinity;
    for (const e of world.entities) {
      if (e.dead || e === this || !world.shouldFight(this, e)) continue;
      if (e.x < x || e.x >= x + w || e.y < y || e.y >= y + h) continue;
      if (!this.clan.canSee(world, e.x, e.y)) continue;
      const d = (e.x - this.x) ** 2 + (e.y - this.y) ** 2;
      if (d < bestDist) {
        bestDist = d;
        best = e;
      }
    }
    return best;
  }

  engageTarget(world, target, forced) {
    if (!target || target.dead || !world.shouldFight(this, target, forced)) return this.randomWalk(world);
    const dist = Math.max(Math.abs(this.x - target.x), Math.abs(this.y - target.y));
    if (dist <= ATTACK_RANGE) {
      this.attack(target, world);
      return;
    }
    this.moveToward(world, target.x, target.y);
  }

  attack(target, world) {
    if (!target || target.dead || this.attackCooldown > 0) return false;
    if (world && world.isProtectedClanPair(this, target)) return false;
    const dist = Math.max(Math.abs(this.x - target.x), Math.abs(this.y - target.y));
    if (dist > ATTACK_RANGE) return false;

    target.health -= ATTACK_DAMAGE;
    this.attackCooldown = ATTACK_COOLDOWN_TICKS;
    if (target.health <= 0) {
      target.dead = true;
      if (this.clan && target.clan && target.clan !== this.clan) this.clan.stats.kills++;
      this.food += target.food || 0;
      target.food = 0;
      for (const [type, n] of Object.entries(target.inventory)) this.addToInventory(type, n);
      target.inventory = {};
    }
    return true;
  }

  claimNeutralArea(world, cx, cy, radius) {
    if (this.clan) return;
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
        if (this.canExtendNeutralClaimTo(world, cell.x, cell.y, pending)) {
          pending.add(key);
          changed = true;
        }
      }
    }

    for (const key of pending) this.claimed.add(key);
  }

  canExtendNeutralClaimTo(world, x, y, pending = new Set()) {
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

  storeCarried(world) {
    const stockpile = this.clan && this.clan.nearestStockpile(this.x, this.y);
    if (!stockpile) return this.randomWalk(world);
    this.moveToward(world, stockpile.x, stockpile.y);
    if (this.x === stockpile.x && this.y === stockpile.y) this.depositResources(stockpile);
  }

  carriedResources() {
    return Object.values(this.inventory).reduce((a, b) => a + b, 0);
  }

  depositResources(stockpile) {
    for (const [type, n] of Object.entries(this.inventory)) {
      stockpile.resources[type] = (stockpile.resources[type] || 0) + n;
    }
    this.inventory = {};
  }

  // Spend 1 movement budget if available; moving a cell always costs 1.
  payMoveCost() {
    if (this.moveBudget < 1) return false;
    this.moveBudget -= 1;
    return true;
  }

  randomWalk(world) {
    if (!this.payMoveCost()) return;
    const dx = Math.floor(Math.random() * 3) - 1;
    const dy = Math.floor(Math.random() * 3) - 1;
    this.x = world.clamp(this.x + dx);
    this.y = world.clamp(this.y + dy);
  }

  moveToward(world, tx, ty) {
    if (this.x === tx && this.y === ty) return;
    if (!this.payMoveCost()) return;
    this.x = world.clamp(this.x + Math.sign(tx - this.x));
    this.y = world.clamp(this.y + Math.sign(ty - this.y));
  }

  addToInventory(type, amount) {
    this.inventory[type] = (this.inventory[type] || 0) + amount;
  }
}
