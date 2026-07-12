//! Evolutionary trainer: evaluate a population of leader brains in headless
//! arenas, score by clan outcome, and evolve. Arenas are independent, so they
//! run across every CPU core via rayon — this is the "max CPU" path.
//!
//! Why CPU and not GPU: each arena is thousands of branchy integer-grid sim
//! ticks (pointer-chasing, not batched tensor math), and the per-clan nets are
//! tiny. That parallelizes perfectly across cores but would be far slower on a
//! GPU. The GPU is already busy rendering the live view.

use crate::brain::Brain;
use crate::rng::Rng;
use crate::world::{Params, World};
use rayon::prelude::*;
use std::time::Instant;

/// Default on-disk location of the evolved champion brain (relative to the run
/// directory). The app loads this on startup; the marathon trainer writes it.
pub const CHAMPION_PATH: &str = "champion.bin";

#[derive(Clone)]
pub struct TrainCfg {
    pub pop_size: usize,
    pub episode_ticks: i32,
    pub clans_per_arena: usize,
    pub repeats: usize,
    pub world_size: i32,
    pub arena_trees: i32,
    pub arena_neutrals: i32,
    pub mutation_rate: f32,
    pub mutation_strength: f32,
    pub elite: usize,
    pub seed: u64,
    pub arena_params: Params,
}

impl Default for TrainCfg {
    fn default() -> Self {
        // Arena economy mirrors the live "new design": farms make territory the
        // food source, wild food is sparse, seasons swing yields, and conflict is
        // reachable within an episode (short grace, low war threshold) so brains
        // are selected on settling, holding land, and fighting for it.
        let mut ap = Params::default();
        ap.max_pellet_fraction = 0.055;
        ap.tree_interval = 120;
        ap.tree_per_cycle = 4;
        ap.clan_grace_ticks = 800;
        ap.starve_ticks = 1400;
        ap.birth_chance = 0.025;
        ap.birth_interval = 180;
        ap.birth_food_cost = 4;
        TrainCfg {
            pop_size: 108,
            episode_ticks: 7000,
            clans_per_arena: 6,
            repeats: 4,
            world_size: 130,
            arena_trees: 110,
            arena_neutrals: 48,
            mutation_rate: 0.10,
            mutation_strength: 0.35,
            elite: 6,
            seed: 0x5EED,
            arena_params: ap,
        }
    }
}

pub struct Trainer {
    pub cfg: TrainCfg,
    pub population: Vec<Brain>,
    pub generation: u32,
    pub best_fitness: f32,
    pub avg_fitness: f32,
    pub best_ever: f32,
    pub best_brain: Option<Brain>,
    pub stagnant_generations: u32,
    pub adaptive_mutation_rate: f32,
    pub adaptive_mutation_strength: f32,
    pub history: Vec<[f64; 2]>,     // (generation, best fitness)
    pub avg_history: Vec<[f64; 2]>, // (generation, average fitness)
    pub last_gen_ms: f64,
    /// Curriculum stage: how wide the world-randomisation distribution is. Rises
    /// when fitness plateaus, so a stalled population gets harder/more varied
    /// worlds (and a bigger map "border") to master — generalising rather than
    /// overfitting one setup.
    pub stage: u32,
    stage_best: f32,
    stage_stall: u32,
    /// Hall of fame: strong past champions kept as frozen self-play opponents, so
    /// the evolving population must beat diverse strong strategies (not just its
    /// current peers) — the path to robust, real-opponent-level play.
    hof: Vec<Brain>,
    rng: Rng,
}

impl Trainer {
    pub fn new(cfg: TrainCfg) -> Self {
        let mut rng = Rng::new(cfg.seed ^ 0x00AB_CDEF);
        let population = (0..cfg.pop_size).map(|_| Brain::random(&mut rng)).collect();
        let adaptive_mutation_rate = cfg.mutation_rate;
        let adaptive_mutation_strength = cfg.mutation_strength;
        Trainer {
            cfg,
            population,
            generation: 0,
            best_fitness: 0.0,
            avg_fitness: 0.0,
            best_ever: f32::MIN,
            best_brain: None,
            stagnant_generations: 0,
            adaptive_mutation_rate,
            adaptive_mutation_strength,
            history: Vec::new(),
            avg_history: Vec::new(),
            last_gen_ms: 0.0,
            stage: 0,
            stage_best: f32::MIN,
            stage_stall: 0,
            hof: Vec::new(),
            rng,
        }
    }

