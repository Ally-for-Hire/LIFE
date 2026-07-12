// Stockpile: a town building that stores shared food and resources.
// A clan's first stockpile turns it into a town; towns can build more.
class Stockpile {
  constructor(x, y) {
    this.x = x;
    this.y = y;
    this.food = 0;
    this.resources = {}; // e.g. { wood: 12, stone: 4 }
  }

  totalResources() {
    return Object.values(this.resources).reduce((a, b) => a + b, 0);
  }
}
