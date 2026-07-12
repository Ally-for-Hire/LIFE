//! The native application: a game-like, watchable playground.
//!
//! No HTML, no browser, no HTTP. The simulation and the renderer live in one
//! process and share memory — the view reads world state directly instead of
//! polling JSON snapshots (the bottleneck the old browser dashboard had).
//!
//! Milestone 1 wiring: variable 1..=5000 ticks/s, pan/zoom viewport, toggleable
//! panels, a click-to-inspect NPC "idea" readout, and live progress graphs.

use crate::entity::Goal;
use crate::trainer::{arena_count, TrainCfg, Trainer};
use crate::world::{Params, World};
use eframe::egui;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// One-click economy scenarios. The default is survival-first growth:
/// clans should stabilize, recruit, and expand before scarcity or war matters.
#[derive(Clone, Copy, PartialEq)]
enum Preset {
    GentleScarcity,
    Balanced,
    Buffet,
    Famine,
}

/// Rolling time-series for the progress graphs.
struct History {
    pop: Vec<[f64; 2]>,
    pellets: Vec<[f64; 2]>,
    leaders: Vec<[f64; 2]>,
    clans: Vec<[f64; 2]>,
    deaths: Vec<[f64; 2]>,
    combat: Vec<[f64; 2]>,
    cap: usize,
}

impl History {
    fn new() -> Self {
        History {
            pop: Vec::new(),
            pellets: Vec::new(),
            leaders: Vec::new(),
            clans: Vec::new(),
            deaths: Vec::new(),
            combat: Vec::new(),
            cap: 1200,
        }
    }
    fn clear(&mut self) {
        self.pop.clear();
        self.pellets.clear();
        self.leaders.clear();
        self.clans.clear();
        self.deaths.clear();
        self.combat.clear();
    }
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        tick: f64,
        pop: f64,
        pellets: f64,
        leaders: f64,
        clans: f64,
        deaths: f64,
        combat: f64,
    ) {
        self.pop.push([tick, pop]);
        self.pellets.push([tick, pellets]);
        self.leaders.push([tick, leaders]);
        self.clans.push([tick, clans]);
        self.deaths.push([tick, deaths]);
        self.combat.push([tick, combat]);
        if self.pop.len() > self.cap {
            let drop = self.pop.len() - self.cap;
            self.pop.drain(0..drop);
            self.pellets.drain(0..drop);
            self.leaders.drain(0..drop);
            self.clans.drain(0..drop);
            self.deaths.drain(0..drop);
            self.combat.drain(0..drop);
        }
    }
}

pub struct LifeApp {
    world: World,
    seed: u64,

    // run state
    running: bool,
    tps: f32,
    tick_accum: f64,

    // populate knobs
    p_entities: i32,
    p_trees: i32,
    p_clans: i32,
    p_size: i32,
    maintain_on: bool,
    p_maintain: i32,

    // view
    zoom: f32,
    pan: egui::Vec2,
    tex: Option<egui::TextureHandle>,
    selected: Option<u32>,

    // panel toggles
    show_controls: bool,
    show_inspector: bool,
    show_graphs: bool,
    show_training: bool,

    // training (runs on a background thread; shared via mutex)
    trainer: Arc<Mutex<Trainer>>,
    train_running: Arc<AtomicBool>,
    train_stop: Arc<AtomicBool>,

    // graphs + metering
    hist: History,
    last_sample_tick: i32,
    last_time: f64,
    last_tick: i32,
    measured_tps: f64,
    last_deaths: u64,
    deaths_rate: f64,
}