    pub fn hof_len(&self) -> usize {
        self.hof.len()
    }

    pub fn snapshot_curriculum(&self) -> (u32, Vec<Brain>) {
        (self.stage, self.hof.clone())
    }

    fn push_hof(&mut self, b: Brain) {
        self.hof.push(b);
        if self.hof.len() > 16 {
            self.hof.remove(0); // drop the oldest (weakest) champion
        }
    }

    /// Like `finish_generation`, plus curriculum: track per-stage progress, keep a
    /// hall of fame of champions, and escalate the world-randomisation `stage`
    /// when fitness plateaus — so a stalled run gets harder, more varied worlds
    /// (and a larger map) to master instead of overfitting the current one.
    pub fn finish_general(&mut self, pop: Vec<Brain>, scores: Vec<f32>, ms: f64) {
        self.finish_generation(pop, scores, ms);
        let best = self.best_fitness;
        if self.stage_best == f32::MIN || best > self.stage_best * 1.02 {
            self.stage_best = best.max(self.stage_best);
            self.stage_stall = 0;
        } else {
            self.stage_stall += 1;
        }
        // bootstrap a couple of self-play opponents early
        if self.hof.len() < 3 {
            if let Some(b) = self.best_brain.clone() {
                self.push_hof(b);
            }
        }
        // plateaued at this stage → widen the world distribution (and the border)
        if self.stage_stall >= 20 && self.stage < MAX_STAGE {
            if let Some(b) = self.best_brain.clone() {
                self.push_hof(b);
            }
            self.stage += 1;
            self.stage_stall = 0;
            self.stage_best = f32::MIN;
        }
    }

    pub fn reset(&mut self) {
        let cfg = self.cfg.clone();
        *self = Trainer::new(cfg);
    }

    /// Save the current champion brain to disk (no-op if there isn't one yet).
    pub fn save_champion(&self, path: &str) -> std::io::Result<()> {
        match &self.best_brain {
            Some(b) => b.save(path),
            None => Ok(()),
        }
    }

    /// Seed the population from a saved champion (continue a prior training run):
    /// the loaded brain becomes the champion and a chunk of the population starts
    /// as mutated copies of it, the rest staying random for diversity.
    pub fn seed_from(&mut self, brain: Brain) {
        self.best_brain = Some(brain.clone());
        let keep = (self.cfg.pop_size / 3).max(1);
        for i in 0..self.population.len() {
            if i == 0 {
                self.population[i] = brain.clone();
            } else if i < keep {
                let mut c = brain.clone();
                c.mutate(&mut self.rng, self.cfg.mutation_rate, self.cfg.mutation_strength);
                self.population[i] = c;
            }
        }
    }
}

