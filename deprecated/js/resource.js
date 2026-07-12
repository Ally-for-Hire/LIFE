// Resource: a harvestable material on the grid (wood, stone, ...).
const RESOURCE_TYPES = {
  wood:  { color: "#8b5a2b" },
  stone: { color: "#9a9a9a" },
};

class Resource {
  constructor(x, y, type, amount = 50) {
    this.x = x;
    this.y = y;
    this.type = type; // key into RESOURCE_TYPES
    this.amount = amount;
  }

  // Take up to n units; returns how much was actually taken.
  harvest(n = 1) {
    const taken = Math.min(n, this.amount);
    this.amount -= taken;
    return taken;
  }

  get depleted() {
    return this.amount <= 0;
  }
}