impl LifeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let seed = 0x1234_5678_9abc_def0;

        let trainer = Arc::new(Mutex::new(Trainer::new(TrainCfg::default())));
        // Continue from a previously trained champion if one is on disk (e.g. an
        // overnight marathon run): the trainer resumes from it and the live world
        // uses it immediately.
        let loaded_champion = crate::brain::Brain::load(crate::trainer::CHAMPION_PATH).ok();
        if let Some(b) = &loaded_champion {
            trainer.lock().unwrap().seed_from(b.clone());
        }
        // Evolution runs from the start: the arena trainer continually breeds
        // better leaders in the background, and the live world inherits them.
        let train_running = Arc::new(AtomicBool::new(true));
        let train_stop = Arc::new(AtomicBool::new(false));
        spawn_training_thread(trainer.clone(), train_running.clone(), train_stop.clone());

        let mut app = LifeApp {
            world: World::new(220, seed),
            seed,
            running: true,
            tps: 60.0,
            tick_accum: 0.0,
            p_entities: 120,
            p_trees: 70,
            p_clans: 5,
            p_size: 220,
            maintain_on: true,
            p_maintain: 120,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            tex: None,
            selected: None,
            show_controls: true,
            show_inspector: true,
            show_graphs: true,
            show_training: true,
            trainer,
            train_running,
            train_stop,
            hist: History::new(),
            last_sample_tick: 0,
            last_time: 0.0,
            last_tick: 0,
            measured_tps: 0.0,
            last_deaths: 0,
            deaths_rate: 0.0,
        };
        app.apply_preset(Preset::GentleScarcity); // default = survival-first growth
        app.world.champion = loaded_champion; // live villages inherit the trained brain
        app
    }

    /// Apply a one-click economy scenario and rebuild the world.
    fn apply_preset(&mut self, preset: Preset) {
        let mut p = Params::default();
        match preset {
            // Survival-first baseline: enough food and grace for clans to
            // stabilize, recruit, and grow before conflict matters.
            // Farms feed the villages; wild trees are a sparse bootstrap. Clans
            // settle, expand onto fertile land, take in refugees, and war over
            // valleys when the seasons turn lean.
            Preset::GentleScarcity => {
                self.p_size = 220;
                self.p_entities = 60;
                self.p_trees = 50;
                self.maintain_on = false;
                self.p_maintain = 120;
                self.p_clans = 5;
                p.tree_interval = 130;
                p.tree_per_cycle = 4;
                p.tree_radius = 8;
                p.max_pellet_fraction = 0.06;
                p.vision_radius = 17;
                p.starve_ticks = 1400;
                p.clan_grace_ticks = 1800;
            }
            Preset::Balanced => {
                self.p_size = 220;
                self.p_entities = 70;
                self.p_trees = 55;
                self.maintain_on = false;
                self.p_maintain = 120;
                self.p_clans = 6;
                p.tree_interval = 130;
                p.tree_per_cycle = 4;
                p.tree_radius = 7;
                p.max_pellet_fraction = 0.055;
                p.vision_radius = 16;
                p.starve_ticks = 1300;
                p.clan_grace_ticks = 1500;
                p.war_threshold = 0.95;
            }
            // Plenty: longer summers, gentler winters, more clans growing fat
            // before they ever need to fight.
            Preset::Buffet => {
                self.p_size = 220;
                self.p_entities = 80;
                self.p_trees = 70;
                self.maintain_on = false;
                self.p_maintain = 120;
                self.p_clans = 5;
                p.tree_interval = 120;
                p.tree_per_cycle = 6;
                p.tree_radius = 8;
                p.max_pellet_fraction = 0.09;
                p.farm_yield = 0.22;
                p.season_amp = 0.35;
                p.vision_radius = 15;
                p.starve_ticks = 1500;
                p.clan_grace_ticks = 2200;
                p.war_threshold = 1.3;
            }
            // Harsh, conflict-heavy: poor soil, brutal winters, sparse wild food.
            // Villages can't grow fat — they must seize each other's valleys to
            // survive the lean season.
            Preset::Famine => {
                self.p_size = 220;
                self.p_entities = 80;
                self.p_trees = 28;
                self.maintain_on = false;
                self.p_maintain = 120;
                self.p_clans = 8;
                p.tree_interval = 220;
                p.tree_per_cycle = 2;
                p.tree_radius = 5;
                p.max_pellet_fraction = 0.03;
                p.farm_yield = 0.07;
                p.season_amp = 0.75;
                p.season_length = 2400;
                p.vision_radius = 13;
                p.starve_ticks = 800;
                p.clan_grace_ticks = 400;
                p.war_threshold = 0.7;
            }
        }
        self.world = World::new(self.p_size, self.seed);
        self.world.params = p;
        self.world.maintain_pop = if self.maintain_on { self.p_maintain } else { 0 };
        self.world.maintain_clans = self.p_clans; // keep a living patchwork of villages
        self.world
            .populate(self.p_entities, self.p_trees, self.p_clans);
        self.selected = None;
        self.hist.clear();
        self.last_sample_tick = 0;
        self.last_deaths = 0;
        self.tex = None;
        self.tick_accum = 0.0;
    }

    fn repopulate(&mut self) {
        let params = self.world.params.clone(); // keep the user's tuning
        self.world = World::new(self.p_size, self.seed);
        self.world.params = params;
        self.world.maintain_pop = if self.maintain_on { self.p_maintain } else { 0 };
        self.world.maintain_clans = self.p_clans; // keep a living patchwork of villages
        self.world
            .populate(self.p_entities, self.p_trees, self.p_clans);
        self.selected = None;
        self.hist.clear();
        self.last_sample_tick = 0;
        self.tex = None; // grid dims may have changed
        self.tick_accum = 0.0;
    }

    /// Paint the whole world into a pixel buffer (one cell = one pixel),
    /// uploaded as a NEAREST-filtered texture and scaled in the viewport.
    fn build_image(&self) -> egui::ColorImage {
        let g = &self.world.grid;
        let w = g.size as usize;
        let n = w * w;

        // base terrain layer
        let mut px = vec![egui::Color32::BLACK; n];
        for i in 0..n {
            px[i] = terrain_color(g.terrain[i]);
        }

        // clan color lookup
        let mut clan_col: HashMap<i32, egui::Color32> = HashMap::new();
        for c in &self.world.clans {
            if !c.disbanded {
                clan_col.insert(
                    c.id,
                    egui::Color32::from_rgb(c.color[0], c.color[1], c.color[2]),
                );
            }
        }

        // territory tint over the terrain
        for i in 0..n {
            let o = g.owner[i];
            if o != crate::grid::NO_OWNER {
                if let Some(col) = clan_col.get(&o) {
                    px[i] = blend(px[i], *col, 0.32);
                }
            }
        }
        // Community logistics overlays. Wood remains visibly tied to forest
        // terrain; roads sit above terrain/territory so their shared benefit is
        // legible even at low zoom.
        for i in 0..n {
            if g.wood[i] > 0 {
                let abundance = (g.wood[i] as f32 / 16.0).clamp(0.2, 1.0);
                px[i] = blend(
                    px[i],
                    egui::Color32::from_rgb(158, 104, 58),
                    0.30 + abundance * 0.35,
                );
            }
            if g.road[i] > 0 {
                px[i] = if self.world.params.community_logistics {
                    egui::Color32::from_rgb(184, 154, 102)
                } else {
                    egui::Color32::from_rgb(104, 106, 110)
                };
            }
        }
        // pellets
        for i in 0..n {
            if g.pellet[i] > 0 {
                px[i] = egui::Color32::from_rgb(64, 168, 96);
            }
        }
        // trees
        for t in &self.world.trees {
            if !t.destroyed && g.in_bounds(t.x, t.y) {
                px[g.idx(t.x, t.y)] = egui::Color32::from_rgb(54, 150, 80);
            }
        }
        // stockpiles
        for c in &self.world.clans {
            if c.disbanded {
                continue;
            }
            if let Some((sx, sy)) = c.stockpile {
                if g.in_bounds(sx, sy) {
                    px[g.idx(sx, sy)] = egui::Color32::from_rgb(232, 212, 92);
                }
            }
        }
        // entities
        for e in &self.world.entities {
            let c = if e.incapacitated_until > self.world.tick {
                egui::Color32::from_rgb(238, 126, 82)
            } else if e.clan >= 0 {
                let base = clan_col
                    .get(&e.clan)
                    .copied()
                    .unwrap_or(egui::Color32::from_rgb(200, 206, 216));
                if e.is_leader {
                    lighten(base, 0.55)
                } else {
                    base
                }
            } else {
                match e.goal {
                    Goal::Starving => egui::Color32::from_rgb(222, 92, 92),
                    Goal::Eating => egui::Color32::from_rgb(150, 222, 150),
                    Goal::SeekFood => egui::Color32::from_rgb(200, 180, 120),
                    _ => egui::Color32::from_rgb(168, 174, 184),
                }
            };
            px[g.idx(e.x, e.y)] = c;
        }

        egui::ColorImage {
            size: [w, w],
            pixels: px,
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let img = self.build_image();
        match self.tex.as_mut() {
            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
            None => {
                self.tex = Some(ctx.load_texture("world", img, egui::TextureOptions::NEAREST));
            }
        }
    }
}