/// Run evolution headlessly for `hours` of wall-clock time, saving the champion
/// to `save_path` periodically (and on exit) and appending a progress line to
/// `log_path` each generation. If a champion already exists at `save_path`,
/// training continues from it. Designed for long unattended runs.
pub fn train_marathon(hours: f64, cfg: TrainCfg, save_path: &str, log_path: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut tr = Trainer::new(cfg);
    // The reigning benchmark champion (king-of-the-hill). Resume from disk if a
    // prior run left one, seeding the population from it too.
    let mut champion: Option<Brain> = None;
    let mut champ_score = f32::MIN;
    if let Ok(prev) = Brain::load(save_path) {
        tr.seed_from(prev.clone());
        champion = Some(prev);
    }
    const BENCH_EVERY: u32 = 6;
    const BENCH_WORLDS: usize = 24;
    const BENCH_SEED: u64 = 0xB3E2_5EED_1234_5678;
    // Append a line and flush it to physical disk, so the log survives a crash
    // or power loss right up to the last completed generation.
    let append = |path: &str, line: &str| {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
            let _ = f.sync_all();
        }
    };
    let started = Instant::now();
    let budget_secs = (hours * 3600.0).max(0.0);
    append(
        log_path,
        &format!(
            "=== marathon start: target {:.1}h, pop {}, episode {} ticks, {} rayon threads ===\n",
            hours,
            tr.cfg.pop_size,
            tr.cfg.episode_ticks,
            rayon::current_num_threads(),
        ),
    );
    let mut prev_stage = tr.stage;
    loop {
        if started.elapsed().as_secs_f64() >= budget_secs {
            break;
        }
        let pop = tr.population.clone();
        let gen = tr.generation;
        let stage = tr.stage;
        let hof = tr.hof.clone();
        let t0 = Instant::now();
        // domain-randomised, self-play evaluation across a distribution of worlds
        let scores = evaluate_general(
            &pop,
            &tr.cfg.arena_params,
            gen,
            stage,
            &hof,
            tr.cfg.seed,
            tr.cfg.episode_ticks,
            tr.cfg.clans_per_arena,
        );
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        tr.finish_general(pop, scores, ms);
        if tr.stage != prev_stage {
            append(
                log_path,
                &format!(
                    ">>> world escalated to stage {} (map border up to ~{} cells, harsher seasons, {} HoF opponents) <<<\n",
                    tr.stage,
                    (120 + 16 * tr.stage as i32).min(220),
                    tr.hof_len(),
                ),
            );
            prev_stage = tr.stage;
        }

        // King-of-the-hill champion: periodically benchmark the reigning champion
        // and this generation's best on the SAME fixed worlds, and keep the winner.
        // Because the benchmark is fixed, the saved champion only improves — never
        // frozen on an early lucky generation (the bug this run fixes). Saving the
        // champion is atomic + fsync'd (durable against crash / power loss).
        if champion.is_none() || champ_score == f32::MIN || tr.generation % BENCH_EVERY == 0 {
            let base = tr.cfg.arena_params.clone();
            let stage = tr.stage;
            let hofb = tr.hof.clone();
            let ep = tr.cfg.episode_ticks;
            let champ_now = champion
                .as_ref()
                .map(|c| benchmark_brain(c, &base, stage, &hofb, ep, BENCH_WORLDS, BENCH_SEED));
            let challenger = tr.best_brain.clone();
            let chal_now = challenger
                .as_ref()
                .map(|c| benchmark_brain(c, &base, stage, &hofb, ep, BENCH_WORLDS, BENCH_SEED));
            match (chal_now, champ_now) {
                (Some(hs), Some(cs)) if hs >= cs => {
                    champion = challenger;
                    champ_score = hs;
                }
                (Some(hs), None) => {
                    champion = challenger;
                    champ_score = hs;
                }
                (_, Some(cs)) => champ_score = cs,
                _ => {}
            }
            if let Some(c) = &champion {
                let _ = c.save(save_path);
                let _ = c.save(&format!("champion-stage{}.bin", tr.stage));
                tr.best_brain = Some(c.clone()); // keep the proven champion in the gene pool
            }
            append(
                log_path,
                &format!(
                    "    [benchmark] champion {:.0} on {} fixed worlds (stage {})\n",
                    champ_score, BENCH_WORLDS, tr.stage
                ),
            );
        }

        append(
            log_path,
            &format!(
                "gen {:>4}  stage {}  best {:>7.0}  avg {:>7.0}  champ {:>7.0}  hof {:>2}  gen_time {:>5.1}s  elapsed {:>5.1}m\n",
                tr.generation,
                tr.stage,
                tr.best_fitness,
                tr.avg_fitness,
                champ_score,
                tr.hof_len(),
                ms / 1000.0,
                started.elapsed().as_secs_f64() / 60.0,
            ),
        );
    }
    if let Some(c) = &champion {
        let _ = c.save(save_path);
    }
    append(
        log_path,
        &format!(
            "=== marathon done: {} generations in {:.2}h, champion {:.0} ===\n",
            tr.generation,
            started.elapsed().as_secs_f64() / 3600.0,
            champ_score,
        ),
    );
}

