//! Evolutionary trainer: evaluate a population of leader brains in headless
//! arenas, score by clan outcome, and evolve. Arenas are independent, so they
//! run across every CPU core via rayon — this is the "max CPU" path.
//!
//! Why CPU and not GPU: each arena is thousands of branchy integer-grid sim
//! ticks (pointer-chasing, not batched tensor math), and the per-clan nets are
//! tiny. That parallelizes perfectly across cores but would be far slower on a
//! GPU. The GPU is already busy rendering the live view.

use crate::brain::Brain;
use crate::quality::{
    routing_metrics, score_clan as score_clan_quality, QualityScore, StrategyNiche, FAIRNESS_FLOOR,
    N_STRATEGY_NICHES, SECURITY_FLOOR, SURVIVAL_FLOOR,
};
use crate::rng::Rng;
use crate::world::{Params, World};
use rayon::prelude::*;
use std::collections::HashSet;
#[cfg(test)]
use std::time::Instant;

/// Default on-disk location of the evolved champion brain (relative to the run
/// directory). The app loads this on startup; the marathon trainer writes it.
pub const CHAMPION_PATH: &str = "champion.bin";

#[cfg(test)]
const PROMOTION_SURVIVAL_FLOOR: f32 = 0.95;
#[cfg(test)]
const PROMOTION_SECURITY_FLOOR: f32 = 0.80;
#[cfg(test)]
const PROMOTION_FAIRNESS_FLOOR: f32 = 0.0;
#[cfg(test)]
const PROMOTION_ROUTING_FLOOR: f32 = 0.10;
#[cfg(test)]
const PROMOTION_EXPERT_COVERAGE_FLOOR: f32 = 0.50;
#[cfg(test)]
const PROMOTION_SECURITY_TOLERANCE: f32 = 0.01;
#[cfg(test)]
const PROMOTION_ROUTING_TOLERANCE: f32 = 0.02;
#[cfg(test)]
const PROMOTION_COVERAGE_TOLERANCE: f32 = 0.05;
#[cfg(test)]
const PROMOTION_LOGISTICS_TOLERANCE: f32 = 0.02;
#[cfg(test)]
const PROMOTION_TRADE_TOLERANCE: f32 = 0.05;

#[derive(Clone)]
struct QdElite {
    brain: Brain,
    quality: QualityScore,
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct AiBenchmarkReport {
    pub worlds: usize,
    pub mean_fitness: f32,
    pub robust_survival: f32,
    pub mean_security: f32,
    pub clan_cohort_survival: f32,
    pub neutral_cohort_survival: f32,
    pub fairness_delta: f32,
    pub robust_fairness_delta: f32,
    pub mean_logistics: f32,
    pub mean_hauling_throughput: f32,
    pub mean_road_utility: f32,
    pub mean_reserve_security: f32,
    pub mean_task_coverage: f32,
    pub mean_care: f32,
    pub mean_trade: f32,
    pub routing_entropy: f32,
    pub expert_coverage: f32,
    pub eligible: bool,
}

/// One arm of a paired logistics benchmark. Values are means across identical
/// generated worlds; robust survival/fairness retain their worst-world gates.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct LogisticsBenchmarkArm {
    pub robust_survival: f32,
    pub mean_security: f32,
    pub clan_cohort_survival: f32,
    pub neutral_cohort_survival: f32,
    pub fairness_delta: f32,
    pub robust_fairness_delta: f32,
    pub hauling_throughput: f32,
    pub road_utility: f32,
    pub reserve_use: f32,
    pub reserve_security: f32,
    pub eligible: bool,
}

/// Deterministic same-world logistics-on/off ablation. This is an explicit
/// benchmark and is never invoked by ordinary generation evaluation, so it
/// does not double normal training cost.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct LogisticsBenchmarkReport {
    pub worlds: usize,
    pub enabled: LogisticsBenchmarkArm,
    pub disabled: LogisticsBenchmarkArm,
    pub clan_survival_delta: f32,
    pub security_delta: f32,
    pub hauling_throughput_delta: f32,
    pub road_utility_delta: f32,
    pub reserve_use_delta: f32,
    pub reserve_security_delta: f32,
    pub survival_non_regression: bool,
}

/// One arm of the deterministic Community Care on/off benchmark.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct CareBenchmarkArm {
    pub robust_survival: f32,
    pub mean_security: f32,
    pub clan_cohort_survival: f32,
    pub neutral_cohort_survival: f32,
    pub fairness_delta: f32,
    pub robust_fairness_delta: f32,
    pub rescue_rate: f32,
    pub incapacitations: f32,
    pub rescues: f32,
    pub bleedouts: f32,
    pub combat_deaths: f32,
    pub eligible: bool,
}

/// Same-world care treatment/control report. The disabled arm keeps every
/// economy and policy input identical but converts lethal combat directly to death.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct CareBenchmarkReport {
    pub worlds: usize,
    pub enabled: CareBenchmarkArm,
    pub disabled: CareBenchmarkArm,
    pub clan_survival_delta: f32,
    pub security_delta: f32,
    pub combat_death_delta: f32,
    pub survival_non_regression: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct TradeBenchmarkArm {
    pub robust_survival: f32,
    pub mean_security: f32,
    pub clan_cohort_survival: f32,
    pub neutral_cohort_survival: f32,
    pub fairness_delta: f32,
    pub robust_fairness_delta: f32,
    pub trade_value: f32,
    pub delivered_food: f32,
    pub delivered_wood: f32,
    pub deliveries: f32,
    pub eligible: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct TradeBenchmarkReport {
    pub worlds: usize,
    pub enabled: TradeBenchmarkArm,
    pub disabled: TradeBenchmarkArm,
    pub clan_survival_delta: f32,
    pub security_delta: f32,
    pub delivered_material_delta: f32,
    pub survival_non_regression: bool,
}

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
    pub robust_survival: f32,
    pub mean_security: f32,
    pub fairness_margin: f32,
    pub mean_logistics: f32,
    pub mean_hauling_throughput: f32,
    pub mean_road_utility: f32,
    pub mean_reserve_security: f32,
    pub mean_task_coverage: f32,
    pub mean_care: f32,
    pub mean_trade: f32,
    pub routing_balance: f32,
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
    qd_archive: Vec<Option<QdElite>>,
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
            robust_survival: 0.0,
            mean_security: 0.0,
            fairness_margin: 0.0,
            mean_logistics: 0.0,
            mean_hauling_throughput: 0.0,
            mean_road_utility: 0.0,
            mean_reserve_security: 0.0,
            mean_task_coverage: 0.0,
            mean_care: 0.0,
            mean_trade: 0.0,
            routing_balance: 0.0,
            history: Vec::new(),
            avg_history: Vec::new(),
            last_gen_ms: 0.0,
            stage: 0,
            stage_best: f32::MIN,
            stage_stall: 0,
            hof: Vec::new(),
            qd_archive: vec![None; N_STRATEGY_NICHES],
            rng,
        }
    }

    pub fn hof_len(&self) -> usize {
        self.hof.len()
    }

    pub fn qd_archive_len(&self) -> usize {
        self.qd_archive.iter().flatten().count()
    }

    pub fn qd_archive_summary(&self) -> Vec<(&'static str, f32)> {
        StrategyNiche::ALL
            .iter()
            .enumerate()
            .filter_map(|(i, niche)| {
                self.qd_archive[i]
                    .as_ref()
                    .map(|elite| (niche.label(), elite.quality.niche_quality(*niche)))
            })
            .collect()
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
    pub fn finish_general(&mut self, pop: Vec<Brain>, scores: Vec<QualityScore>, ms: f64) {
        self.finish_quality_generation(pop, scores, ms);
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
                c.mutate(
                    &mut self.rng,
                    self.cfg.mutation_rate,
                    self.cfg.mutation_strength,
                );
                self.population[i] = c;
            }
        }
    }
}

