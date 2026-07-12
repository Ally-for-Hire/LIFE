// Tree: a durable food source. It periodically drops pellets nearby until
// destroyed by editing or future combat/building tools.
const TREE_DEFAULT_PELLETS = 5;
const TREE_DEFAULT_INTERVAL = 150;
const TREE_DEFAULT_RADIUS = 7;
const TREE_DEFAULT_HEALTH = 12;

class Tree {
  static nextId = 1;

  constructor(x, y, options = {}) {
    this.id = options.id !== undefined ? options.id : Tree.nextId++;
    Tree.nextId = Math.max(Tree.nextId, this.id + 1);
    this.x = x;
    this.y = y;
    this.pelletsPerCycle = options.pelletsPerCycle || TREE_DEFAULT_PELLETS;
    this.interval = options.interval || TREE_DEFAULT_INTERVAL;
    this.radius = options.radius || TREE_DEFAULT_RADIUS;
    this.health = options.health !== undefined ? options.health : TREE_DEFAULT_HEALTH;
    this.lastSpawnTick = options.lastSpawnTick || 0;
    this.destroyed = false;
  }

  update(world) {
    if (this.destroyed || world.tick - this.lastSpawnTick < this.interval) return;
    this.lastSpawnTick = world.tick;

    const claimed = [];
    const neutral = [];
    const open = [];
    for (let y = this.y - this.radius; y <= this.y + this.radius; y++) {
      for (let x = this.x - this.radius; x <= this.x + this.radius; x++) {
        if (!world.inBounds(x, y)) continue;
        if ((x - this.x) ** 2 + (y - this.y) ** 2 > this.radius ** 2) continue;
        if (world.pelletAt(x, y) || world.treeAt(x, y)) continue;

        const key = world.key(x, y);
        if (world.clans.some(clan => clan.claimed.has(key))) claimed.push({ x, y });
        else if (world.entities.some(e => !e.dead && !e.clan && e.claimed.has(key))) neutral.push({ x, y });
        else open.push({ x, y });
      }
    }

    const pool = claimed.length ? claimed : (neutral.length ? neutral : open);
    let spawned = 0;
    while (pool.length && spawned < this.pelletsPerCycle) {
      if (world.pellets.size >= world.maxPellets) break;
      const index = Math.floor(Math.random() * pool.length);
      const cell = pool.splice(index, 1)[0];
      world.addPellet(new Pellet(cell.x, cell.y));
      spawned++;
    }
  }

  damage(amount) {
    this.health -= amount;
    if (this.health <= 0) this.destroyed = true;
  }

  serialize() {
    return {
      id: this.id,
      x: this.x,
      y: this.y,
      pelletsPerCycle: this.pelletsPerCycle,
      interval: this.interval,
      radius: this.radius,
      health: this.health,
      lastSpawnTick: this.lastSpawnTick,
    };
  }
}