impl eframe::App for LifeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- advance the sim by wall-clock at the chosen tick rate ---
        let dt = (ctx.input(|i| i.stable_dt) as f64).min(0.1);
        if self.running {
            self.tick_accum += self.tps as f64 * dt;
            let mut steps = self.tick_accum.floor() as i64;
            self.tick_accum -= steps as f64;
            let max_steps = 6000; // safety cap so a hitch can't spiral
            if steps > max_steps {
                steps = max_steps;
                self.tick_accum = 0.0;
            }
            for _ in 0..steps {
                self.world.step();
            }
        }

        // --- meter achieved ticks/s and sample the graphs ---
        let now = ctx.input(|i| i.time);
        if now - self.last_time >= 0.5 {
            let win = now - self.last_time;
            self.measured_tps = (self.world.tick - self.last_tick) as f64 / win;
            self.deaths_rate =
                self.world.deaths_starved.saturating_sub(self.last_deaths) as f64 / win;
            self.last_time = now;
            self.last_tick = self.world.tick;
            self.last_deaths = self.world.deaths_starved;
            // Keep the live world's champion in sync with the background trainer
            // so newly-formed villages can inherit evolved strategies automatically.
            if let Ok(t) = self.trainer.try_lock() {
                if t.best_brain.is_some() {
                    self.world.champion = t.best_brain.clone();
                }
            }
        }
        let pellets = self.world.pellet_count();
        if self.running && self.world.tick - self.last_sample_tick >= 25 {
            self.hist.push(
                self.world.tick as f64,
                self.world.population() as f64,
                pellets as f64,
                self.world.leader_count() as f64,
                self.world.clan_count() as f64,
                self.world.deaths_starved as f64,
                self.world.deaths_combat as f64,
            );
            self.last_sample_tick = self.world.tick;
        }

        self.update_texture(ctx);

        // --- top bar ---
        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("LIFE");
                ui.separator();
                if ui
                    .button(if self.running { "⏸ Pause" } else { "▶ Run" })
                    .clicked()
                {
                    self.running = !self.running;
                }
                if ui.button("Step").clicked() {
                    self.world.step();
                }
                ui.separator();
                ui.add(
                    egui::Slider::new(&mut self.tps, 1.0..=5000.0)
                        .logarithmic(true)
                        .text("ticks/s"),
                );
                ui.separator();
                ui.label(format!("tick {}", self.world.tick));
                ui.label(format!("· {} NPCs", self.world.population()));
                ui.label(format!("· {} clans", self.world.clan_count()));
                ui.label(format!("· {} born", self.world.births));
                ui.label(format!("· {} food", pellets));
                ui.label(format!(
                    "· {} starved ({:.1}/s)",
                    self.world.deaths_starved, self.deaths_rate
                ));
                ui.label(format!("· {} killed", self.world.deaths_combat));
                ui.label(format!("· {:.0} tps", self.measured_tps));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_training, "Training");
                    ui.toggle_value(&mut self.show_graphs, "Graphs");
                    ui.toggle_value(&mut self.show_inspector, "Inspector");
                    ui.toggle_value(&mut self.show_controls, "Controls");
                });
            });
        });

        // --- left: controls / knobs ---
        if self.show_controls {
            egui::SidePanel::left("controls")
                .default_width(250.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("PRESETS").small().weak());
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Gentle").clicked() {
                                self.apply_preset(Preset::GentleScarcity);
                            }
                            if ui.button("Balanced").clicked() {
                                self.apply_preset(Preset::Balanced);
                            }
                            if ui.button("Buffet").clicked() {
                                self.apply_preset(Preset::Buffet);
                            }
                            if ui.button("Famine").clicked() {
                                self.apply_preset(Preset::Famine);
                            }
                        });

                        ui.separator();
                        ui.label(egui::RichText::new("SIMULATION").small().weak());
                        ui.horizontal(|ui| {
                            if ui
                                .button(if self.running { "Pause" } else { "Run" })
                                .clicked()
                            {
                                self.running = !self.running;
                            }
                            if ui.button("Step 1").clicked() {
                                self.world.step();
                            }
                            if ui.button("Step 100").clicked() {
                                for _ in 0..100 {
                                    self.world.step();
                                }
                            }
                        });
                        ui.add(
                            egui::Slider::new(&mut self.tps, 1.0..=5000.0)
                                .logarithmic(true)
                                .text("ticks/s"),
                        );

                        ui.separator();
                        ui.label(egui::RichText::new("POPULATE").small().weak());
                        ui.horizontal(|ui| {
                            ui.label("world size");
                            ui.add(egui::DragValue::new(&mut self.p_size).range(48..=1024));
                        });
                        ui.horizontal(|ui| {
                            ui.label("NPCs");
                            ui.add(egui::DragValue::new(&mut self.p_entities).range(0..=50000));
                        });
                        ui.horizontal(|ui| {
                            ui.label("trees");
                            ui.add(egui::DragValue::new(&mut self.p_trees).range(0..=20000));
                        });
                        ui.horizontal(|ui| {
                            ui.label("starting clans");
                            ui.add(egui::DragValue::new(&mut self.p_clans).range(0..=500));
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.maintain_on, "maintain");
                            ui.add_enabled(
                                self.maintain_on,
                                egui::DragValue::new(&mut self.p_maintain).range(0..=50000),
                            );
                        });
                        if ui.button("Populate fresh").clicked() {
                            self.repopulate();
                        }
                        if self.maintain_on != (self.world.maintain_pop > 0)
                            || (self.maintain_on && self.world.maintain_pop != self.p_maintain)
                        {
                            self.world.maintain_pop =
                                if self.maintain_on { self.p_maintain } else { 0 };
                        }

                        params_ui(ui, &mut self.world.params, self.tps);

                        ui.separator();
                        ui.label(egui::RichText::new("VIEW").small().weak());
                        if ui.button("Reset view").clicked() {
                            self.zoom = 1.0;
                            self.pan = egui::Vec2::ZERO;
                        }
                        ui.label(
                            egui::RichText::new("drag = pan · scroll = zoom")
                                .small()
                                .weak(),
                        );

                        ui.separator();
                        ui.label(egui::RichText::new("LEGEND").small().weak());
                        legend_row(ui, egui::Color32::from_rgb(255, 238, 150), "leader");
                        legend_row(
                            ui,
                            egui::Color32::from_rgb(200, 206, 216),
                            "villager (wandering)",
                        );
                        legend_row(ui, egui::Color32::from_rgb(222, 198, 120), "seeking food");
                        legend_row(ui, egui::Color32::from_rgb(150, 222, 150), "eating");
                        legend_row(ui, egui::Color32::from_rgb(222, 92, 92), "starving");
                        legend_row(ui, egui::Color32::from_rgb(64, 168, 96), "food (pellet)");
                        legend_row(ui, egui::Color32::from_rgb(54, 150, 80), "tree");
                        legend_row(
                            ui,
                            egui::Color32::from_rgb(158, 104, 58),
                            "harvestable forest wood",
                        );
                        legend_row(
                            ui,
                            if self.world.params.community_logistics {
                                egui::Color32::from_rgb(184, 154, 102)
                            } else {
                                egui::Color32::from_rgb(104, 106, 110)
                            },
                            if self.world.params.community_logistics {
                                "community road"
                            } else {
                                "road (benefit disabled)"
                            },
                        );
                    });
                });
        }

        // --- right: NPC inspector ("ideas / goals") ---
        if self.show_inspector {
            egui::SidePanel::right("inspector")
                .default_width(230.0)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("NPC INSPECTOR").small().weak());
                    ui.separator();
                    match self.selected.and_then(|id| self.world.entity_by_id(id)) {
                        Some(e) => {
                            ui.heading(format!("NPC #{}", e.id));
                            let role = if e.clan >= 0 {
                                if e.is_leader {
                                    "role: clan leader"
                                } else {
                                    "role: clan member"
                                }
                            } else {
                                "role: neutral"
                            };
                            ui.label(role);
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("idea: {}", e.goal.label())).strong(),
                            );
                            if e.clan >= 0 {
                                ui.label(format!("community role: {}", e.work_role.label()));
                                ui.label(format!(
                                    "role commitment: {} ticks",
                                    (e.work_until - self.world.tick).max(0)
                                ));
                                if e.incapacitated_until > self.world.tick {
                                    ui.label(format!(
                                        "rescue window: {} ticks",
                                        e.incapacitated_until - self.world.tick
                                    ));
                                }
                            }
                            let hunger = e.hunger(self.world.params.starve_ticks);
                            ui.add(
                                egui::ProgressBar::new(e.health / e.max_health)
                                    .text(format!("health {:.1}/{:.0}", e.health, e.max_health)),
                            );
                            ui.add(
                                egui::ProgressBar::new(hunger.min(1.0))
                                    .text(format!("hunger {:.0}%", hunger * 100.0)),
                            );
                            ui.label(format!("carried food: {}", e.food));
                            ui.label(format!("carried wood: {}", e.wood));
                            ui.label(format!("speed: {:.2} cells/tick", e.speed));
                            ui.label(format!("position: {}, {}", e.x, e.y));

                            if let Some(c) = self.world.clan_by_id(e.clan) {
                                ui.separator();
                                ui.label(egui::RichText::new("CLAN").small().weak());
                                ui.label(
                                    egui::RichText::new(format!(
                                        "#{} — order: {}",
                                        c.id,
                                        c.mode.label()
                                    ))
                                    .strong(),
                                );
                                ui.label(format!("members: {}", self.world.clan_population(c.id)));
                                ui.label(format!("stockpile food: {}", c.food));
                                ui.label(format!("emergency reserve: {} food", c.reserve_food));
                                ui.label(format!("stockpile wood: {}", c.wood));
                                ui.label(if self.world.params.community_logistics {
                                    "logistics infrastructure: enabled"
                                } else {
                                    "logistics infrastructure: disabled (ablation)"
                                });
                                ui.label(if self.world.params.community_care {
                                    "community care: enabled"
                                } else {
                                    "community care: disabled (ablation)"
                                });
                                ui.label(format!("territory: {} tiles", c.territory));
                                ui.label(format!("aggression: {:.2}", c.aggression));
                                ui.label(format!(
                                    "kills {} · losses {} · recruited {}",
                                    c.stats.kills, c.stats.losses, c.stats.recruits
                                ));
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new("community workforce:").small().weak(),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    for (i, &count) in c.workforce.iter().enumerate() {
                                        if count > 0 {
                                            ui.label(format!(
                                                "{} {}",
                                                crate::clan::ClanMode::from_index(i).label(),
                                                count
                                            ));
                                        }
                                    }
                                });
                                ui.label(format!(
                                    "deliveries: {} food / {} wood",
                                    c.stats.food_delivered, c.stats.wood_delivered
                                ));
                                ui.label(format!(
                                    "roads: {} built / {} member-steps / {:.2} move cost saved",
                                    c.stats.roads_built,
                                    c.stats.road_steps,
                                    c.stats.road_cost_saved_milli as f32 / 1000.0
                                ));
                                ui.label(format!(
                                    "reserve: {} deposited / {} released",
                                    c.stats.reserve_deposited, c.stats.reserve_released
                                ));
                                ui.label(format!(
                                    "care: {} rescued / {} incapacitated / {} bled out",
                                    c.stats.rescues,
                                    c.stats.incapacitations,
                                    c.stats.bleedouts
                                ));
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new("master → sub-mind routing:")
                                        .small()
                                        .weak(),
                                );
                                for (i, label) in crate::brain::SUBMIND_LABELS.iter().enumerate() {
                                    let v = c.brain.last_gate[i];
                                    ui.add(
                                        egui::ProgressBar::new(v).text(format!("{label} {v:.2}")),
                                    );
                                }
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new("blended action utilities:")
                                        .small()
                                        .weak(),
                                );
                                for (i, label) in crate::brain::OUT_LABELS.iter().enumerate() {
                                    let v = c.brain.last_out[i];
                                    ui.add(
                                        egui::ProgressBar::new(v).text(format!("{label} {v:.2}")),
                                    );
                                }
                            }
                        }
                        None => {
                            self.selected = None;
                            ui.label("Click an NPC in the world to read its current goal & idea.");
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(egui::RichText::new("CLANS").small().weak());
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            let mut clans: Vec<&crate::clan::Clan> =
                                self.world.clans.iter().filter(|c| !c.disbanded).collect();
                            clans.sort_by_key(|c| {
                                std::cmp::Reverse(self.world.clan_population(c.id))
                            });
                            if clans.is_empty() {
                                ui.label(egui::RichText::new("no clans").weak());
                            }
                            for c in clans {
                                let col =
                                    egui::Color32::from_rgb(c.color[0], c.color[1], c.color[2]);
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(r, 2.0, col);
                                    ui.label(format!(
                                        "#{} {} · {}p · {}f+{}r · {}w · {} roads · {} rescues · K{} L{}",
                                        c.id,
                                        c.mode.label(),
                                        self.world.clan_population(c.id),
                                        c.food,
                                        c.reserve_food,
                                        c.wood,
                                        c.stats.roads_built,
                                        c.stats.rescues,
                                        c.stats.kills,
                                        c.stats.losses
                                    ));
                                });
                            }
                        });
                });
        }

        // --- bottom: progress graphs ---
        if self.show_graphs {
            egui::TopBottomPanel::bottom("graphs")
                .resizable(true)
                .default_height(170.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("PROGRESS").small().weak());
                    ui.columns(3, |cols| {
                        egui_plot::Plot::new("pop_plot")
                            .height(120.0)
                            .allow_scroll(false)
                            .show(&mut cols[0], |p| {
                                p.line(
                                    egui_plot::Line::new(self.hist.pop.clone()).name("population"),
                                );
                                p.line(
                                    egui_plot::Line::new(self.hist.leaders.clone()).name("leaders"),
                                );
                                p.line(egui_plot::Line::new(self.hist.clans.clone()).name("clans"));
                            });
                        egui_plot::Plot::new("food_plot")
                            .height(120.0)
                            .allow_scroll(false)
                            .show(&mut cols[1], |p| {
                                p.line(
                                    egui_plot::Line::new(self.hist.pellets.clone()).name("food"),
                                );
                            });
                        egui_plot::Plot::new("deaths_plot")
                            .height(120.0)
                            .allow_scroll(false)
                            .show(&mut cols[2], |p| {
                                p.line(
                                    egui_plot::Line::new(self.hist.deaths.clone())
                                        .name("starvation"),
                                );
                                p.line(
                                    egui_plot::Line::new(self.hist.combat.clone()).name("combat"),
                                );
                            });
                    });
                });
        }

        // --- center: the world viewport ---
        egui::CentralPanel::default().show(ctx, |ui| {
            let size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

            if response.dragged() {
                self.pan += response.drag_delta();
            }
            if response.hovered() {
                let scroll = ui.input(|i| i.raw_scroll_delta.y);
                if scroll != 0.0 {
                    let factor = (scroll * 0.0015).exp();
                    self.zoom = (self.zoom * factor).clamp(0.2, 80.0);
                }
            }

            let base = rect.width().min(rect.height());
            let world_px = base * self.zoom;
            let center = rect.center() + self.pan;
            let img_rect = egui::Rect::from_center_size(center, egui::Vec2::splat(world_px));
            let cell = world_px / self.world.grid.size as f32;

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 10, 13));
            if let Some(tex) = &self.tex {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(tex.id(), img_rect, uv, egui::Color32::WHITE);
            }

            if let Some(e) = self.selected.and_then(|id| self.world.entity_by_id(id)) {
                let p = img_rect.min + egui::vec2(e.x as f32 * cell, e.y as f32 * cell);
                painter.rect_stroke(
                    egui::Rect::from_min_size(p, egui::Vec2::splat(cell.max(3.0))),
                    0.0,
                    egui::Stroke::new(1.5, egui::Color32::YELLOW),
                );
            }

            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let cx = ((p.x - img_rect.min.x) / cell).floor() as i32;
                    let cy = ((p.y - img_rect.min.y) / cell).floor() as i32;
                    if self.world.grid.in_bounds(cx, cy) {
                        self.selected = self.world.entity_near(cx, cy, 4);
                    }
                }
            }
        });

        // --- training window (floating, toggleable) ---
        if self.show_training {
            let mut open = true;
            egui::Window::new("Training — leader evolution")
                .open(&mut open)
                .default_width(360.0)
                .show(ctx, |ui| {
                    let mut t = self.trainer.lock().unwrap();
                    let running = self.train_running.load(Ordering::Relaxed);
                    ui.horizontal(|ui| {
                        if ui
                            .button(if running {
                                "⏸ Stop"
                            } else {
                                "▶ Start training"
                            })
                            .clicked()
                        {
                            self.train_running.store(!running, Ordering::Relaxed);
                        }
                        if ui.button("Reset population").clicked() {
                            t.reset();
                        }
                    });
                    if running {
                        ui.label(
                            egui::RichText::new(format!(
                                "training on {} CPU threads | {} arenas/gen",
                                rayon::current_num_threads(),
                                arena_count(t.population.len(), &t.cfg)
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(120, 200, 140)),
                        );
                    }
                    ui.separator();
                    egui::Grid::new("train_stats")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("generation");
                            ui.label(format!("{}", t.generation));
                            ui.end_row();
                            ui.label("best fitness");
                            ui.label(format!("{:.1}", t.best_fitness));
                            ui.end_row();
                            ui.label("avg fitness");
                            ui.label(format!("{:.1}", t.avg_fitness));
                            ui.end_row();
                            ui.label("best ever");
                            ui.label(format!(
                                "{:.1}",
                                if t.best_ever == f32::MIN {
                                    0.0
                                } else {
                                    t.best_ever
                                }
                            ));
                            ui.end_row();
                            ui.label("last gen time");
                            ui.label(format!("{:.0} ms", t.last_gen_ms));
                            ui.end_row();
                            ui.label("stagnation");
                            ui.label(format!("{} gens", t.stagnant_generations));
                            ui.end_row();
                            ui.label("curriculum stage");
                            ui.label(format!("{} / {}", t.stage, crate::trainer::MAX_STAGE));
                            ui.end_row();
                            ui.label("hall of fame");
                            ui.label(format!("{} champions", t.hof_len()));
                            ui.end_row();
                            ui.label("robust survival");
                            ui.label(format!("{:.0}%", t.robust_survival * 100.0));
                            ui.end_row();
                            ui.label("food security");
                            ui.label(format!("{:.0}%", t.mean_security * 100.0));
                            ui.end_row();
                            ui.label("community logistics");
                            ui.label(format!("{:.0}%", t.mean_logistics * 100.0));
                            ui.end_row();
                            ui.label("hauling throughput");
                            ui.label(format!("{:.0}%", t.mean_hauling_throughput * 100.0));
                            ui.end_row();
                            ui.label("road utility");
                            ui.label(format!("{:.0}%", t.mean_road_utility * 100.0));
                            ui.end_row();
                            ui.label("reserve security");
                            ui.label(format!("{:.0}%", t.mean_reserve_security * 100.0));
                            ui.end_row();
                            ui.label("task coverage");
                            ui.label(format!("{:.0}%", t.mean_task_coverage * 100.0));
                            ui.end_row();
                            ui.label("community care");
                            ui.label(format!("{:.0}%", t.mean_care * 100.0));
                            ui.end_row();
                            ui.label("clan fairness floor");
                            ui.label(format!("{:+.0}%", t.fairness_margin * 100.0));
                            ui.end_row();
                            ui.label("routing balance");
                            ui.label(format!("{:.0}%", t.routing_balance * 100.0));
                            ui.end_row();
                            ui.label("strategy archive");
                            ui.label(format!(
                                "{} / {} niches",
                                t.qd_archive_len(),
                                crate::quality::N_STRATEGY_NICHES
                            ));
                            ui.end_row();
                            ui.label("adaptive mutation");
                            ui.label(format!(
                                "{:.2} / {:.2}",
                                t.adaptive_mutation_rate, t.adaptive_mutation_strength
                            ));
                            ui.end_row();
                        });
                    let niches = t.qd_archive_summary();
                    if !niches.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                niches
                                    .iter()
                                    .map(|(name, quality)| format!("{name} {quality:.2}"))
                                    .collect::<Vec<_>>()
                                    .join("  |  "),
                            )
                            .small()
                            .weak(),
                        );
                    }
                    ui.separator();
                    egui::CollapsingHeader::new("Config")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Slider::new(&mut t.cfg.pop_size, 8..=512).text("population"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.episode_ticks, 500..=20000)
                                    .text("episode ticks"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.clans_per_arena, 2..=16)
                                    .text("clans / arena"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.repeats, 1..=64).text("min repeats"),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} effective arenas/gen",
                                    arena_count(t.population.len(), &t.cfg)
                                ))
                                .small()
                                .weak(),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.world_size, 64..=256)
                                    .text("arena size"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.arena_trees, 0..=400)
                                    .text("arena trees"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.arena_neutrals, 0..=400)
                                    .text("arena neutrals"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.mutation_rate, 0.0..=1.0)
                                    .text("mutation rate"),
                            );
                            ui.add(
                                egui::Slider::new(&mut t.cfg.mutation_strength, 0.0..=1.5)
                                    .text("mutation strength"),
                            );
                            ui.add(egui::Slider::new(&mut t.cfg.elite, 0..=16).text("elite"));
                        });

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                t.best_brain.is_some(),
                                egui::Button::new("Seed best brain → live world"),
                            )
                            .clicked()
                        {
                            if let Some(b) = t.best_brain.clone() {
                                self.world.seed_clan(b);
                            }
                        }
                        if ui
                            .add_enabled(t.best_brain.is_some(), egui::Button::new("💾 Save"))
                            .on_hover_text(format!(
                                "save champion to {}",
                                crate::trainer::CHAMPION_PATH
                            ))
                            .clicked()
                        {
                            let _ = t.save_champion(crate::trainer::CHAMPION_PATH);
                        }
                    });

                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("fitness over generations")
                            .small()
                            .weak(),
                    );
                    let best = t.history.clone();
                    let avg = t.avg_history.clone();
                    egui_plot::Plot::new("fitness_plot")
                        .height(150.0)
                        .allow_scroll(false)
                        .show(ui, |p| {
                            p.line(egui_plot::Line::new(best).name("best"));
                            p.line(egui_plot::Line::new(avg).name("avg"));
                        });
                });
            self.show_training = open;
        }

        if self.running || self.train_running.load(Ordering::Relaxed) {
            ctx.request_repaint();
        }
    }
}