impl Trainer {
    /// Apply one finished generation's scores, record stats, and breed the next.
    pub fn finish_generation(&mut self, pop: Vec<Brain>, scores: Vec<f32>, ms: f64) {
        let mut ranked: Vec<(Brain, f32)> = pop.into_iter().zip(scores).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let best = ranked.first().map(|r| r.1).unwrap_or(0.0);
        let avg = if ranked.is_empty() {
            0.0
        } else {
            ranked.iter().map(|r| r.1).sum::<f32>() / ranked.len() as f32
        };
        self.best_fitness = best;
        self.avg_fitness = avg;
        // best_brain tracks the *current* generation's best. Under common-random-
        // numbers evaluation every brain faces the same worlds this generation, so
        // this is a fair pick — and it's never frozen on an early lucky generation
        // (the old bug). A monotonic, benchmark-validated champion for disk is kept
        // separately by the marathon (`benchmark_brain`).
        self.best_brain = ranked.first().map(|r| r.0.clone());
        let improvement_margin = self.best_ever.abs().max(1.0) * 0.002;
        if best > self.best_ever + improvement_margin {
            self.best_ever = best;
            self.stagnant_generations = 0;
        } else {
            self.stagnant_generations = self.stagnant_generations.saturating_add(1);
        }
        let g = self.generation as f64;
        self.history.push([g, best as f64]);
        self.avg_history.push([g, avg as f64]);
        if self.history.len() > 2000 {
            self.history.remove(0);
            self.avg_history.remove(0);
        }
        self.last_gen_ms = ms;
        self.generation += 1;
        self.evolve(ranked);
    }

    fn evolve(&mut self, ranked: Vec<(Brain, f32)>) {
        let pop_size = self.cfg.pop_size;
        let elite = self.cfg.elite.min(ranked.len());
        let mut next: Vec<Brain> = Vec::with_capacity(pop_size);
        let stagnation = self.stagnant_generations as f32;
        self.adaptive_mutation_rate =
            (self.cfg.mutation_rate * (1.0 + stagnation * 0.08)).clamp(0.02, 0.85);
        self.adaptive_mutation_strength =
            (self.cfg.mutation_strength * (1.0 + stagnation * 0.10)).clamp(0.02, 2.0);
        let immigrant_fraction = if self.stagnant_generations >= 24 {
            0.30
        } else if self.stagnant_generations >= 12 {
            0.18
        } else if self.stagnant_generations >= 6 {
            0.08
        } else {
            0.0
        };
        let immigrants = ((pop_size as f32) * immigrant_fraction).round() as usize;
        if let Some(best) = &self.best_brain {
            next.push(best.clone());
        }
        for r in ranked.iter().take(elite) {
            if next.len() < pop_size.saturating_sub(immigrants) {
                next.push(r.0.clone());
            }
        }
        if self.stagnant_generations >= 4 {
            if let Some(best) = &self.best_brain {
                let burst = (pop_size / 12).max(1);
                for _ in 0..burst {
                    if next.len() >= pop_size.saturating_sub(immigrants) {
                        break;
                    }
                    let mut child = best.clone();
                    child.mutate(
                        &mut self.rng,
                        self.adaptive_mutation_rate,
                        self.adaptive_mutation_strength * 1.5,
                    );
                    next.push(child);
                }
            }
        }
        while next.len() < pop_size.saturating_sub(immigrants) {
            let a = self.tournament(&ranked).clone();
            let b = self.tournament(&ranked).clone();
            let mut child = Brain::crossover(&a, &b, &mut self.rng);
            child.mutate(
                &mut self.rng,
                self.adaptive_mutation_rate,
                self.adaptive_mutation_strength,
            );
            next.push(child);
        }
        while next.len() < pop_size {
            let mut b = Brain::random(&mut self.rng);
            b.mutate(
                &mut self.rng,
                self.adaptive_mutation_rate,
                self.adaptive_mutation_strength,
            );
            next.push(b);
        }
        self.population = next;
    }

    fn tournament<'a>(&mut self, ranked: &'a [(Brain, f32)]) -> &'a Brain {
        let k = 3.min(ranked.len()).max(1);
        let mut best = 0usize;
        let mut best_fit = f32::MIN;
        for _ in 0..k {
            let i = self.rng.below(ranked.len() as i32) as usize;
            if ranked[i].1 > best_fit {
                best_fit = ranked[i].1;
                best = i;
            }
        }
        &ranked[best].0
    }
}