/// Run evolution headlessly for `hours` of wall-clock time, saving the champion
/// to `save_path` periodically (and on exit) and appending a progress line to
/// `log_path` each generation. If a champion already exists at `save_path`,
/// training continues from it. Designed for long unattended runs.
#[cfg(test)]
pub fn train_marathon(hours: f64, cfg: TrainCfg, save_path: &str, log_path: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut tr = Trainer::new(cfg);
    // The reigning benchmark champion (king-of-the-hill). Resume from disk if a
    // prior run left one, seeding the population from it too.
    let mut champion: Option<Brain> = None;
    let mut champ_score = f32::MIN;
    let mut champ_quality: Option<QualityScore> = None;
    if let Ok(prev) = Brain::load(save_path) {
        tr.seed_from(prev.clone());
        champion = Some(prev);
    }
    const BENCH_EVERY: u32 = 6;
    const BENCH_WORLDS: usize = 24;
    const BENCH_SEED: u64 = 0xB3E2_5EED_1234_5678;
    const LOGISTICS_GATE_WORLDS: usize = 13;
    const LOGISTICS_GATE_EPISODE: i32 = 4000;
    const LOGISTICS_GATE_SEED: u64 = 0x51FE_BEEF;
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
        let scores = evaluate_general_quality(
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
        // and this generation's best on the SAME fixed worlds. A headline winner
        // must then pass the paired logistics and trade gates before it can replace the saved
        // brain. Saving remains atomic + fsync'd (durable against crash / power loss).
        if champion.is_none() || champ_score == f32::MIN || tr.generation % BENCH_EVERY == 0 {
            let base = tr.cfg.arena_params.clone();
            let stage = tr.stage;
            let hofb = tr.hof.clone();
            let ep = tr.cfg.episode_ticks;
            let champ_now = champion
                .as_ref()
                .map(|c| benchmark_quality(c, &base, stage, &hofb, ep, BENCH_WORLDS, BENCH_SEED));
            let challenger = tr.best_brain.clone();
            let chal_now = challenger
                .as_ref()
                .map(|c| benchmark_quality(c, &base, stage, &hofb, ep, BENCH_WORLDS, BENCH_SEED));
            let mut gate_result = None;
            match (chal_now, champ_now) {
                (Some(hq), reigning) if reigning.is_none_or(|cq| quality_better(&hq, &cq)) => {
                    let gate_episode = ep.min(LOGISTICS_GATE_EPISODE).max(1);
                    let challenger_logistics = benchmark_logistics_quality(
                        challenger
                            .as_ref()
                            .expect("benchmarked challenger should exist"),
                        &base,
                        stage,
                        gate_episode,
                        LOGISTICS_GATE_WORLDS,
                        LOGISTICS_GATE_SEED,
                    );
                    let reigning_logistics = champion.as_ref().map(|brain| {
                        benchmark_logistics_quality(
                            brain,
                            &base,
                            stage,
                            gate_episode,
                            LOGISTICS_GATE_WORLDS,
                            LOGISTICS_GATE_SEED,
                        )
                    });
                    let challenger_trade = benchmark_trade_quality(
                        challenger
                            .as_ref()
                            .expect("benchmarked challenger should exist"),
                        &base,
                        stage,
                        gate_episode,
                        LOGISTICS_GATE_WORLDS,
                        0x7ADE_BEEF,
                    );
                    let reigning_trade = champion.as_ref().map(|brain| {
                        benchmark_trade_quality(
                            brain,
                            &base,
                            stage,
                            gate_episode,
                            LOGISTICS_GATE_WORLDS,
                            0x7ADE_BEEF,
                        )
                    });
                    let rejections = champion_promotion_rejections(
                        &hq,
                        reigning.as_ref(),
                        &challenger_logistics,
                        reigning_logistics.as_ref(),
                        &challenger_trade,
                        reigning_trade.as_ref(),
                    );
                    let accepted = rejections.is_empty();
                    gate_result =
                        Some((accepted, rejections, challenger_logistics, challenger_trade));
                    if accepted {
                        champion = challenger;
                        champ_score = hq.fitness;
                        champ_quality = Some(hq);
                    } else if let Some(cq) = reigning {
                        champ_score = cq.fitness;
                        champ_quality = Some(cq);
                    }
                }
                (_, Some(cq)) => {
                    champ_score = cq.fitness;
                    champ_quality = Some(cq);
                }
                _ => {}
            }
            if let Some(c) = &champion {
                let _ = c.save(save_path);
                let _ = c.save(&format!("champion-stage{}.bin", tr.stage));
                tr.best_brain = Some(c.clone()); // keep the proven champion in the gene pool
            }
            let cq = champ_quality.unwrap_or_default();
            append(
                log_path,
                &format!(
                    "    [benchmark] champion {:.0} survival {:.2} security {:.2} logistics {:.2} haul {:.2} roads {:.2} reserve {:.2} tasks {:.2} care {:.2} trade {:.2} routing {:.2}/{:.2} on {} fixed worlds (stage {})\n",
                    champ_score,
                    cq.robust_survival,
                    cq.security,
                    cq.logistics,
                    cq.hauling_throughput,
                    cq.road_utility,
                    cq.reserve_security,
                    cq.task_coverage,
                    cq.care,
                    cq.trade,
                    cq.routing_entropy,
                    cq.expert_coverage,
                    BENCH_WORLDS,
                    tr.stage
                ),
            );
            if let Some((accepted, rejections, logistics, trade)) = gate_result {
                let verdict = if accepted { "accepted" } else { "rejected" };
                append(
                    log_path,
                    &format!(
                        "    [promotion gate] {verdict}: paired survival {:+.3}, security {:+.3}, haul {:+.3}, roads {:+.3}, reserve use {:+.3}; reasons: {}\n",
                        logistics.clan_survival_delta,
                        logistics.security_delta,
                        logistics.hauling_throughput_delta,
                        logistics.road_utility_delta,
                        logistics.reserve_use_delta,
                        if rejections.is_empty() {
                            "none".to_owned()
                        } else {
                            rejections.join(", ")
                        },
                    ),
                );
                append(
                    log_path,
                    &format!(
                        "    [trade gate] {verdict}: paired survival {:+.3}, security {:+.3}, delivered {:+.1}, trips {:.1}\n",
                        trade.clan_survival_delta,
                        trade.security_delta,
                        trade.delivered_material_delta,
                        trade.enabled.deliveries,
                    ),
                );
            }
        }

        append(
            log_path,
            &format!(
                "gen {:>4}  stage {}  best {:>7.0}  avg {:>7.0}  survival {:.2}  fairness {:+.2}  niches {}/{}  champ {:>7.0}  hof {:>2}  gen_time {:>5.1}s  elapsed {:>5.1}m\n",
                tr.generation,
                tr.stage,
                tr.best_fitness,
                tr.avg_fitness,
                tr.robust_survival,
                tr.fairness_margin,
                tr.qd_archive_len(),
                N_STRATEGY_NICHES,
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
    #[cfg(test)]
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

    /// Apply survival-gated multi-metric results, update the persistent niche
    /// archive, then breed from both strong generalists and distinct specialists.
    fn finish_quality_generation(&mut self, pop: Vec<Brain>, scores: Vec<QualityScore>, ms: f64) {
        let mut ranked: Vec<(Brain, QualityScore)> = pop.into_iter().zip(scores).collect();
        ranked.sort_by(|a, b| {
            b.1.selection_score()
                .partial_cmp(&a.1.selection_score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.update_qd_archive(&ranked);
        let best_idx = ranked
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.1.eligible)
            .max_by(|(_, a), (_, b)| {
                a.1.fitness
                    .partial_cmp(&b.1.fitness)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        let best_quality = ranked.get(best_idx).map(|r| r.1).unwrap_or_default();
        let avg = if ranked.is_empty() {
            0.0
        } else {
            ranked.iter().map(|r| r.1.fitness).sum::<f32>() / ranked.len() as f32
        };

        self.best_fitness = best_quality.fitness;
        self.avg_fitness = avg;
        self.best_brain = ranked.get(best_idx).map(|r| r.0.clone());
        self.robust_survival = best_quality.robust_survival;
        self.mean_security = best_quality.security;
        self.fairness_margin = best_quality.robust_fairness;
        self.mean_logistics = best_quality.logistics;
        self.mean_hauling_throughput = best_quality.hauling_throughput;
        self.mean_road_utility = best_quality.road_utility;
        self.mean_reserve_security = best_quality.reserve_security;
        self.mean_task_coverage = best_quality.task_coverage;
        self.mean_care = best_quality.care;
        self.mean_trade = best_quality.trade;
        self.routing_balance = best_quality.routing_entropy * best_quality.expert_coverage;
        let improvement_margin = self.best_ever.abs().max(1.0) * 0.002;
        if self.best_fitness > self.best_ever + improvement_margin {
            self.best_ever = self.best_fitness;
            self.stagnant_generations = 0;
        } else {
            self.stagnant_generations = self.stagnant_generations.saturating_add(1);
        }
        let g = self.generation as f64;
        self.history.push([g, self.best_fitness as f64]);
        self.avg_history.push([g, avg as f64]);
        if self.history.len() > 2000 {
            self.history.remove(0);
            self.avg_history.remove(0);
        }
        self.last_gen_ms = ms;
        self.generation += 1;
        self.evolve_quality(ranked);
    }

    fn update_qd_archive(&mut self, ranked: &[(Brain, QualityScore)]) {
        let mut used = HashSet::new();
        for niche in StrategyNiche::ALL {
            let candidate = ranked
                .iter()
                .enumerate()
                .filter(|(i, (_, q))| !used.contains(i) && q.qualifies_for(niche))
                .max_by(|(_, a), (_, b)| {
                    let aq = a.1.niche_quality(niche);
                    let bq = b.1.niche_quality(niche);
                    aq.partial_cmp(&bq)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| {
                            a.1.fitness
                                .partial_cmp(&b.1.fitness)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });
            let Some((candidate_idx, (brain, quality))) = candidate else {
                continue;
            };
            used.insert(candidate_idx);
            let slot = &mut self.qd_archive[niche.index()];
            let replace = slot.as_ref().is_none_or(|old| {
                let new_niche = quality.niche_quality(niche);
                let old_niche = old.quality.niche_quality(niche);
                new_niche > old_niche + 0.002
                    || ((new_niche - old_niche).abs() <= 0.002
                        && quality.fitness > old.quality.fitness)
            });
            if replace {
                *slot = Some(QdElite {
                    brain: brain.clone(),
                    quality: *quality,
                });
            }
        }
    }

    fn evolve_quality(&mut self, ranked: Vec<(Brain, QualityScore)>) {
        let pop_size = self.cfg.pop_size;
        if pop_size == 0 || ranked.is_empty() {
            self.population.clear();
            return;
        }
        let mut next = Vec::with_capacity(pop_size);
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
        let breeding_limit = pop_size.saturating_sub(immigrants);

        if let Some(best) = &self.best_brain {
            next.push(best.clone());
        }
        for elite in self.qd_archive.iter().flatten() {
            if next.len() >= breeding_limit {
                break;
            }
            next.push(elite.brain.clone());
        }
        for candidate in ranked.iter().take(self.cfg.elite.min(ranked.len())) {
            if next.len() >= breeding_limit {
                break;
            }
            next.push(candidate.0.clone());
        }
        if self.stagnant_generations >= 4 {
            for elite in self.qd_archive.iter().flatten() {
                if next.len() >= breeding_limit {
                    break;
                }
                let mut child = elite.brain.clone();
                child.mutate(
                    &mut self.rng,
                    self.adaptive_mutation_rate,
                    self.adaptive_mutation_strength * 1.35,
                );
                next.push(child);
            }
        }
        while next.len() < breeding_limit {
            let a = self.tournament_quality(&ranked).clone();
            let b = self.tournament_quality(&ranked).clone();
            let mut child = Brain::crossover(&a, &b, &mut self.rng);
            child.mutate(
                &mut self.rng,
                self.adaptive_mutation_rate,
                self.adaptive_mutation_strength,
            );
            next.push(child);
        }
        while next.len() < pop_size {
            let mut brain = Brain::random(&mut self.rng);
            brain.mutate(
                &mut self.rng,
                self.adaptive_mutation_rate,
                self.adaptive_mutation_strength,
            );
            next.push(brain);
        }
        self.population = next;
    }

    fn tournament_quality<'a>(&mut self, ranked: &'a [(Brain, QualityScore)]) -> &'a Brain {
        let k = 3.min(ranked.len()).max(1);
        let mut best = 0usize;
        let mut best_score = f32::MIN;
        for _ in 0..k {
            let i = self.rng.below(ranked.len() as i32) as usize;
            let score = ranked[i].1.selection_score();
            if score > best_score {
                best_score = score;
                best = i;
            }
        }
        &ranked[best].0
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
#[cfg(test)]
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
        .map(|i| {
            if cnt[i] > 0 {
                sum[i] / cnt[i] as f32
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
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

#[derive(Clone, Copy)]
struct QualityTotals {
    sum: QualityScore,
    robust_survival: f32,
    robust_fairness: f32,
    count: u32,
}

impl Default for QualityTotals {
    fn default() -> Self {
        QualityTotals {
            sum: QualityScore::default(),
            robust_survival: 1.0,
            robust_fairness: 1.0,
            count: 0,
        }
    }
}

impl QualityTotals {
    fn add(&mut self, score: QualityScore) {
        self.sum.fitness += score.fitness;
        self.sum.survival += score.survival;
        self.sum.security += score.security;
        self.sum.fairness += score.fairness;
        self.sum.settlement += score.settlement;
        self.sum.expansion += score.expansion;
        self.sum.cooperation += score.cooperation;
        self.sum.logistics += score.logistics;
        self.sum.hauling_throughput += score.hauling_throughput;
        self.sum.road_utility += score.road_utility;
        self.sum.reserve_security += score.reserve_security;
        self.sum.task_coverage += score.task_coverage;
        self.sum.care += score.care;
        self.sum.trade += score.trade;
        self.sum.defense += score.defense;
        self.sum.combat += score.combat;
        self.robust_survival = self.robust_survival.min(score.survival);
        self.robust_fairness = self.robust_fairness.min(score.fairness);
        self.count += 1;
    }

    fn finish(self, brain: &Brain) -> QualityScore {
        if self.count == 0 {
            return QualityScore::default();
        }
        let inv = 1.0 / self.count as f32;
        let (routing_entropy, expert_coverage) = routing_metrics(brain);
        let survival = self.sum.survival * inv;
        let security = self.sum.security * inv;
        let fairness = self.sum.fairness * inv;
        QualityScore {
            fitness: self.sum.fitness * inv,
            survival,
            robust_survival: self.robust_survival,
            security,
            fairness,
            robust_fairness: self.robust_fairness,
            settlement: self.sum.settlement * inv,
            expansion: self.sum.expansion * inv,
            cooperation: self.sum.cooperation * inv,
            logistics: self.sum.logistics * inv,
            hauling_throughput: self.sum.hauling_throughput * inv,
            road_utility: self.sum.road_utility * inv,
            reserve_security: self.sum.reserve_security * inv,
            task_coverage: self.sum.task_coverage * inv,
            care: self.sum.care * inv,
            trade: self.sum.trade * inv,
            defense: self.sum.defense * inv,
            combat: self.sum.combat * inv,
            routing_entropy,
            expert_coverage,
            eligible: self.robust_survival >= SURVIVAL_FLOOR
                && security >= SECURITY_FLOOR
                && self.robust_fairness >= FAIRNESS_FLOOR,
        }
    }
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
    p.disaster_rate = if d > 0.5 {
        r(rng, 0.0, 1.4 * (d - 0.5))
    } else {
        0.0
    };
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
) -> Vec<QualityScore> {
    let mut w = World::new(spec.world_size, seed);
    w.params = spec.params.clone();
    let mut brains: Vec<Brain> = scored.to_vec();
    brains.extend_from_slice(opp);
    let ids = w.setup_arena(&brains, spec.trees, spec.neutrals);
    let clan_cohorts: Vec<HashSet<u32>> = ids
        .iter()
        .take(scored.len())
        .map(|cid| {
            w.entities
                .iter()
                .filter(|entity| entity.clan == *cid)
                .map(|entity| entity.id)
                .collect()
        })
        .collect();
    let neutral_cohort: HashSet<u32> = w
        .entities
        .iter()
        .filter(|entity| entity.clan < 0)
        .map(|entity| entity.id)
        .collect();
    for _ in 0..episode {
        w.step();
    }
    let alive: HashSet<u32> = w
        .entities
        .iter()
        .filter(|entity| entity.is_active())
        .map(|entity| entity.id)
        .collect();
    let cohort_ratio = |cohort: &HashSet<u32>| {
        if cohort.is_empty() {
            1.0
        } else {
            cohort.iter().filter(|id| alive.contains(id)).count() as f32 / cohort.len() as f32
        }
    };
    let neutral_survival = cohort_ratio(&neutral_cohort);
    ids.iter()
        .take(scored.len())
        .zip(clan_cohorts)
        .map(|(&cid, cohort)| {
            let mut quality = score_clan_quality(&w, cid);
            quality.fairness = cohort_ratio(&cohort) - neutral_survival;
            quality.robust_fairness = quality.fairness;
            quality.eligible &= quality.fairness >= FAIRNESS_FLOOR;
            quality
        })
        .collect()
}

/// Domain-randomised, self-play evaluation with **common random numbers**: this
/// generation draws ONE shared set of randomised worlds (+ fixed opponents per
/// world), and *every* brain is scored on the *same* worlds. That makes the
/// within-generation ranking fair and low-variance (so selection actually works),
/// while the worlds still rotate across generations and span stages `0..=stage`
/// (so brains must generalise, not overfit one map). A brain's fitness is its
/// mean over the shared worlds. Spans all cores via rayon.
pub fn evaluate_general_quality(
    pop: &[Brain],
    base: &Params,
    gen: u32,
    stage: u32,
    hof: &[Brain],
    seed: u64,
    episode: i32,
    clans_per_arena: usize,
) -> Vec<QualityScore> {
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

    let results: Vec<Vec<(usize, QualityScore)>> = tasks
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

    let mut totals = vec![QualityTotals::default(); n];
    for r in results {
        for (bi, s) in r {
            totals[bi].add(s);
        }
    }
    (0..n).map(|i| totals[i].finish(&pop[i])).collect()
}

/// Score one brain across a **fixed** benchmark of randomised worlds (constant
/// `seed`, spread across stages `0..=stage`), against `opponents`. Because the
/// worlds are fixed, two brains benchmarked with the same args are directly
/// comparable — this is how a *monotonic* champion is chosen, immune to the
/// per-generation luck that froze the old champion. Parallel across worlds.
#[cfg(test)]
pub fn benchmark_quality(
    brain: &Brain,
    base: &Params,
    stage: u32,
    opponents: &[Brain],
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> QualityScore {
    if n_worlds == 0 {
        return QualityScore::default();
    }
    let scores: Vec<QualityScore> = (0..n_worlds)
        .into_par_iter()
        .map(|wi| {
            let mut wr = Rng::new(seed ^ (wi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let eff_stage = (wi as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let aseed = seed.wrapping_add((wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            run_arena_general(
                std::slice::from_ref(brain),
                opponents,
                &spec,
                episode,
                aseed,
            )[0]
        })
        .collect();
    let mut totals = QualityTotals::default();
    for score in scores {
        totals.add(score);
    }
    totals.finish(brain)
}

/// Deterministic behavioral contract across fixed worlds. Initial clan members
/// and neutrals are tracked as cohorts, so recruitment cannot disguise whether
/// group membership was actually safer than starting unaffiliated.
#[cfg(test)]
pub fn benchmark_ai_quality(
    brain: &Brain,
    base: &Params,
    stage: u32,
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> AiBenchmarkReport {
    if n_worlds == 0 {
        return AiBenchmarkReport::default();
    }
    let results: Vec<(QualityScore, f32, f32)> = (0..n_worlds)
        .into_par_iter()
        .map(|wi| {
            let mut wr = Rng::new(seed ^ (wi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let eff_stage = (wi as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let aseed = seed.wrapping_add((wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            let mut world = World::new(spec.world_size, aseed);
            world.params = spec.params;
            let ids = world.setup_arena(std::slice::from_ref(brain), spec.trees, spec.neutrals);
            let cid = ids[0];
            let clan_cohort: HashSet<u32> = world
                .entities
                .iter()
                .filter(|entity| entity.clan == cid)
                .map(|entity| entity.id)
                .collect();
            let neutral_cohort: HashSet<u32> = world
                .entities
                .iter()
                .filter(|entity| entity.clan < 0)
                .map(|entity| entity.id)
                .collect();
            for _ in 0..episode {
                world.step();
            }
            let alive: HashSet<u32> = world
                .entities
                .iter()
                .filter(|entity| entity.is_active())
                .map(|entity| entity.id)
                .collect();
            let cohort_ratio = |cohort: &HashSet<u32>| {
                if cohort.is_empty() {
                    1.0
                } else {
                    cohort.iter().filter(|id| alive.contains(id)).count() as f32
                        / cohort.len() as f32
                }
            };
            (
                score_clan_quality(&world, cid),
                cohort_ratio(&clan_cohort),
                cohort_ratio(&neutral_cohort),
            )
        })
        .collect();

    let mut totals = QualityTotals::default();
    let mut clan_survival = 0.0;
    let mut neutral_survival = 0.0;
    for (quality, clan, neutral) in results {
        totals.add(quality);
        clan_survival += clan;
        neutral_survival += neutral;
    }
    let quality = totals.finish(brain);
    let inv = 1.0 / n_worlds as f32;
    let clan_cohort_survival = clan_survival * inv;
    let neutral_cohort_survival = neutral_survival * inv;
    let fairness_delta = clan_cohort_survival - neutral_cohort_survival;
    AiBenchmarkReport {
        worlds: n_worlds,
        mean_fitness: quality.fitness,
        robust_survival: quality.robust_survival,
        mean_security: quality.security,
        clan_cohort_survival,
        neutral_cohort_survival,
        fairness_delta,
        robust_fairness_delta: quality.robust_fairness,
        mean_logistics: quality.logistics,
        mean_hauling_throughput: quality.hauling_throughput,
        mean_road_utility: quality.road_utility,
        mean_reserve_security: quality.reserve_security,
        mean_task_coverage: quality.task_coverage,
        mean_care: quality.care,
        mean_trade: quality.trade,
        routing_entropy: quality.routing_entropy,
        expert_coverage: quality.expert_coverage,
        eligible: quality.eligible && fairness_delta >= -0.05,
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct LogisticsBenchmarkObservation {
    quality: QualityScore,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    reserve_use: f32,
}

#[cfg(test)]
#[derive(Default)]
struct LogisticsArmTotals {
    quality: QualityTotals,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    reserve_use: f32,
    count: u32,
}

#[cfg(test)]
impl LogisticsArmTotals {
    fn add(&mut self, observation: LogisticsBenchmarkObservation) {
        self.quality.add(observation.quality);
        self.clan_cohort_survival += observation.clan_cohort_survival;
        self.neutral_cohort_survival += observation.neutral_cohort_survival;
        self.reserve_use += observation.reserve_use;
        self.count += 1;
    }

    fn finish(self, brain: &Brain) -> LogisticsBenchmarkArm {
        if self.count == 0 {
            return LogisticsBenchmarkArm::default();
        }
        let inv = 1.0 / self.count as f32;
        let quality = self.quality.finish(brain);
        let clan_cohort_survival = self.clan_cohort_survival * inv;
        let neutral_cohort_survival = self.neutral_cohort_survival * inv;
        let fairness_delta = clan_cohort_survival - neutral_cohort_survival;
        LogisticsBenchmarkArm {
            robust_survival: quality.robust_survival,
            mean_security: quality.security,
            clan_cohort_survival,
            neutral_cohort_survival,
            fairness_delta,
            robust_fairness_delta: quality.robust_fairness,
            hauling_throughput: quality.hauling_throughput,
            road_utility: quality.road_utility,
            reserve_use: self.reserve_use * inv,
            reserve_security: quality.reserve_security,
            eligible: quality.eligible && fairness_delta >= FAIRNESS_FLOOR,
        }
    }
}

/// Compare Community Logistics V1 against a disabled control on identical
/// generated worlds and seeds. Both arms start from the same initial cohort;
/// only `Params::community_logistics` differs. Call this explicitly for release
/// validation or diagnostics -- ordinary training never pays for both arms.
#[cfg(test)]
pub fn benchmark_logistics_quality(
    brain: &Brain,
    base: &Params,
    stage: u32,
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> LogisticsBenchmarkReport {
    if n_worlds == 0 {
        return LogisticsBenchmarkReport::default();
    }
    let paired: Vec<(LogisticsBenchmarkObservation, LogisticsBenchmarkObservation)> = (0..n_worlds)
        .into_par_iter()
        .map(|wi| {
            let mut wr = Rng::new(seed ^ (wi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let eff_stage = (wi as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let aseed = seed.wrapping_add((wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            (
                run_logistics_benchmark_arm(brain, &spec, episode, aseed, true),
                run_logistics_benchmark_arm(brain, &spec, episode, aseed, false),
            )
        })
        .collect();

    let mut enabled_totals = LogisticsArmTotals::default();
    let mut disabled_totals = LogisticsArmTotals::default();
    for (enabled, disabled) in paired {
        enabled_totals.add(enabled);
        disabled_totals.add(disabled);
    }
    let enabled = enabled_totals.finish(brain);
    let disabled = disabled_totals.finish(brain);
    let clan_survival_delta = enabled.clan_cohort_survival - disabled.clan_cohort_survival;
    LogisticsBenchmarkReport {
        worlds: n_worlds,
        clan_survival_delta,
        security_delta: enabled.mean_security - disabled.mean_security,
        hauling_throughput_delta: enabled.hauling_throughput - disabled.hauling_throughput,
        road_utility_delta: enabled.road_utility - disabled.road_utility,
        reserve_use_delta: enabled.reserve_use - disabled.reserve_use,
        reserve_security_delta: enabled.reserve_security - disabled.reserve_security,
        survival_non_regression: enabled.robust_survival + 1e-6 >= disabled.robust_survival
            && clan_survival_delta >= -1e-6,
        enabled,
        disabled,
    }
}

#[cfg(test)]
fn run_logistics_benchmark_arm(
    brain: &Brain,
    spec: &WorldSpec,
    episode: i32,
    seed: u64,
    community_logistics: bool,
) -> LogisticsBenchmarkObservation {
    let mut world = World::new(spec.world_size, seed);
    world.params = spec.params.clone();
    world.params.community_logistics = community_logistics;
    let ids = world.setup_arena(std::slice::from_ref(brain), spec.trees, spec.neutrals);
    let cid = ids[0];
    let clan_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan == cid)
        .map(|entity| entity.id)
        .collect();
    let neutral_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan < 0)
        .map(|entity| entity.id)
        .collect();
    for _ in 0..episode {
        world.step();
    }
    let alive: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.is_active())
        .map(|entity| entity.id)
        .collect();
    let cohort_ratio = |cohort: &HashSet<u32>| {
        if cohort.is_empty() {
            1.0
        } else {
            cohort.iter().filter(|id| alive.contains(id)).count() as f32 / cohort.len() as f32
        }
    };
    let clan_cohort_survival = cohort_ratio(&clan_cohort);
    let neutral_cohort_survival = cohort_ratio(&neutral_cohort);
    let mut quality = score_clan_quality(&world, cid);
    quality.fairness = clan_cohort_survival - neutral_cohort_survival;
    quality.robust_fairness = quality.fairness;
    quality.eligible &= quality.fairness >= FAIRNESS_FLOOR;
    let reserve_use = world.clan_by_id(cid).map_or(0.0, |clan| {
        let events = clan
            .stats
            .reserve_deposited
            .saturating_add(clan.stats.reserve_released) as f32;
        let rate = events * 1000.0 / clan.stats.pop_tick_sum.max(1) as f32;
        rate / (rate + 3.0)
    });
    LogisticsBenchmarkObservation {
        quality,
        clan_cohort_survival,
        neutral_cohort_survival,
        reserve_use,
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct CareBenchmarkObservation {
    quality: QualityScore,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    incapacitations: f32,
    rescues: f32,
    bleedouts: f32,
    combat_deaths: f32,
}

#[cfg(test)]
#[derive(Default)]
struct CareArmTotals {
    quality: QualityTotals,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    incapacitations: f32,
    rescues: f32,
    bleedouts: f32,
    combat_deaths: f32,
    count: u32,
}

#[cfg(test)]
impl CareArmTotals {
    fn add(&mut self, observation: CareBenchmarkObservation) {
        self.quality.add(observation.quality);
        self.clan_cohort_survival += observation.clan_cohort_survival;
        self.neutral_cohort_survival += observation.neutral_cohort_survival;
        self.incapacitations += observation.incapacitations;
        self.rescues += observation.rescues;
        self.bleedouts += observation.bleedouts;
        self.combat_deaths += observation.combat_deaths;
        self.count += 1;
    }

    fn finish(self, brain: &Brain) -> CareBenchmarkArm {
        if self.count == 0 {
            return CareBenchmarkArm::default();
        }
        let inv = 1.0 / self.count as f32;
        let quality = self.quality.finish(brain);
        let clan_cohort_survival = self.clan_cohort_survival * inv;
        let neutral_cohort_survival = self.neutral_cohort_survival * inv;
        CareBenchmarkArm {
            robust_survival: quality.robust_survival,
            mean_security: quality.security,
            clan_cohort_survival,
            neutral_cohort_survival,
            fairness_delta: clan_cohort_survival - neutral_cohort_survival,
            robust_fairness_delta: quality.robust_fairness,
            rescue_rate: quality.care,
            incapacitations: self.incapacitations * inv,
            rescues: self.rescues * inv,
            bleedouts: self.bleedouts * inv,
            combat_deaths: self.combat_deaths * inv,
            eligible: quality.eligible
                && clan_cohort_survival - neutral_cohort_survival >= FAIRNESS_FLOOR,
        }
    }
}

/// Compare Community Care V1 against immediate combat death on identical
/// generated worlds and seeds. Ordinary training never pays for both arms.
#[cfg(test)]
pub fn benchmark_care_quality(
    brain: &Brain,
    base: &Params,
    stage: u32,
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> CareBenchmarkReport {
    if n_worlds == 0 {
        return CareBenchmarkReport::default();
    }
    let paired: Vec<(CareBenchmarkObservation, CareBenchmarkObservation)> = (0..n_worlds)
        .into_par_iter()
        .map(|wi| {
            let mut wr = Rng::new(seed ^ (wi as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let eff_stage = (wi as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut wr, eff_stage);
            let arena_seed = seed.wrapping_add((wi as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            (
                run_care_benchmark_arm(brain, &spec, episode, arena_seed, true),
                run_care_benchmark_arm(brain, &spec, episode, arena_seed, false),
            )
        })
        .collect();

    let mut enabled_totals = CareArmTotals::default();
    let mut disabled_totals = CareArmTotals::default();
    for (enabled, disabled) in paired {
        enabled_totals.add(enabled);
        disabled_totals.add(disabled);
    }
    let enabled = enabled_totals.finish(brain);
    let disabled = disabled_totals.finish(brain);
    let clan_survival_delta = enabled.clan_cohort_survival - disabled.clan_cohort_survival;
    CareBenchmarkReport {
        worlds: n_worlds,
        clan_survival_delta,
        security_delta: enabled.mean_security - disabled.mean_security,
        combat_death_delta: enabled.combat_deaths - disabled.combat_deaths,
        survival_non_regression: enabled.robust_survival + 1e-6 >= disabled.robust_survival
            && clan_survival_delta >= -1e-6,
        enabled,
        disabled,
    }
}

#[cfg(test)]
fn run_care_benchmark_arm(
    brain: &Brain,
    spec: &WorldSpec,
    episode: i32,
    seed: u64,
    community_care: bool,
) -> CareBenchmarkObservation {
    let mut world = World::new(spec.world_size, seed);
    world.params = spec.params.clone();
    world.params.community_care = community_care;
    let brains = [brain.clone(), brain.clone()];
    let ids = world.setup_arena(&brains, spec.trees, spec.neutrals);
    let clan_id = ids[0];
    let clan_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan == clan_id)
        .map(|entity| entity.id)
        .collect();
    let neutral_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan < 0)
        .map(|entity| entity.id)
        .collect();
    for _ in 0..episode {
        world.step();
    }
    let alive: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.is_active())
        .map(|entity| entity.id)
        .collect();
    let cohort_ratio = |cohort: &HashSet<u32>| {
        if cohort.is_empty() {
            1.0
        } else {
            cohort.iter().filter(|id| alive.contains(id)).count() as f32 / cohort.len() as f32
        }
    };
    let clan_cohort_survival = cohort_ratio(&clan_cohort);
    let neutral_cohort_survival = cohort_ratio(&neutral_cohort);
    let mut quality = score_clan_quality(&world, clan_id);
    quality.fairness = clan_cohort_survival - neutral_cohort_survival;
    quality.robust_fairness = quality.fairness;
    quality.eligible &= quality.fairness >= FAIRNESS_FLOOR;
    let (incapacitations, rescues, bleedouts) =
        world.clan_by_id(clan_id).map_or((0.0, 0.0, 0.0), |clan| {
            (
                clan.stats.incapacitations as f32,
                clan.stats.rescues as f32,
                clan.stats.bleedouts as f32,
            )
        });
    CareBenchmarkObservation {
        quality,
        clan_cohort_survival,
        neutral_cohort_survival,
        incapacitations,
        rescues,
        bleedouts,
        combat_deaths: world.deaths_combat as f32,
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct TradeBenchmarkObservation {
    quality: QualityScore,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    delivered_food: f32,
    delivered_wood: f32,
    deliveries: f32,
}

#[cfg(test)]
#[derive(Default)]
struct TradeArmTotals {
    quality: QualityTotals,
    clan_cohort_survival: f32,
    neutral_cohort_survival: f32,
    delivered_food: f32,
    delivered_wood: f32,
    deliveries: f32,
    count: u32,
}

#[cfg(test)]
impl TradeArmTotals {
    fn add(&mut self, observation: TradeBenchmarkObservation) {
        self.quality.add(observation.quality);
        self.clan_cohort_survival += observation.clan_cohort_survival;
        self.neutral_cohort_survival += observation.neutral_cohort_survival;
        self.delivered_food += observation.delivered_food;
        self.delivered_wood += observation.delivered_wood;
        self.deliveries += observation.deliveries;
        self.count += 1;
    }

    fn finish(self, brain: &Brain) -> TradeBenchmarkArm {
        if self.count == 0 {
            return TradeBenchmarkArm::default();
        }
        let inv = 1.0 / self.count as f32;
        let quality = self.quality.finish(brain);
        let clan_cohort_survival = self.clan_cohort_survival * inv;
        let neutral_cohort_survival = self.neutral_cohort_survival * inv;
        TradeBenchmarkArm {
            robust_survival: quality.robust_survival,
            mean_security: quality.security,
            clan_cohort_survival,
            neutral_cohort_survival,
            fairness_delta: clan_cohort_survival - neutral_cohort_survival,
            robust_fairness_delta: quality.robust_fairness,
            trade_value: quality.trade,
            delivered_food: self.delivered_food * inv,
            delivered_wood: self.delivered_wood * inv,
            deliveries: self.deliveries * inv,
            eligible: quality.eligible
                && clan_cohort_survival - neutral_cohort_survival >= FAIRNESS_FLOOR,
        }
    }
}

/// Compare physical Trade/Diplomacy V1 against a no-trade control on identical
/// worlds. Existing food/wood, policy dimensions, and hostile behavior remain.
#[cfg(test)]
pub fn benchmark_trade_quality(
    brain: &Brain,
    base: &Params,
    stage: u32,
    episode: i32,
    n_worlds: usize,
    seed: u64,
) -> TradeBenchmarkReport {
    if n_worlds == 0 {
        return TradeBenchmarkReport::default();
    }
    let paired: Vec<(TradeBenchmarkObservation, TradeBenchmarkObservation)> = (0..n_worlds)
        .into_par_iter()
        .map(|world_index| {
            let mut rng = Rng::new(seed ^ (world_index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let effective_stage = (world_index as u32) % (stage + 1);
            let spec = random_world_spec(base, &mut rng, effective_stage);
            let arena_seed =
                seed.wrapping_add((world_index as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
            (
                run_trade_benchmark_arm(brain, &spec, episode, arena_seed, true),
                run_trade_benchmark_arm(brain, &spec, episode, arena_seed, false),
            )
        })
        .collect();
    let mut enabled_totals = TradeArmTotals::default();
    let mut disabled_totals = TradeArmTotals::default();
    for (enabled, disabled) in paired {
        enabled_totals.add(enabled);
        disabled_totals.add(disabled);
    }
    let enabled = enabled_totals.finish(brain);
    let disabled = disabled_totals.finish(brain);
    let clan_survival_delta = enabled.clan_cohort_survival - disabled.clan_cohort_survival;
    TradeBenchmarkReport {
        worlds: n_worlds,
        clan_survival_delta,
        security_delta: enabled.mean_security - disabled.mean_security,
        delivered_material_delta: enabled.delivered_food + enabled.delivered_wood
            - disabled.delivered_food
            - disabled.delivered_wood,
        survival_non_regression: enabled.robust_survival + 1e-6 >= disabled.robust_survival
            && clan_survival_delta >= -1e-6,
        enabled,
        disabled,
    }
}

#[cfg(test)]
fn run_trade_benchmark_arm(
    brain: &Brain,
    spec: &WorldSpec,
    episode: i32,
    seed: u64,
    community_trade: bool,
) -> TradeBenchmarkObservation {
    let mut world = World::new(spec.world_size, seed);
    world.params = spec.params.clone();
    world.params.community_trade = community_trade;
    let brains = [brain.clone(), brain.clone(), brain.clone()];
    let ids = world.setup_arena(&brains, spec.trees, spec.neutrals);
    let clan_id = ids[0];
    let clan_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan == clan_id)
        .map(|entity| entity.id)
        .collect();
    let neutral_cohort: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.clan < 0)
        .map(|entity| entity.id)
        .collect();
    for _ in 0..episode {
        world.step();
    }
    let alive: HashSet<u32> = world
        .entities
        .iter()
        .filter(|entity| entity.is_active())
        .map(|entity| entity.id)
        .collect();
    let cohort_ratio = |cohort: &HashSet<u32>| {
        if cohort.is_empty() {
            1.0
        } else {
            cohort.iter().filter(|id| alive.contains(id)).count() as f32 / cohort.len() as f32
        }
    };
    let clan_cohort_survival = cohort_ratio(&clan_cohort);
    let neutral_cohort_survival = cohort_ratio(&neutral_cohort);
    let mut quality = score_clan_quality(&world, clan_id);
    quality.fairness = clan_cohort_survival - neutral_cohort_survival;
    quality.robust_fairness = quality.fairness;
    quality.eligible &= quality.fairness >= FAIRNESS_FLOOR;
    let (delivered_food, delivered_wood, deliveries) =
        world.clan_by_id(clan_id).map_or((0.0, 0.0, 0.0), |clan| {
            (
                clan.stats.trade_food_sent as f32,
                clan.stats.trade_wood_sent as f32,
                clan.stats.trade_deliveries as f32,
            )
        });
    TradeBenchmarkObservation {
        quality,
        clan_cohort_survival,
        neutral_cohort_survival,
        delivered_food,
        delivered_wood,
        deliveries,
    }
}

#[cfg(test)]
fn quality_better(challenger: &QualityScore, reigning: &QualityScore) -> bool {
    match (challenger.eligible, reigning.eligible) {
        (true, false) => true,
        (false, true) => false,
        _ => challenger.selection_score() >= reigning.selection_score(),
    }
}

#[cfg(test)]
fn champion_promotion_rejections(
    challenger: &QualityScore,
    reigning: Option<&QualityScore>,
    logistics: &LogisticsBenchmarkReport,
    reigning_logistics: Option<&LogisticsBenchmarkReport>,
    trade: &TradeBenchmarkReport,
    reigning_trade: Option<&TradeBenchmarkReport>,
) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if !challenger.eligible {
        reasons.push("headline benchmark ineligible");
    }
    if challenger.robust_survival < PROMOTION_SURVIVAL_FLOOR {
        reasons.push("robust survival below promotion floor");
    }
    if challenger.security < PROMOTION_SECURITY_FLOOR {
        reasons.push("food security below promotion floor");
    }
    if challenger.fairness < PROMOTION_FAIRNESS_FLOOR {
        reasons.push("clans underperform neutrals");
    }
    if challenger.robust_fairness < FAIRNESS_FLOOR {
        reasons.push("worst-world fairness below floor");
    }
    if challenger.routing_entropy < PROMOTION_ROUTING_FLOOR {
        reasons.push("routing entropy below floor");
    }
    if challenger.expert_coverage < PROMOTION_EXPERT_COVERAGE_FLOOR {
        reasons.push("expert coverage below floor");
    }

    if let Some(current) = reigning {
        if !quality_better(challenger, current) {
            reasons.push("headline quality did not improve");
        }
        if challenger.robust_survival + f32::EPSILON < current.robust_survival {
            reasons.push("robust survival regressed from champion");
        }
        if challenger.security + PROMOTION_SECURITY_TOLERANCE < current.security {
            reasons.push("food security regressed from champion");
        }
        if challenger.fairness + PROMOTION_SECURITY_TOLERANCE < current.fairness {
            reasons.push("fairness regressed from champion");
        }
        if challenger.routing_entropy + PROMOTION_ROUTING_TOLERANCE < current.routing_entropy {
            reasons.push("routing entropy regressed from champion");
        }
        if challenger.expert_coverage + PROMOTION_COVERAGE_TOLERANCE < current.expert_coverage {
            reasons.push("expert coverage regressed from champion");
        }
        if challenger.care + PROMOTION_COVERAGE_TOLERANCE < current.care {
            reasons.push("community care regressed from champion");
        }
        if challenger.trade + PROMOTION_COVERAGE_TOLERANCE < current.trade {
            reasons.push("delivered trade regressed from champion");
        }
    }

    if !logistics.enabled.eligible {
        reasons.push("logistics-enabled benchmark ineligible");
    }
    if logistics.enabled.robust_survival < PROMOTION_SURVIVAL_FLOOR {
        reasons.push("logistics-enabled survival below promotion floor");
    }
    if logistics.enabled.mean_security < PROMOTION_SECURITY_FLOOR {
        reasons.push("logistics-enabled security below promotion floor");
    }
    if logistics.enabled.fairness_delta < PROMOTION_FAIRNESS_FLOOR {
        reasons.push("logistics-enabled clans underperform neutrals");
    }
    if logistics.enabled.robust_fairness_delta < FAIRNESS_FLOOR {
        reasons.push("logistics-enabled worst-world fairness below floor");
    }
    if !logistics.survival_non_regression {
        reasons.push("logistics reduces paired survival");
    }
    if logistics.security_delta < -PROMOTION_SECURITY_TOLERANCE {
        reasons.push("logistics reduces paired food security");
    }
    let transport_gain =
        logistics.hauling_throughput_delta * 0.60 + logistics.road_utility_delta * 0.40;
    if transport_gain < 0.0 {
        reasons.push("logistics does not improve paired transport");
    }
    if logistics.reserve_use_delta < 0.0 {
        reasons.push("logistics does not activate the paired reserve");
    }

    if let Some(current) = reigning_logistics {
        if logistics.enabled.robust_survival + f32::EPSILON < current.enabled.robust_survival {
            reasons.push("paired logistics survival regressed from champion");
        }
        if logistics.enabled.mean_security + PROMOTION_SECURITY_TOLERANCE
            < current.enabled.mean_security
        {
            reasons.push("paired logistics security regressed from champion");
        }
        if logistics.enabled.fairness_delta + PROMOTION_SECURITY_TOLERANCE
            < current.enabled.fairness_delta
        {
            reasons.push("paired logistics fairness regressed from champion");
        }
        let candidate_value = promotion_logistics_value(&logistics.enabled);
        let current_value = promotion_logistics_value(&current.enabled);
        if candidate_value + PROMOTION_LOGISTICS_TOLERANCE < current_value {
            reasons.push("causal logistics value regressed from champion");
        }
    }

    if !trade.enabled.eligible {
        reasons.push("trade-enabled benchmark ineligible");
    }
    if trade.enabled.robust_survival < PROMOTION_SURVIVAL_FLOOR {
        reasons.push("trade-enabled survival below promotion floor");
    }
    if trade.enabled.mean_security < PROMOTION_SECURITY_FLOOR {
        reasons.push("trade-enabled security below promotion floor");
    }
    if trade.enabled.fairness_delta < PROMOTION_FAIRNESS_FLOOR {
        reasons.push("trade-enabled clans underperform neutrals");
    }
    if trade.enabled.robust_fairness_delta < FAIRNESS_FLOOR {
        reasons.push("trade-enabled worst-world fairness below floor");
    }
    if !trade.survival_non_regression {
        reasons.push("trade reduces paired survival");
    }
    if trade.security_delta < -PROMOTION_SECURITY_TOLERANCE {
        reasons.push("trade reduces paired food security");
    }
    if trade.delivered_material_delta <= 0.0 || trade.enabled.deliveries <= 0.0 {
        reasons.push("trade does not deliver physical material");
    }

    if let Some(current) = reigning_trade {
        if trade.enabled.robust_survival + f32::EPSILON < current.enabled.robust_survival {
            reasons.push("paired trade survival regressed from champion");
        }
        if trade.enabled.mean_security + PROMOTION_SECURITY_TOLERANCE
            < current.enabled.mean_security
        {
            reasons.push("paired trade security regressed from champion");
        }
        if trade.enabled.fairness_delta + PROMOTION_SECURITY_TOLERANCE
            < current.enabled.fairness_delta
        {
            reasons.push("paired trade fairness regressed from champion");
        }
        if trade.enabled.trade_value + PROMOTION_TRADE_TOLERANCE < current.enabled.trade_value {
            reasons.push("causal trade value regressed from champion");
        }
    }
    reasons
}

#[cfg(test)]
fn promotion_logistics_value(arm: &LogisticsBenchmarkArm) -> f32 {
    arm.hauling_throughput * 0.50 + arm.road_utility * 0.30 + arm.reserve_security * 0.20
}

/// Fitness from a clan's final state. Kept as a smooth weighted sum (no hard
/// cliffs) so the gradient is learnable; a wiped-out clan scores 0.
#[cfg(test)]
fn score_clan(w: &World, cid: i32) -> f32 {
    score_clan_quality(w, cid).fitness
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

    #[test]
    fn survival_gate_beats_flashy_extinction() {
        let extinct = QualityScore {
            fitness: 1_000_000.0,
            survival: 0.0,
            robust_survival: 0.0,
            security: 0.0,
            ..QualityScore::default()
        };
        let viable = QualityScore {
            fitness: 100.0,
            survival: 1.0,
            robust_survival: 1.0,
            security: 0.8,
            eligible: true,
            ..QualityScore::default()
        };
        assert!(viable.selection_score() > extinct.selection_score());
    }

    #[test]
    fn quality_diversity_archive_keeps_distinct_niches() {
        let mut cfg = TrainCfg::default();
        cfg.pop_size = N_STRATEGY_NICHES;
        cfg.elite = 0;
        let mut trainer = Trainer::new(cfg);
        let population = trainer.population.clone();
        let base = QualityScore {
            fitness: 100.0,
            survival: 1.0,
            robust_survival: 1.0,
            security: 0.8,
            settlement: 0.6,
            expansion: 0.2,
            defense: 0.7,
            eligible: true,
            ..QualityScore::default()
        };
        let mut scores = vec![base; N_STRATEGY_NICHES];
        scores[0].security = 1.0;
        scores[1].settlement = 1.0;
        scores[1].expansion = 1.0;
        scores[2].logistics = 1.0;
        scores[2].reserve_security = 1.0;
        scores[2].task_coverage = 1.0;
        scores[3].defense = 1.0;
        scores[4].combat = 1.0;
        trainer.finish_general(population, scores, 0.0);
        assert_eq!(trainer.qd_archive_len(), N_STRATEGY_NICHES);
        assert_eq!(trainer.population.len(), N_STRATEGY_NICHES);
    }

    #[test]
    fn ai_quality_benchmark_is_deterministic() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let base = Params::default();
        let a = benchmark_ai_quality(&brain, &base, MAX_STAGE, 4000, 13, 0x51FE_BEEF);
        let b = benchmark_ai_quality(&brain, &base, MAX_STAGE, 4000, 13, 0x51FE_BEEF);
        println!("AI quality benchmark: {a:#?}");
        assert_eq!(a.worlds, 13);
        assert!((a.mean_fitness - b.mean_fitness).abs() < 1e-5);
        assert!((a.fairness_delta - b.fairness_delta).abs() < 1e-6);
        assert!((a.robust_fairness_delta - b.robust_fairness_delta).abs() < 1e-6);
        assert!((a.mean_logistics - b.mean_logistics).abs() < 1e-6);
        assert!((a.mean_hauling_throughput - b.mean_hauling_throughput).abs() < 1e-6);
        assert!((a.mean_road_utility - b.mean_road_utility).abs() < 1e-6);
        assert!((a.mean_reserve_security - b.mean_reserve_security).abs() < 1e-6);
        assert!((a.mean_task_coverage - b.mean_task_coverage).abs() < 1e-6);
        assert!((a.mean_care - b.mean_care).abs() < 1e-6);
        assert!((a.mean_trade - b.mean_trade).abs() < 1e-6);
        assert!((0.0..=1.0).contains(&a.routing_entropy));
        assert!((0.0..=1.0).contains(&a.expert_coverage));
        assert!(
            a.eligible,
            "tracked champion must remain survival-qualified"
        );
        assert!(
            a.robust_survival >= 0.95,
            "robust survival regressed: {a:#?}"
        );
        assert!(a.mean_security >= 0.80, "food security regressed: {a:#?}");
        assert!(
            a.clan_cohort_survival >= a.neutral_cohort_survival,
            "clan membership became worse than neutrality: {a:#?}"
        );
        assert!(
            a.expert_coverage >= 0.50,
            "expert collapse regressed: {a:#?}"
        );
        assert!(
            a.routing_entropy >= 0.10,
            "routing collapse regressed: {a:#?}"
        );
    }

    #[test]
    fn logistics_ablation_is_deterministic() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let base = Params::default();
        let a = benchmark_logistics_quality(&brain, &base, 3, 1500, 4, 0x10C1_571C5);
        let b = benchmark_logistics_quality(&brain, &base, 3, 1500, 4, 0x10C1_571C5);
        assert_eq!(a.worlds, b.worlds);
        assert!((a.clan_survival_delta - b.clan_survival_delta).abs() < 1e-6);
        assert!((a.security_delta - b.security_delta).abs() < 1e-6);
        assert!((a.hauling_throughput_delta - b.hauling_throughput_delta).abs() < 1e-6);
        assert!((a.road_utility_delta - b.road_utility_delta).abs() < 1e-6);
        assert!((a.reserve_use_delta - b.reserve_use_delta).abs() < 1e-6);
        assert!((a.reserve_security_delta - b.reserve_security_delta).abs() < 1e-6);
        assert_eq!(a.survival_non_regression, b.survival_non_regression);
        assert_eq!(a.disabled.road_utility, 0.0);
    }

    #[test]
    fn tracked_champion_logistics_preserves_survival_gates() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let report = benchmark_logistics_quality(
            &brain,
            &Params::default(),
            MAX_STAGE,
            4000,
            13,
            0x51FE_BEEF,
        );
        println!("Community Logistics V1.1 paired benchmark: {report:#?}");
        assert!(
            report.enabled.eligible,
            "enabled logistics must preserve survival/security/fairness gates: {report:#?}"
        );
        assert!(
            report.enabled.robust_survival >= 0.95,
            "enabled logistics robust survival regressed: {report:#?}"
        );
        assert!(
            report.enabled.mean_security >= 0.80,
            "enabled logistics food security regressed: {report:#?}"
        );
        assert!(
            report.enabled.fairness_delta >= FAIRNESS_FLOOR,
            "enabled logistics fairness regressed: {report:#?}"
        );
        assert!(
            report.enabled.robust_fairness_delta >= FAIRNESS_FLOOR,
            "enabled logistics worst-world fairness regressed: {report:#?}"
        );
        assert!(
            (report.enabled.fairness_delta
                - (report.enabled.clan_cohort_survival - report.enabled.neutral_cohort_survival))
                .abs()
                < 1e-6,
            "cohort fairness accounting diverged: {report:#?}"
        );
        assert!(
            report.survival_non_regression,
            "logistics-enabled cohort survival fell below the paired control: {report:#?}"
        );
        assert!(
            report.security_delta >= -0.01,
            "logistics-enabled food security fell materially below the paired control: {report:#?}"
        );
    }

    #[test]
    fn care_ablation_is_deterministic() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let base = Params::default();
        let a = benchmark_care_quality(&brain, &base, 3, 1500, 4, 0xCA2E_5EED);
        let b = benchmark_care_quality(&brain, &base, 3, 1500, 4, 0xCA2E_5EED);
        assert_eq!(a.worlds, b.worlds);
        assert!((a.clan_survival_delta - b.clan_survival_delta).abs() < 1e-6);
        assert!((a.security_delta - b.security_delta).abs() < 1e-6);
        assert!((a.combat_death_delta - b.combat_death_delta).abs() < 1e-6);
        assert!((a.enabled.rescues - b.enabled.rescues).abs() < 1e-6);
        assert!((a.enabled.rescue_rate - b.enabled.rescue_rate).abs() < 1e-6);
        assert!((a.enabled.bleedouts - b.enabled.bleedouts).abs() < 1e-6);
        assert!(
            (a.enabled.neutral_cohort_survival - b.enabled.neutral_cohort_survival).abs() < 1e-6
        );
        assert!((a.enabled.robust_fairness_delta - b.enabled.robust_fairness_delta).abs() < 1e-6);
        assert_eq!(a.disabled.rescues, 0.0);
    }

    #[test]
    fn tracked_champion_care_preserves_survival_gates() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let report =
            benchmark_care_quality(&brain, &Params::default(), MAX_STAGE, 4000, 13, 0xCA2E_BEEF);
        println!("Community Care V1 paired benchmark: {report:#?}");
        assert!(
            report.enabled.eligible,
            "care enabled became ineligible: {report:#?}"
        );
        assert!(
            report.enabled.robust_survival >= PROMOTION_SURVIVAL_FLOOR,
            "care robust survival regressed: {report:#?}"
        );
        assert!(
            report.enabled.mean_security >= PROMOTION_SECURITY_FLOOR,
            "care food security regressed: {report:#?}"
        );
        assert!(
            report.enabled.fairness_delta >= FAIRNESS_FLOOR,
            "care clan fairness regressed: {report:#?}"
        );
        assert!(
            report.survival_non_regression,
            "care reduced paired cohort survival: {report:#?}"
        );
        assert!(
            report.security_delta >= -PROMOTION_SECURITY_TOLERANCE,
            "care reduced paired food security: {report:#?}"
        );
        if report.enabled.incapacitations > 0.0 {
            assert!(
                report.enabled.rescues > 0.0,
                "care had opportunities but completed no rescues: {report:#?}"
            );
        }
    }

    #[test]
    fn trade_ablation_is_deterministic() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let base = Params::default();
        let a = benchmark_trade_quality(&brain, &base, 3, 1500, 4, 0x7ADE_5EED);
        let b = benchmark_trade_quality(&brain, &base, 3, 1500, 4, 0x7ADE_5EED);
        assert_eq!(a.worlds, b.worlds);
        assert!((a.clan_survival_delta - b.clan_survival_delta).abs() < 1e-6);
        assert!((a.security_delta - b.security_delta).abs() < 1e-6);
        assert!((a.delivered_material_delta - b.delivered_material_delta).abs() < 1e-6);
        assert!((a.enabled.trade_value - b.enabled.trade_value).abs() < 1e-6);
        assert!((a.enabled.deliveries - b.enabled.deliveries).abs() < 1e-6);
        assert!(
            (a.enabled.neutral_cohort_survival - b.enabled.neutral_cohort_survival).abs() < 1e-6
        );
        assert!((a.enabled.robust_fairness_delta - b.enabled.robust_fairness_delta).abs() < 1e-6);
        assert_eq!(a.disabled.deliveries, 0.0);
    }

    #[test]
    fn tracked_champion_trade_preserves_survival_gates() {
        let brain = Brain::load(CHAMPION_PATH).expect("tracked champion.bin should load");
        let report =
            benchmark_trade_quality(&brain, &Params::default(), MAX_STAGE, 4000, 13, 0x7ADE_BEEF);
        println!("Trade/Diplomacy V1 paired benchmark: {report:#?}");
        assert!(
            report.enabled.eligible,
            "trade enabled became ineligible: {report:#?}"
        );
        assert!(
            report.enabled.robust_survival >= PROMOTION_SURVIVAL_FLOOR,
            "trade robust survival regressed: {report:#?}"
        );
        assert!(
            report.enabled.mean_security >= PROMOTION_SECURITY_FLOOR,
            "trade food security regressed: {report:#?}"
        );
        assert!(
            report.enabled.fairness_delta >= FAIRNESS_FLOOR,
            "trade clan fairness regressed: {report:#?}"
        );
        assert!(
            report.survival_non_regression,
            "trade reduced paired cohort survival: {report:#?}"
        );
        assert!(
            report.security_delta >= -PROMOTION_SECURITY_TOLERANCE,
            "trade reduced paired food security: {report:#?}"
        );
        assert!(
            report.delivered_material_delta > 0.0 && report.enabled.deliveries > 0.0,
            "trade treatment did not produce physical deliveries: {report:#?}"
        );
    }

    fn promotion_quality(fitness: f32) -> QualityScore {
        QualityScore {
            fitness,
            survival: 1.0,
            robust_survival: 1.0,
            security: 0.90,
            fairness: 0.05,
            robust_fairness: 0.0,
            logistics: 0.50,
            hauling_throughput: 0.50,
            road_utility: 0.30,
            reserve_security: 0.60,
            task_coverage: 0.80,
            routing_entropy: 0.50,
            expert_coverage: 0.80,
            eligible: true,
            ..QualityScore::default()
        }
    }

    fn promotion_logistics(hauling: f32, roads: f32) -> LogisticsBenchmarkReport {
        let enabled = LogisticsBenchmarkArm {
            robust_survival: 1.0,
            mean_security: 0.90,
            clan_cohort_survival: 1.0,
            neutral_cohort_survival: 0.95,
            fairness_delta: 0.05,
            robust_fairness_delta: 0.0,
            hauling_throughput: hauling,
            road_utility: roads,
            reserve_use: 0.25,
            reserve_security: 0.60,
            eligible: true,
        };
        let disabled = LogisticsBenchmarkArm {
            robust_survival: 1.0,
            mean_security: 0.90,
            clan_cohort_survival: 1.0,
            neutral_cohort_survival: 0.95,
            fairness_delta: 0.05,
            robust_fairness_delta: 0.0,
            hauling_throughput: 0.35,
            road_utility: 0.0,
            reserve_use: 0.0,
            reserve_security: 0.0,
            eligible: true,
        };
        LogisticsBenchmarkReport {
            worlds: 13,
            hauling_throughput_delta: enabled.hauling_throughput - disabled.hauling_throughput,
            road_utility_delta: enabled.road_utility - disabled.road_utility,
            reserve_use_delta: enabled.reserve_use - disabled.reserve_use,
            reserve_security_delta: enabled.reserve_security - disabled.reserve_security,
            survival_non_regression: true,
            enabled,
            disabled,
            ..LogisticsBenchmarkReport::default()
        }
    }

    fn promotion_trade(trade_value: f32, deliveries: f32) -> TradeBenchmarkReport {
        let enabled = TradeBenchmarkArm {
            robust_survival: 1.0,
            mean_security: 0.90,
            clan_cohort_survival: 1.0,
            neutral_cohort_survival: 0.95,
            fairness_delta: 0.05,
            robust_fairness_delta: 0.0,
            trade_value,
            delivered_food: deliveries * 0.6,
            delivered_wood: deliveries * 0.4,
            deliveries,
            eligible: true,
        };
        TradeBenchmarkReport {
            worlds: 13,
            enabled,
            disabled: TradeBenchmarkArm {
                robust_survival: 1.0,
                mean_security: 0.90,
                clan_cohort_survival: 1.0,
                neutral_cohort_survival: 0.95,
                fairness_delta: 0.05,
                robust_fairness_delta: 0.0,
                eligible: true,
                ..TradeBenchmarkArm::default()
            },
            clan_survival_delta: 0.0,
            security_delta: 0.0,
            delivered_material_delta: deliveries,
            survival_non_regression: true,
        }
    }

    #[test]
    fn champion_promotion_accepts_a_safe_causal_improvement() {
        let incumbent = promotion_quality(100.0);
        let challenger = promotion_quality(110.0);
        let incumbent_logistics = promotion_logistics(0.48, 0.28);
        let challenger_logistics = promotion_logistics(0.52, 0.32);
        let incumbent_trade = promotion_trade(0.40, 10.0);
        let challenger_trade = promotion_trade(0.45, 12.0);
        let rejections = champion_promotion_rejections(
            &challenger,
            Some(&incumbent),
            &challenger_logistics,
            Some(&incumbent_logistics),
            &challenger_trade,
            Some(&incumbent_trade),
        );
        assert!(
            rejections.is_empty(),
            "unexpected rejections: {rejections:?}"
        );
    }

    #[test]
    fn champion_promotion_rejects_flashy_survival_regression() {
        let incumbent = promotion_quality(100.0);
        let mut challenger = promotion_quality(200.0);
        challenger.robust_survival = 0.90;
        let logistics = promotion_logistics(0.52, 0.32);
        let trade = promotion_trade(0.45, 12.0);
        let rejections = champion_promotion_rejections(
            &challenger,
            Some(&incumbent),
            &logistics,
            Some(&logistics),
            &trade,
            Some(&trade),
        );
        assert!(rejections.contains(&"robust survival below promotion floor"));
        assert!(rejections.contains(&"robust survival regressed from champion"));
    }

    #[test]
    fn champion_promotion_rejects_harmful_or_hollow_logistics() {
        let incumbent = promotion_quality(100.0);
        let challenger = promotion_quality(110.0);
        let incumbent_logistics = promotion_logistics(0.50, 0.30);
        let mut challenger_logistics = promotion_logistics(0.20, 0.0);
        challenger_logistics.security_delta = -0.02;
        challenger_logistics.survival_non_regression = false;
        let trade = promotion_trade(0.45, 12.0);
        let rejections = champion_promotion_rejections(
            &challenger,
            Some(&incumbent),
            &challenger_logistics,
            Some(&incumbent_logistics),
            &trade,
            Some(&trade),
        );
        assert!(rejections.contains(&"logistics reduces paired survival"));
        assert!(rejections.contains(&"logistics reduces paired food security"));
        assert!(rejections.contains(&"logistics does not improve paired transport"));
        assert!(rejections.contains(&"causal logistics value regressed from champion"));
    }

    #[test]
    fn champion_promotion_rejects_inert_or_regressed_trade() {
        let incumbent = promotion_quality(100.0);
        let challenger = promotion_quality(110.0);
        let logistics = promotion_logistics(0.52, 0.32);
        let incumbent_trade = promotion_trade(0.50, 12.0);
        let mut challenger_trade = promotion_trade(0.0, 0.0);
        challenger_trade.security_delta = -0.02;
        let rejections = champion_promotion_rejections(
            &challenger,
            Some(&incumbent),
            &logistics,
            Some(&logistics),
            &challenger_trade,
            Some(&incumbent_trade),
        );
        assert!(rejections.contains(&"trade reduces paired food security"));
        assert!(rejections.contains(&"trade does not deliver physical material"));
        assert!(rejections.contains(&"causal trade value regressed from champion"));
    }

    #[test]
    fn quality_training_runs_several_generations() {
        let mut cfg = TrainCfg::default();
        cfg.pop_size = 10;
        cfg.episode_ticks = 1200;
        cfg.clans_per_arena = 5;
        let mut trainer = Trainer::new(cfg);
        for _ in 0..3 {
            let population = trainer.population.clone();
            let generation = trainer.generation;
            let scores = evaluate_general_quality(
                &population,
                &trainer.cfg.arena_params,
                generation,
                trainer.stage,
                &[],
                trainer.cfg.seed,
                trainer.cfg.episode_ticks,
                trainer.cfg.clans_per_arena,
            );
            trainer.finish_general(population, scores, 0.0);
        }
        assert_eq!(trainer.generation, 3);
        assert_eq!(trainer.population.len(), 10);
        assert!(trainer.best_fitness.is_finite());
        assert!(
            trainer.qd_archive_len() > 0,
            "quality training should preserve at least one survival-qualified niche"
        );
    }
}