fn spawn_training_thread(
    trainer: Arc<Mutex<Trainer>>,
    running: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if !running.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(40));
            continue;
        }
        // snapshot under a brief lock, evaluate unlocked (so the UI stays smooth)
        let (pop, cfg, gen, stage, hof) = {
            let t = trainer.lock().unwrap();
            let (stage, hof) = t.snapshot_curriculum();
            (
                t.population.clone(),
                t.cfg.clone(),
                t.generation,
                stage,
                hof,
            )
        };
        let start = std::time::Instant::now();
        // domain-randomised, self-play evaluation so the GUI trainer also breeds
        // generally-strong leaders rather than overfitting one world.
        let scores = crate::trainer::evaluate_general_quality(
            &pop,
            &cfg.arena_params,
            gen,
            stage,
            &hof,
            cfg.seed,
            cfg.episode_ticks,
            cfg.clans_per_arena,
        );
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut t = trainer.lock().unwrap();
        // skip if the population was reset/replaced while we were evaluating
        if t.generation == gen && t.population.len() == pop.len() {
            t.finish_general(pop, scores, ms);
        }
    });
}

impl Drop for LifeApp {
    fn drop(&mut self) {
        self.train_stop.store(true, Ordering::Relaxed);
    }
}

/// Live world-parameter sliders. Bound straight to `world.params`, so every
/// change takes effect on the next tick — no rebuild, no repopulate.
fn params_ui(ui: &mut egui::Ui, p: &mut Params, tps: f32) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("WORLD PARAMETERS").small().weak());
        if ui.small_button("reset").clicked() {
            *p = Params::default();
        }
    });

    egui::CollapsingHeader::new("Food / trees")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.tree_interval, 1..=2000).text("drop interval (ticks)"));
            ui.add(egui::Slider::new(&mut p.tree_per_cycle, 0..=40).text("pellets / drop"));
            ui.add(egui::Slider::new(&mut p.tree_radius, 1..=40).text("spread radius"));
            ui.add(egui::Slider::new(&mut p.pellet_energy, 1..=100).text("pellet energy"));
            ui.add(
                egui::Slider::new(&mut p.max_pellet_fraction, 0.0..=0.6).text("max food density"),
            );
            let per_tree = p.tree_per_cycle as f32 / p.tree_interval.max(1) as f32;
            ui.label(
                egui::RichText::new(format!(
                    "≈ {:.3} food/tick per tree · {:.0} food/s per tree at {:.0} tps",
                    per_tree,
                    per_tree * tps,
                    tps
                ))
                .small()
                .weak(),
            );
        });

    egui::CollapsingHeader::new("Hunger / health")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.starve_ticks, 60..=5000).text("ticks to starve"));
            ui.add(egui::Slider::new(&mut p.starve_damage, 0.0..=2.0).text("starve dmg/tick"));
            ui.add(egui::Slider::new(&mut p.heal_rate, 0.0..=1.0).text("heal/tick"));
            ui.add(egui::Slider::new(&mut p.base_health, 1.0..=100.0).text("villager health"));
            ui.add(egui::Slider::new(&mut p.leader_health, 1.0..=200.0).text("leader health"));
            ui.add(egui::Slider::new(&mut p.hunger_min, 0.0..=1.0).text("hunger trigger min"));
            ui.add(egui::Slider::new(&mut p.hunger_max, 0.0..=1.0).text("hunger trigger max"));
        });

    egui::CollapsingHeader::new("Movement / perception")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.min_speed, 0.01..=2.0).text("min speed"));
            ui.add(egui::Slider::new(&mut p.max_speed, 0.01..=2.0).text("max speed"));
            ui.add(egui::Slider::new(&mut p.vision_radius, 1..=60).text("vision radius"));
            ui.add(egui::Slider::new(&mut p.leader_chance, 0.0..=0.5).text("leader chance"));
        });

    egui::CollapsingHeader::new("Clans / combat")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.carry_limit, 1..=50).text("carry limit"));
            ui.add(egui::Slider::new(&mut p.attack_damage, 0.0..=5.0).text("attack damage"));
            ui.add(egui::Slider::new(&mut p.attack_cooldown, 1..=200).text("attack cooldown"));
            ui.add(
                egui::Slider::new(&mut p.clan_grace_ticks, 0..=5000).text("peace grace (ticks)"),
            );
            ui.add(egui::Slider::new(&mut p.war_threshold, 0.0..=2.0).text("war threshold"));
            ui.add(egui::Slider::new(&mut p.recruit_radius, 1..=10).text("recruit radius"));
            ui.label(
                egui::RichText::new("war when two clans' combined aggression ≥ threshold")
                    .small()
                    .weak(),
            );
        });

    egui::CollapsingHeader::new("Growth / expansion")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.birth_chance, 0.0..=1.0).text("birth chance / pair"));
            ui.add(
                egui::Slider::new(&mut p.birth_interval, 10..=2000).text("birth interval (ticks)"),
            );
            ui.add(egui::Slider::new(&mut p.birth_food_cost, 0..=20).text("birth food cost"));
            ui.add(
                egui::Slider::new(&mut p.claim_interval, 1..=400).text("claim interval (ticks)"),
            );
            ui.add(
                egui::Slider::new(&mut p.members_per_claim, 1..=20).text("members / claimed tile"),
            );
            ui.label(
                egui::RichText::new("each pair of NPCs may birth one child per check, if fed")
                    .small()
                    .weak(),
            );
        });

    egui::CollapsingHeader::new("Farming / seasons")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(egui::Slider::new(&mut p.farm_yield, 0.0..=0.6).text("farm yield (owned land)"));
            ui.add(egui::Slider::new(&mut p.farm_interval, 1..=120).text("farm interval (ticks)"));
            ui.add(egui::Slider::new(&mut p.home_range, 4..=80).text("home range (work radius)"));
            ui.add(
                egui::Slider::new(&mut p.expand_claim_radius, 1..=4).text("claim radius (expand)"),
            );
            ui.add(
                egui::Slider::new(&mut p.season_length, 0..=10000).text("season length (ticks)"),
            );
            ui.add(egui::Slider::new(&mut p.season_amp, 0.0..=0.95).text("season amplitude"));
            ui.add(
                egui::Slider::new(&mut p.soil_depletion_rate, 0.0..=1.0)
                    .text("soil depletion (0 = off)"),
            );
            ui.add(
                egui::Slider::new(&mut p.disaster_rate, 0.0..=1.0)
                    .text("disasters / blights (0 = off)"),
            );
            ui.label(
                egui::RichText::new(
                    "owned, fertile land grows food; lean seasons cut yields and spark wars",
                )
                .small()
                .weak(),
            );
        });

    egui::CollapsingHeader::new("Community logistics")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(
                &mut p.community_logistics,
                "enable Community Logistics V1",
            );
            ui.label(
                egui::RichText::new(if p.community_logistics {
                    "wood hauling, reserves, road construction, and road movement savings are active"
                } else {
                    "causal ablation: reserve/wood/road mechanics are off and existing roads give no movement benefit; simultaneous roles remain active"
                })
                .small()
                .weak(),
            );
        });

    egui::CollapsingHeader::new("Community care")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut p.community_care, "enable Community Care V1");
            ui.label(
                egui::RichText::new(if p.community_care {
                    "nearby gatherers and defenders evacuate incapacitated clanmates before bleed-out"
                } else {
                    "causal ablation: lethal combat causes immediate death and no rescue response"
                })
                .small()
                .weak(),
            );
        });

    egui::CollapsingHeader::new("Terrain (Populate to apply)")
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(&mut p.terrain_on, "generate terrain");
            ui.add(egui::Slider::new(&mut p.water_level, 0.0..=0.6).text("water level"));
            ui.add(egui::Slider::new(&mut p.mountain_level, 0.5..=0.98).text("mountain level"));
            ui.label(
                egui::RichText::new("regenerates on Populate fresh / preset")
                    .small()
                    .weak(),
            );
        });
}

fn terrain_color(t: u8) -> egui::Color32 {
    use crate::grid::terrain::*;
    match t {
        WATER => egui::Color32::from_rgb(34, 64, 104),
        SAND => egui::Color32::from_rgb(122, 112, 80),
        FOREST => egui::Color32::from_rgb(28, 56, 36),
        HILL => egui::Color32::from_rgb(80, 72, 54),
        MOUNTAIN => egui::Color32::from_rgb(94, 96, 102),
        _ => egui::Color32::from_rgb(40, 54, 42), // plains
    }
}

fn blend(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let mix = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn lighten(c: egui::Color32, t: f32) -> egui::Color32 {
    blend(c, egui::Color32::WHITE, t)
}

fn legend_row(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, color);
        ui.label(label);
    });
}