pub fn effective_repeats(pop_len: usize, cfg: &TrainCfg) -> usize {
    if pop_len == 0 {
        return 0;
    }
    let c = cfg.clans_per_arena.clamp(2, pop_len.max(2));
    let groups_per_repeat = ((pop_len + c - 1) / c).max(1);
    let target_groups = rayon::current_num_threads().max(1) * 8;
    let cpu_repeats = (target_groups + groups_per_repeat - 1) / groups_per_repeat;
    cfg.repeats.max(1).max(cpu_repeats)
}

pub fn arena_count(pop_len: usize, cfg: &TrainCfg) -> usize {
    if pop_len == 0 {
        return 0;
    }
    let c = cfg.clans_per_arena.clamp(2, pop_len.max(2));
    ((pop_len + c - 1) / c).max(1) * effective_repeats(pop_len, cfg)
}

/// Evaluate the whole population in parallel arenas; returns mean fitness per
/// brain index. Uses rayon's global pool, so it spans all CPU cores.
pub fn evaluate_parallel(pop: &[Brain], cfg: &TrainCfg, gen: u32) -> Vec<f32> {
    let n = pop.len();
    if n == 0 {
        return vec![];
    }
    let c = cfg.clans_per_arena.clamp(2, n.max(2));

    let mut groups: Vec<Vec<usize>> = Vec::new();
    for rep in 0..effective_repeats(n, cfg) {
        let mut idx: Vec<usize> = (0..n).collect();
        let mut rng = Rng::new(
            cfg.seed
                ^ (gen as u64).wrapping_mul(0x0000_0100_0000_01B3)
                ^ (rep as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
        );
        for i in (1..idx.len()).rev() {
            let j = rng.below((i + 1) as i32) as usize;
            idx.swap(i, j);
        }
        for chunk in idx.chunks(c) {
            if chunk.len() >= 2 {
                groups.push(chunk.to_vec());
            } else if let Some(g) = groups.last_mut() {
                g.push(chunk[0]); // attach a lone leftover to the previous arena
            }
        }
    }

    let results: Vec<Vec<(usize, f32)>> = groups
        .par_iter()
        .enumerate()
        .map(|(ai, g)| run_arena(pop, g, cfg, gen, ai))
        .collect();

    let mut sum = vec![0f32; n];
    let mut cnt = vec![0u32; n];
    for r in results {
        for (bi, s) in r {
            sum[bi] += s;
            cnt[bi] += 1;
        }
    }
    (0..n)
        .map(|i| if cnt[i] > 0 { sum[i] / cnt[i] as f32 } else { 0.0 })
        .collect()
}

fn run_arena(
    pop: &[Brain],
    group: &[usize],
    cfg: &TrainCfg,
    gen: u32,
    ai: usize,
) -> Vec<(usize, f32)> {
    let seed = cfg
        .seed
        .wrapping_add((gen as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add((ai as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
    let mut w = World::new(cfg.world_size, seed);
    w.params = cfg.arena_params.clone();
    let brains: Vec<Brain> = group.iter().map(|&i| pop[i].clone()).collect();
    let ids = w.setup_arena(&brains, cfg.arena_trees, cfg.arena_neutrals);
    for _ in 0..cfg.episode_ticks {
        w.step();
    }
    group
        .iter()
        .zip(ids)
        .map(|(&bi, cid)| (bi, score_clan(&w, cid)))
        .collect()
}

/// Highest curriculum stage — caps how large/harsh/varied worlds get. Raised so
/// the curriculum stays open-ended well past a "settled" champion: higher stages
/// keep widening the world distribution (combat lethality, scarcity, density,
/// metabolism, …), forcing ever-more-general mastery rather than overfitting.
pub const MAX_STAGE: u32 = 12;

/// A randomly drawn world for domain-randomised evaluation.
struct WorldSpec {
    params: Params,
    world_size: i32,
    trees: i32,
    neutrals: i32,
}

/// Draw a random world whose difficulty/variety scales with `stage`. The map
/// "border" (size) grows with stage, seasons can be harsher, food scarcer, and
/// terrain more varied — but the ranges still include easy worlds, so a brain
/// must stay good across the whole distribution (no overfitting one setup).
fn random_world_spec(base: &Params, rng: &mut Rng, stage: u32) -> WorldSpec {
    let s = stage as f32;
    let d = (s / MAX_STAGE as f32).min(1.0); // 0 (easy) .. 1 (hardest) difficulty knob
    let r = |rng: &mut Rng, lo: f32, hi: f32| lo + (hi - lo) * rng.f32();
    let mut p = base.clone();
    // Map "border" grows with stage (capped by the engine's practical max).
    let wmax = (120.0 + 14.0 * s).min(220.0);
    let world_size = r(rng, 96.0, wmax) as i32;
    // Food economy: scarcer and more variable as it gets harder.
    p.max_pellet_fraction = (r(rng, 0.03, 0.085) - 0.02 * d).max(0.015);
    p.farm_yield = r(rng, 0.08, 0.22);
    p.pellet_energy = r(rng, 7.0, 14.0) as i32;
    p.tree_per_cycle = r(rng, 2.0, 7.0) as i32;
    // Seasons: harsher swings at high stage.
    p.season_amp = r(rng, 0.2, (0.5 + 0.4 * d).min(0.92));
    p.season_length = r(rng, 1600.0, 3800.0) as i32;
    // Conflict: lower war threshold + deadlier, faster combat at high stage.
    p.war_threshold = (r(rng, 0.7, 1.4) - 0.4 * d).max(0.4);
    p.clan_grace_ticks = (r(rng, 300.0, 1400.0) - 700.0 * d).max(120.0) as i32;
    p.attack_damage = r(rng, 0.3, 0.55 + 0.45 * d);
    p.attack_cooldown = r(rng, 14.0, 28.0) as i32;
    // Metabolism / survival pressure.
    p.starve_ticks = (r(rng, 900.0, 1600.0) - 300.0 * d).max(500.0) as i32;
    p.vision_radius = r(rng, 11.0, 22.0) as i32;
    p.carry_limit = r(rng, 5.0, 16.0) as i32;
    p.home_range = r(rng, 18.0, 34.0) as i32;
    // Settlement density + growth.
    p.members_per_claim = r(rng, 1.0, 3.99) as i32;
    p.birth_chance = r(rng, 0.015, 0.04);
    p.birth_interval = r(rng, 140.0, 240.0) as i32;
    // Soil depletion: introduced gradually by the curriculum (off at low stages),
    // so brains first master the simple economy, then learn to rotate/expand land
    // as exhaustion bites at higher stages.
    p.soil_depletion_rate = if d > 0.25 { r(rng, 0.0, 0.8 * d) } else { 0.0 };
    // Regional disasters: only the hardest worlds, ramping with difficulty.
    p.disaster_rate = if d > 0.5 { r(rng, 0.0, 1.4 * (d - 0.5)) } else { 0.0 };
    // Terrain shape.
    p.water_level = r(rng, 0.22, 0.42);
    p.mountain_level = r(rng, 0.72, 0.88);
    let area = (world_size as f32) * (world_size as f32);
    let trees = (area * r(rng, 0.002, 0.006)) as i32;
    let neutrals = (area * r(rng, 0.001, 0.004)) as i32;
    WorldSpec {
        params: p,
        world_size,
        trees: trees.max(8),
        neutrals: neutrals.max(6),
    }
}

/// Run one randomised arena: `scored` brains compete alongside `opp` (frozen
/// hall-of-fame opponents). Returns a fitness for each `scored` brain only.
fn run_arena_general(
    scored: &[Brain],
    opp: &[Brain],
    spec: &WorldSpec,
    episode: i32,
    seed: u64,
) -> Vec<f32> {
    let mut w = World::new(spec.world_size, seed);
    w.params = spec.params.clone();
    let mut brains: Vec<Brain> = scored.to_vec();
    brains.extend_from_slice(opp);
    let ids = w.setup_arena(&brains, spec.trees, spec.neutrals);
    for _ in 0..episode {
        w.step();
    }
    ids.iter()
        .take(scored.len())
        .map(|&cid| score_clan(&w, cid))
        .collect()
}

/// Domain-randomised, self-play evaluation with **common random numbers**: this
/// generation draws ONE shared set of randomised worlds (+ fixed opponents per
/// world), and *every* brain is scored on the *same* worlds. That makes the
/// within-generation ranking fair and low-variance (so selection actually works),
/// while the worlds still rotate across generations and span stages `0..=stage`
/// (so brains must generalise, not overfit one map). A brain's fitness is its
/// mean over the shared worlds. Spans all cores via rayon.
pub fn evaluate_general(
    pop: &[Brain],
    base: &Params,
    gen: u32,
    stage: u32,
    hof: &[Brain],
    seed: u64,
    episode: i32,
    clans_per_arena: usize,
) -> Vec<f32> {
    let n = pop.len();
    if n == 0 {
        return vec![];
    }
    let p_per = clans_per_arena.clamp(2, n.max(2));
    let n_worlds = (6 + stage as usize / 2).min(10);

    // Pre-draw this generation's shared worlds (same for every brain = CRN).
    let worlds: Vec<(WorldSpec, Vec<Brain>)> = (0..n_worlds)
        .map(|wi| {
            let mut wr = Rng::new(
                seed ^ (gen as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    ^ (wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03),
            );
            let eff_stage = if stage == 0 {
                0
            } else {
                wr.below(stage as i32 + 1) as u32
            };
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let n_opp = if hof.is_empty() {
                0
            } else {
                (1 + eff_stage as usize / 2).min(3)
            };
            let opp: Vec<Brain> = (0..n_opp)
                .map(|_| hof[wr.below(hof.len() as i32) as usize].clone())
                .collect();
            (spec, opp)
        })
        .collect();

    // For each shared world, partition the whole population into arenas, so every
    // brain is scored once per world (faces all `n_worlds`).
    let mut tasks: Vec<(usize, Vec<usize>)> = Vec::new();
    for wi in 0..n_worlds {
        let mut idx: Vec<usize> = (0..n).collect();
        let mut sr = Rng::new(
            seed ^ (gen as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
                ^ (wi as u64).wrapping_mul(0x0000_0100_0000_01B3),
        );
        for i in (1..idx.len()).rev() {
            let j = sr.below((i + 1) as i32) as usize;
            idx.swap(i, j);
        }
        for chunk in idx.chunks(p_per) {
            tasks.push((wi, chunk.to_vec()));
        }
    }

    let results: Vec<Vec<(usize, f32)>> = tasks
        .par_iter()
        .enumerate()
        .map(|(ti, (wi, g))| {
            let (spec, opp) = &worlds[*wi];
            let scored: Vec<Brain> = g.iter().map(|&i| pop[i].clone()).collect();
            let aseed = seed
                .wrapping_add((gen as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                .wrapping_add((ti as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            let scores = run_arena_general(&scored, opp, spec, episode, aseed);
            g.iter().cloned().zip(scores).collect()
        })
        .collect();

    let mut sum = vec![0f32; n];
    let mut cnt = vec![0u32; n];
    for r in results {
        for (bi, s) in r {
            sum[bi] += s;
            cnt[bi] += 1;
        }
    }
    (0..n)
        .map(|i| if cnt[i] > 0 { sum[i] / cnt[i] as f32 } else { 0.0 })
        .collect()
}

/// Score one brain across a **fixed** benchmark of randomised worlds (constant
/// `seed`, spread across stages `0..=stage`), against `opponents`. Because the
/// worlds are fixed, two brains benchmarked with the same args are directly
/// comparable — this is how a *monotonic* champion is chosen, immune to the
/// per-generation luck that froze the old champion. Parallel across worlds.
pub fn benchmark_brain(
    brain: &Brain,
    base: &Params,
    stage: u32,
    opponents: &[Brain],
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> f32 {
    if n_worlds == 0 {
        return 0.0;
    }
    let scores: Vec<f32> = (0..n_worlds)
        .into_par_iter()
        .map(|wi| {
            let mut wr = Rng::new(seed ^ (wi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let eff_stage = (wi as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let aseed = seed.wrapping_add((wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            run_arena_general(std::slice::from_ref(brain), opponents, &spec, episode, aseed)[0]
        })
        .collect();
    scores.iter().sum::<f32>() / scores.len() as f32
}

/// Fitness from a clan's final state. Kept as a smooth weighted sum (no hard
/// cliffs) so the gradient is learnable; a wiped-out clan scores 0.
fn score_clan(w: &World, cid: i32) -> f32 {
    match w.clan_by_id(cid) {
        Some(c) => {
            let pop = w.clan_population(cid) as f32;
            let food = c.food as f32;
            let terr = c.territory as f32;
            let kills = c.stats.kills as f32;
            let losses = c.stats.losses as f32;
            let recruits = c.stats.recruits as f32;
            let peak = c.stats.peak_pop as f32;
            let alive_ticks = c.stats.alive_ticks.max(1) as f32;
            let possible_ticks = (w.tick - c.stats.founded_tick).max(1) as f32;
            let survival = (alive_ticks / possible_ticks).clamp(0.0, 1.0);
            let avg_pop = c.stats.pop_tick_sum as f32 / alive_ticks;
            let avg_food = c.stats.food_tick_sum as f32 / alive_ticks;
            let avg_hunger = if c.stats.pop_tick_sum > 0 {
                c.stats.hunger_tick_sum / c.stats.pop_tick_sum as f32
            } else {
                1.0
            };
            let reserve_per_cap = food / pop.max(1.0);
            let fertile_cap = c.fertile_capacity;
            // Fraction of member-time spent on the clan's own land: the direct
            // measure of "settled village that uses its territory."
            let settled_frac = if c.stats.pop_tick_sum > 0 {
                c.stats.on_terr_tick_sum as f32 / c.stats.pop_tick_sum as f32
            } else {
                0.0
            };
            let group_multiplier = if pop >= 2.0 {
                1.0 + (pop - 1.0).min(30.0) * 0.035
            } else {
                0.25
            };
            let survival_score = survival * 120.0;
            let population_score = pop.powf(1.15) * 7.0 + avg_pop * 4.0 + peak * 2.0;
            // Living ON your land beats merely claiming it — this is the village
            // reward, and the biggest single term for a stable settled clan.
            let settled_score = settled_frac * avg_pop * 8.0;
            // Held productive land matters, but with diminishing returns so the
            // optimum is "enough fertile land to feed the village," not grab-all.
            let land_score = fertile_cap.sqrt() * 8.0 + terr.sqrt() * 2.0;
            let reserve_score = reserve_per_cap.min(12.0) * 5.0 + avg_food.sqrt() * 2.0;
            let cooperation_score = recruits * 5.0;
            // Winning land/raids pays; losing warriors costs, but less than before
            // (some losses are the price of taking a neighbour's valley).
            let combat_score = kills * 1.2;
            let hunger_penalty = avg_hunger * 40.0 + c.stats.starving_ticks as f32 * 0.06;
            let loss_penalty = losses * 18.0;
            ((survival_score
                + population_score
                + settled_score
                + land_score
                + reserve_score
                + cooperation_score
                + combat_score)
                * group_multiplier
                - hunger_penalty
                - loss_penalty)
                .max(0.0)
        }
        None => 0.0, // disbanded / wiped out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_runs_several_generations() {
        let mut cfg = TrainCfg::default();
        cfg.pop_size = 12;
        cfg.episode_ticks = 1500;
        cfg.clans_per_arena = 4;
        cfg.repeats = 1;
        cfg.world_size = 90;
        cfg.arena_trees = 50;
        cfg.arena_neutrals = 20;

        let mut tr = Trainer::new(cfg);
        for _ in 0..6 {
            let pop = tr.population.clone();
            let gen = tr.generation;
            let scores = evaluate_parallel(&pop, &tr.cfg, gen);
            tr.finish_generation(pop, scores, 0.0);
        }
        println!(
            "after 6 gens: best={:.1} avg={:.1} best_ever={:.1}",
            tr.best_fitness, tr.avg_fitness, tr.best_ever
        );
        assert_eq!(tr.generation, 6);
        assert!(tr.best_ever.is_finite() && tr.best_ever > 0.0);
        assert!(tr.best_brain.is_some());
        assert_eq!(tr.population.len(), 12);
    }
}
