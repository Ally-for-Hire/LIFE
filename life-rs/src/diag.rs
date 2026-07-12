//! Headless diagnostics — run the sim without the GUI and print a rich,
//! text-based report so behaviour can be observed and iterated on.
//!
//! Run a scenario with, e.g.:
//!   cargo test --release diag_gentle -- --nocapture
//!
//! The metrics here target the central design question: are clans forming
//! settled villages that *use* their territory, or are they scattering across
//! the map to forage? Watch `home_dist` (mean member distance from the
//! stockpile) and `on_terr%` (share of members standing on owned land).

#![cfg(test)]

use crate::clan::ClanMode;
use crate::world::{Params, World};

/// Build a world matching the app's "Gentle / survival-first" default preset.
fn gentle_world(seed: u64) -> World {
    let mut p = Params::default();
    p.tree_interval = 125;
    p.tree_per_cycle = 4;
    p.tree_radius = 8;
    p.max_pellet_fraction = 0.055;
    p.vision_radius = 17;
    p.starve_ticks = 1400;
    p.clan_grace_ticks = 2400;
    p.war_threshold = 1.55;
    let mut w = World::new(220, seed);
    w.params = p;
    w.maintain_pop = 0; // population is limited by food/territory, not floored
    w.maintain_clans = 5; // keep ~5 villages alive (refugees re-form them)
    w.populate(70, 55, 5);
    w
}

struct ClanSnap {
    id: i32,
    pop: usize,
    territory: u32,
    food: i32,
    wood: i32,
    reserve_food: i32,
    roads_built: u32,
    mode: ClanMode,
    kills: u32,
    losses: u32,
    recruits: u32,
    home_dist: f32, // mean member distance from stockpile
    max_dist: f32,
    on_terr: f32, // fraction of members standing on owned land
    logistics: f32,
    hauling_throughput: f32,
    road_utility: f32,
    reserve_security: f32,
    task_coverage: f32,
}

fn snapshot_clans(w: &World) -> Vec<ClanSnap> {
    let mut out = Vec::new();
    for c in w.clans.iter().filter(|c| !c.disbanded) {
        let (sx, sy) = match c.stockpile {
            Some(p) => p,
            None => continue,
        };
        let mut n = 0usize;
        let mut dsum = 0f32;
        let mut dmax = 0f32;
        let mut on = 0usize;
        for e in w.entities.iter().filter(|e| e.clan == c.id) {
            let d = (((e.x - sx).pow(2) + (e.y - sy).pow(2)) as f32).sqrt();
            dsum += d;
            dmax = dmax.max(d);
            if w.grid.owner[w.grid.idx(e.x, e.y)] == c.id {
                on += 1;
            }
            n += 1;
        }
        if n == 0 {
            continue;
        }
        let quality = crate::quality::score_clan(w, c.id);
        out.push(ClanSnap {
            id: c.id,
            pop: n,
            territory: c.territory,
            food: c.food,
            wood: c.wood,
            reserve_food: c.reserve_food,
            roads_built: c.stats.roads_built,
            mode: c.mode,
            kills: c.stats.kills,
            losses: c.stats.losses,
            recruits: c.stats.recruits,
            home_dist: dsum / n as f32,
            max_dist: dmax,
            on_terr: on as f32 / n as f32,
            logistics: quality.logistics,
            hauling_throughput: quality.hauling_throughput,
            road_utility: quality.road_utility,
            reserve_security: quality.reserve_security,
            task_coverage: quality.task_coverage,
        });
    }
    out
}

fn goal_histogram(w: &World) -> std::collections::BTreeMap<&'static str, usize> {
    let mut m: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for e in &w.entities {
        *m.entry(e.goal.label()).or_insert(0) += 1;
    }
    m
}

fn mode_histogram(w: &World) -> std::collections::BTreeMap<&'static str, usize> {
    let mut m: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for c in w.clans.iter().filter(|c| !c.disbanded) {
        *m.entry(c.mode.label()).or_insert(0) += 1;
    }
    m
}

