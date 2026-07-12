// Task: an order a leader issues to their clan. Members are assigned to open
// orders by Clan.assignTasks; "collect" is the standing default job.
//
// Food-seeking is NOT a task — it's the priority behaviour every entity runs
// when hunger crosses its personal threshold, preempting whatever task it has.
const TASK_DEFS = {
  collect:      { maxWorkers: Infinity }, // gather pellets, haul excess to the nearest stockpile
  defend:       { maxWorkers: 3 },        // patrol a w x h area of the grid
  claim:        { maxWorkers: 1 },        // walk to a cell and claim it (3x3) as town territory
  recruit:      { maxWorkers: 1, leaderOnly: true }, // leader walks to a neutral entity to recruit it
  scout:        { maxWorkers: 2, leaderCanWork: true }, // reveal a target area for the clan
  fightEntity:  { maxWorkers: 3 },        // attack a specific visible entity
  fightArea:    { maxWorkers: 4 },        // attack hostiles in a specific area
  hunt:         { maxWorkers: 2 },        // pursue a specific person by id/last known position
  moveBuilding: { maxWorkers: 1 },        // carry a stockpile to a new location
  harvest:      { maxWorkers: 3 },        // mine/chop resources, store at the nearest stockpile
  store:        { maxWorkers: 3 },        // deposit whatever resources you're carrying
  putResources: { maxWorkers: 2 },        // haul resources from other stockpiles to the one nearest X,Y
  putPellets:   { maxWorkers: 1 },        // build a new stockpile at X,Y
};

class Task {
  constructor(type, params = {}) {
    this.type = type;     // key of TASK_DEFS
    this.params = params; // e.g. {x, y}, {x, y, w, h}, {fromX, fromY, toX, toY}
    this.done = false;
  }

  get def() {
    return TASK_DEFS[this.type];
  }
}