fn report(label: &str, mut w: World, ticks: i32, every: i32) {
    println!("\n================ {label} ================");
    println!(
        "{:>6} | {:>4} {:>4} {:>4} | {:>6} {:>6} {:>6} | {:>8} {:>7} {:>7} | {:>4} {:>5} {:>5} {:>4} {:>5} | modes",
        "tick", "pop", "cln", "ldr", "food", "strv", "kill", "homeDist", "onTerr%", "terr", "log%", "haul%", "road%", "res%", "task%"
    );
    let mut t = 0;
    while t < ticks {
        for _ in 0..every {
            w.step();
            t += 1;
            if t >= ticks {
                break;
            }
        }
        let clans = snapshot_clans(&w);
        let total_pop = w.population();
        let total_terr: u32 = clans.iter().map(|c| c.territory).sum();
        let total_clan_food: i32 = clans.iter().map(|c| c.food).sum();
        // pop-weighted means
        let (mut dsum, mut osum, mut lsum, mut hsum, mut usum, mut rsum, mut tsum, mut wn) =
            (0f32, 0f32, 0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
        for c in &clans {
            dsum += c.home_dist * c.pop as f32;
            osum += c.on_terr * c.pop as f32;
            lsum += c.logistics * c.pop as f32;
            hsum += c.hauling_throughput * c.pop as f32;
            usum += c.road_utility * c.pop as f32;
            rsum += c.reserve_security * c.pop as f32;
            tsum += c.task_coverage * c.pop as f32;
            wn += c.pop as f32;
        }
        let home_dist = if wn > 0.0 { dsum / wn } else { 0.0 };
        let on_terr = if wn > 0.0 { osum / wn } else { 0.0 };
        let logistics = if wn > 0.0 { lsum / wn } else { 0.0 };
        let hauling = if wn > 0.0 { hsum / wn } else { 0.0 };
        let road_utility = if wn > 0.0 { usum / wn } else { 0.0 };
        let reserve = if wn > 0.0 { rsum / wn } else { 0.0 };
        let tasks = if wn > 0.0 { tsum / wn } else { 0.0 };
        let modes = mode_histogram(&w);
        let modestr: Vec<String> = modes.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        println!(
            "{:>6} | {:>4} {:>4} {:>4} | {:>6} {:>6} {:>6} | {:>8.1} {:>6.0}% {:>7} | {:>3.0}% {:>3.0}% {:>3.0}% {:>3.0}% {:>3.0}% | {}",
            t,
            total_pop,
            w.clan_count(),
            w.leader_count(),
            total_clan_food,
            w.deaths_starved,
            w.deaths_combat,
            home_dist,
            on_terr * 100.0,
            total_terr,
            logistics * 100.0,
            hauling * 100.0,
            road_utility * 100.0,
            reserve * 100.0,
            tasks * 100.0,
            modestr.join(" ")
        );
    }

    // final detail
    println!("\n-- final per-clan ({label}) --");
    let mut clans = snapshot_clans(&w);
    clans.sort_by_key(|c| std::cmp::Reverse(c.pop));
    for c in &clans {
        println!(
            "  clan#{:<3} pop{:<4} terr{:<5} food{:<5} wood{:<4} reserve{:<4} roads{:<3} {:<8} K{:<3} L{:<3} R{:<3} home{:>5.1} max{:>5.1} onTerr{:>4.0}% logistics{:>3.0}% haul{:>3.0}% road{:>3.0}% reserve{:>3.0}% tasks{:>3.0}%",
            c.id, c.pop, c.territory, c.food, c.wood, c.reserve_food, c.roads_built,
            c.mode.label(), c.kills, c.losses, c.recruits,
            c.home_dist, c.max_dist, c.on_terr * 100.0, c.logistics * 100.0,
            c.hauling_throughput * 100.0, c.road_utility * 100.0,
            c.reserve_security * 100.0, c.task_coverage * 100.0
        );
    }
    println!("\n-- final goals ({label}) --");
    for (k, v) in goal_histogram(&w) {
        println!("  {k:<22} {v}");
    }
    let gens: Vec<u32> = w
        .clans
        .iter()
        .filter(|c| !c.disbanded)
        .map(|c| c.brain.generation)
        .collect();
    let avg_gen = if gens.is_empty() {
        0.0
    } else {
        gens.iter().sum::<u32>() as f32 / gens.len() as f32
    };
    let max_gen = gens.iter().copied().max().unwrap_or(0);
    println!(
        "\nSUMMARY {label}: pop={} clans={} born={} starved={} killed={} food_on_map={} owned_tiles={} brain_gen(avg/max)={:.1}/{}",
        w.population(),
        w.clan_count(),
        w.births,
        w.deaths_starved,
        w.deaths_combat,
        w.pellet_count(),
        clans.iter().map(|c| c.territory).sum::<u32>(),
        avg_gen,
        max_gen,
    );
}

#[test]
fn diag_gentle() {
    report(
        "GENTLE 40k",
        gentle_world(0x1234_5678_9abc_def0),
        40_000,
        2000,
    );
}

/// Train brains in the arena for a few generations, then drop the champion into
/// a fresh world (every clan gets the trained brain) and watch it play. This is
/// the "do trained AIs thrive?" check — compare its settle/expand/fight stats to
/// the random-brain baseline above.
#[test]
fn diag_trained() {
    use crate::trainer::{evaluate_parallel, TrainCfg, Trainer};
    let mut cfg = TrainCfg::default();
    cfg.pop_size = 36;
    cfg.episode_ticks = 4000;
    cfg.clans_per_arena = 6;
    cfg.repeats = 2;
    cfg.world_size = 130;
    let gens = 12;
    let mut tr = Trainer::new(cfg);
    println!("\n== training {gens} generations ==");
    for _ in 0..gens {
        let pop = tr.population.clone();
        let g = tr.generation;
        let scores = evaluate_parallel(&pop, &tr.cfg, g);
        tr.finish_generation(pop, scores, 0.0);
        println!(
            "gen {:>2}: best={:.0} avg={:.0} best_ever={:.0}",
            tr.generation, tr.best_fitness, tr.avg_fitness, tr.best_ever
        );
    }
    let champ = tr.best_brain.clone().expect("a champion");
    // Showcase: a world where all clans run the trained champion brain.
    let mut p = Params::default();
    p.vision_radius = 17;
    let mut w = World::new(220, 0x1234_5678_9abc_def0);
    w.params = p;
    w.maintain_clans = 5;
    w.populate(60, 55, 0); // no random clans
    for _ in 0..5 {
        w.seed_clan(champ.clone());
    }
    report("TRAINED CHAMPION 24k", w, 24_000, 4000);
}

/// Long unattended evolution. Ignored by default; launch explicitly with:
///   LIFE_TRAIN_HOURS=8 cargo test --release train_marathon -- --ignored --nocapture
/// Saves the champion to `champion.bin` (which the app auto-loads) and appends a
/// per-generation log to `training-log.txt`. Continues from an existing champion.
#[test]
#[ignore]
fn train_marathon() {
    use crate::trainer::{train_marathon, TrainCfg, CHAMPION_PATH};
    let hours = std::env::var("LIFE_TRAIN_HOURS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(8.0);
    // Use every logical core for the arenas (the test harness doesn't call the
    // app's rayon setup, so configure the global pool explicitly here).
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cores)
        .build_global();
    println!("marathon: using {cores} rayon threads");
    // Tuned for quality over an 8h unattended run: a large diverse population,
    // multi-season episodes so fitness rewards villages that survive winters,
    // and enough repeats for stable scores. Scales across all CPU cores.
    let mut cfg = TrainCfg::default();
    cfg.pop_size = 160;
    cfg.episode_ticks = 9000;
    cfg.clans_per_arena = 6;
    cfg.repeats = 4;
    cfg.world_size = 130;
    cfg.elite = 8;
    println!("marathon: {hours}h → {CHAMPION_PATH} (log: training-log.txt)");
    train_marathon(hours, cfg, CHAMPION_PATH, "training-log.txt");
}

/// Drop the *saved marathon champion* (`champion.bin`) into a live-default world
/// — all clans run it — and watch it play. This is the "did the 8h champion turn
/// out good?" check: look for tight villages, high onTerr%, expansion, and combat.
#[test]
fn diag_showcase() {
    use crate::brain::Brain;
    use crate::trainer::CHAMPION_PATH;
    let champ = match Brain::load(CHAMPION_PATH) {
        Ok(b) => {
            println!("loaded {CHAMPION_PATH} (generation {})", b.generation);
            b
        }
        Err(e) => {
            println!("no champion to showcase ({e})");
            return;
        }
    };
    let mut p = Params::default();
    p.vision_radius = 17;
    let mut w = World::new(220, 0x1234_5678_9abc_def0);
    w.params = p;
    w.maintain_clans = 5;
    w.populate(60, 50, 0); // no random clans — champion only
    for _ in 0..5 {
        w.seed_clan(champ.clone());
    }
    report("TRAINED CHAMPION showcase 24k", w, 24_000, 4000);
}

/// Causal Community Logistics V1.1 report: every enabled world is paired with
/// a logistics-disabled control built from the exact same world spec and seed.
#[test]
fn diag_logistics_ablation() {
    use crate::brain::Brain;
    use crate::trainer::{benchmark_logistics_quality, CHAMPION_PATH, MAX_STAGE};
    let champ = match Brain::load(CHAMPION_PATH) {
        Ok(brain) => brain,
        Err(error) => {
            println!("no champion to benchmark ({error})");
            return;
        }
    };
    let report =
        benchmark_logistics_quality(&champ, &Params::default(), MAX_STAGE, 4000, 13, 0x51FE_BEEF);
    println!("\n== Community Logistics V1.1 paired ablation ==");
    println!("worlds: {}", report.worlds);
    println!("enabled : {:#?}", report.enabled);
    println!("disabled: {:#?}", report.disabled);
    println!(
        "deltas: clan survival {:+.3}, security {:+.3}, haul {:+.3}, road utility {:+.3}, reserve use/score {:+.3}/{:+.3}",
        report.clan_survival_delta,
        report.security_delta,
        report.hauling_throughput_delta,
        report.road_utility_delta,
        report.reserve_use_delta,
        report.reserve_security_delta,
    );
    println!(
        "survival non-regression: {}",
        report.survival_non_regression
    );
}

/// Probe the trained champion's mixture-of-experts: show qualitative routing
/// examples, then report the authoritative contextual-specialization contract.
#[test]
fn diag_subminds() {
    use crate::brain::{Brain, N_EXPERTS, N_IN, OUT_LABELS, SUBMIND_LABELS};
    use crate::quality::contextual_specialization_metrics;
    use crate::trainer::CHAMPION_PATH;
    let champ = match Brain::load(CHAMPION_PATH) {
        Ok(b) => b,
        Err(e) => {
            println!("no champion to probe ({e})");
            return;
        }
    };
    let base = || {
        let mut v = [0.0f32; N_IN];
        v[0] = 0.3; // some population
        v[1] = 0.5; // moderate food
        v[31] = 1.0; // bias
        v
    };
    let mode_of = |o: &[f32]| {
        (0..6)
            .max_by(|&a, &b| o[a].partial_cmp(&o[b]).unwrap())
            .unwrap()
    };
    let situations: [(&str, fn(&mut [f32; N_IN])); 6] = [
        ("peace/growth ", |v| {
            v[2] = 0.1;
            v[4] = 0.8;
            v[11] = 1.0;
            v[13] = 0.8;
        }),
        ("famine/winter", |v| {
            v[1] = 0.05;
            v[2] = 0.85;
            v[13] = 0.1;
            v[15] = -1.0;
        }),
        ("war/threat   ", |v| {
            v[6] = 0.8;
            v[8] = 0.6;
            v[12] = 1.0;
        }),
        ("crowded      ", |v| {
            v[0] = 0.9;
            v[3] = 0.95;
            v[4] = 0.05;
            v[11] = 1.0;
        }),
        ("disaster     ", |v| {
            v[2] = 0.5;
            v[26] = 0.8;
            v[27] = 1.0;
        }),
        ("recruits near", |v| {
            v[7] = 0.8;
            v[4] = 0.6;
        }),
    ];
    println!("\n== champion sub-mind routing ({N_EXPERTS} experts) ==");
    for (name, set) in situations.iter() {
        let mut v = base();
        set(&mut v);
        let (out, gate) = champ.evaluate(&v);
        let g: Vec<String> = (0..N_EXPERTS)
            .map(|i| format!("{}:{:.2}", SUBMIND_LABELS[i], gate[i]))
            .collect();
        let experts = champ.expert_outputs(&v);
        let ex: Vec<String> = experts
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{}>{}", SUBMIND_LABELS[i], OUT_LABELS[mode_of(o)]))
            .collect();
        println!(
            "[{name}] gate {} -> {} (aggr {:.2}) | experts want: {}",
            g.join(" "),
            OUT_LABELS[mode_of(&out)],
            out[6],
            ex.join(" ")
        );
    }
    let specialization = contextual_specialization_metrics(&champ);
    println!(
        "specialization: {:.2} ({})",
        specialization.specialization_score(),
        if specialization.qualifies() {
            "meets contract"
        } else {
            "below contract"
        }
    );
    println!(
        "  utilization {:.2} · decisiveness {:.2} · context information {:.2} · top-1 coverage {:.2} · expert divergence {:.2}",
        specialization.utilization_balance,
        specialization.decisiveness,
        specialization.contextual_mutual_information,
        specialization.contextual_top1_coverage,
        specialization.expert_output_divergence,
    );
}

#[test]
fn brain_save_load_roundtrip() {
    use crate::brain::Brain;
    use crate::rng::Rng;
    let mut rng = Rng::new(12345);
    let b = Brain::random(&mut rng);
    let path = "test_brain_roundtrip.bin";
    b.save(path).expect("save");
    let loaded = Brain::load(path).expect("load");
    let inputs = [0.3f32; crate::brain::N_IN];
    let (o1, g1) = b.evaluate(&inputs);
    let (o2, g2) = loaded.evaluate(&inputs);
    for i in 0..crate::brain::N_OUT {
        assert!(
            (o1[i] - o2[i]).abs() < 1e-6,
            "output {i} differs after roundtrip"
        );
    }
    for i in 0..crate::brain::N_EXPERTS {
        assert!(
            (g1[i] - g2[i]).abs() < 1e-6,
            "gate {i} differs after roundtrip"
        );
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn diag_seeds() {
    // Same economy, several seeds — check the village/war dynamic isn't a fluke.
    for s in [1u64, 7, 99] {
        report(&format!("SEED {s} 24k"), gentle_world(s), 24_000, 4000);
    }
}
