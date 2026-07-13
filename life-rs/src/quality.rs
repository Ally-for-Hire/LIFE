//! Survival-gated quality metrics and behavioral niches for leader evolution.
//!
//! Survival is an eligibility requirement, not one weighted term among many.
//! Eligible brains are preserved across distinct strategy niches so one high-
//! scoring monoculture cannot erase useful builders, defenders, or raiders.

use crate::brain::{Brain, N_EXPERTS, N_IN, N_MODES};
use crate::clan::ClanMode;
use crate::world::World;

pub const SURVIVAL_FLOOR: f32 = 0.80;
pub const SECURITY_FLOOR: f32 = 0.50;
pub const FAIRNESS_FLOOR: f32 = -0.05;
pub const N_STRATEGY_NICHES: usize = 5;

const MIN_SPECIALIZATION_UTILIZATION_BALANCE: f32 = 0.55;
const MIN_SPECIALIZATION_DECISIVENESS: f32 = 0.50;
const MIN_SPECIALIZATION_MUTUAL_INFORMATION: f32 = 0.25;
const MIN_SPECIALIZATION_TOP1_COVERAGE: f32 = 0.75;
const MIN_SPECIALIZATION_OUTPUT_DIVERGENCE: f32 = 0.05;

/// Deterministic evidence that an MoE delegates distinct contexts to distinct,
/// behaviorally different experts. No single field is sufficient: uniform
/// mixing, one-expert collapse, and duplicated experts each fail a different
/// part of this contract.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContextualSpecializationMetrics {
    /// Entropy of mean soft expert utilization, normalized to [0,1].
    pub utilization_balance: f32,
    /// One minus mean per-context routing entropy, normalized to [0,1].
    pub decisiveness: f32,
    /// I(context; expert), normalized by log(expert count).
    pub contextual_mutual_information: f32,
    /// Fraction of experts selected top-1 in at least one probe context.
    pub contextual_top1_coverage: f32,
    /// Mean pairwise L1 distance between contextually selected experts' raw outputs.
    pub expert_output_divergence: f32,
}

impl ContextualSpecializationMetrics {
    /// Smooth tie-shaping score. A geometric mean makes every required aspect
    /// matter and returns zero for both uniform mixing and hard single-expert
    /// collapse. Survival eligibility remains outside this metric.
    pub fn specialization_score(self) -> f32 {
        (self.utilization_balance
            * self.decisiveness
            * self.contextual_mutual_information
            * self.contextual_top1_coverage
            * self.expert_output_divergence)
            .clamp(0.0, 1.0)
            .powf(0.2)
    }

    pub fn qualifies(self) -> bool {
        self.utilization_balance >= MIN_SPECIALIZATION_UTILIZATION_BALANCE
            && self.decisiveness >= MIN_SPECIALIZATION_DECISIVENESS
            && self.contextual_mutual_information >= MIN_SPECIALIZATION_MUTUAL_INFORMATION
            && self.contextual_top1_coverage >= MIN_SPECIALIZATION_TOP1_COVERAGE
            && self.expert_output_divergence >= MIN_SPECIALIZATION_OUTPUT_DIVERGENCE
    }

    /// Continuous distance to the same five-part qualification contract. This is
    /// training guidance only: zero means every exact threshold passes, while a
    /// finite positive value lets a promotion proxy distinguish near-passes from
    /// collapsed routers without weakening the boolean release gate.
    #[cfg(test)]
    pub(crate) fn qualification_deficit(self) -> f32 {
        fn shortfall(value: f32, floor: f32) -> f32 {
            if !value.is_finite() {
                return 1000.0;
            }
            ((floor - value).max(0.0) / floor.max(0.01)).min(1000.0)
        }

        (shortfall(
            self.utilization_balance,
            MIN_SPECIALIZATION_UTILIZATION_BALANCE,
        ) + shortfall(self.decisiveness, MIN_SPECIALIZATION_DECISIVENESS)
            + shortfall(
                self.contextual_mutual_information,
                MIN_SPECIALIZATION_MUTUAL_INFORMATION,
            )
            + shortfall(
                self.contextual_top1_coverage,
                MIN_SPECIALIZATION_TOP1_COVERAGE,
            )
            + shortfall(
                self.expert_output_divergence,
                MIN_SPECIALIZATION_OUTPUT_DIVERGENCE,
            ))
        .min(5000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategyNiche {
    Survivor,
    Builder,
    Cooperator,
    Defender,
    Raider,
}

impl StrategyNiche {
    pub const ALL: [StrategyNiche; N_STRATEGY_NICHES] = [
        StrategyNiche::Survivor,
        StrategyNiche::Builder,
        StrategyNiche::Cooperator,
        StrategyNiche::Defender,
        StrategyNiche::Raider,
    ];

    pub fn index(self) -> usize {
        match self {
            StrategyNiche::Survivor => 0,
            StrategyNiche::Builder => 1,
            StrategyNiche::Cooperator => 2,
            StrategyNiche::Defender => 3,
            StrategyNiche::Raider => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StrategyNiche::Survivor => "survivor",
            StrategyNiche::Builder => "builder",
            StrategyNiche::Cooperator => "cooperator",
            StrategyNiche::Defender => "defender",
            StrategyNiche::Raider => "raider",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QualityScore {
    pub fitness: f32,
    pub survival: f32,
    pub robust_survival: f32,
    pub security: f32,
    pub fairness: f32,
    pub robust_fairness: f32,
    pub settlement: f32,
    pub expansion: f32,
    pub cooperation: f32,
    /// Composite of useful hauling and causally observed road value.
    pub logistics: f32,
    /// Food and wood delivered to the stockpile per member-time.
    pub hauling_throughput: f32,
    /// Road use and movement-cost savings, penalized when construction outruns use.
    pub road_utility: f32,
    /// Emergency food held or deliberately cycled through the protected reserve.
    pub reserve_security: f32,
    /// Breadth and balance of sticky community jobs, including the safety core.
    pub task_coverage: f32,
    /// Successful recovery of incapacitated clanmates, normalized by member-time.
    pub care: f32,
    /// Physically delivered inter-clan food/wood per member-time.
    pub trade: f32,
    /// Completed development plus causally used shelter/storage/market/wall benefits.
    pub infrastructure: f32,
    /// Normalized settlement technology level.
    pub technology: f32,
    /// Physically owned equipment readiness plus observed combat value.
    pub military: f32,
    pub defense: f32,
    pub combat: f32,
    /// Legacy field name: contextual MoE specialization score.
    pub routing_entropy: f32,
    /// Legacy field name: contextual top-1 expert coverage.
    pub expert_coverage: f32,
    pub eligible: bool,
}

impl Default for QualityScore {
    fn default() -> Self {
        QualityScore {
            fitness: 0.0,
            survival: 0.0,
            robust_survival: 0.0,
            security: 0.0,
            fairness: 0.0,
            robust_fairness: 0.0,
            settlement: 0.0,
            expansion: 0.0,
            cooperation: 0.0,
            logistics: 0.0,
            hauling_throughput: 0.0,
            road_utility: 0.0,
            reserve_security: 0.0,
            task_coverage: 0.0,
            care: 0.0,
            trade: 0.0,
            infrastructure: 0.0,
            technology: 0.0,
            military: 0.0,
            defense: 0.0,
            combat: 0.0,
            routing_entropy: 0.0,
            expert_coverage: 0.0,
            eligible: false,
        }
    }
}

impl QualityScore {
    pub fn niche_scores(self) -> [f32; N_STRATEGY_NICHES] {
        [
            self.survival * 0.45
                + self.security * 0.35
                + ((self.fairness + 1.0) * 0.5).clamp(0.0, 1.0) * 0.20,
            self.settlement * 0.45 + self.expansion * 0.35 + self.security * 0.20,
            self.cooperation * 0.30
                + self.logistics * 0.18
                + self.reserve_security * 0.12
                + self.task_coverage * 0.10
                + self.care * 0.10
                + self.trade * 0.15
                + self.survival * 0.03
                + self.security * 0.02,
            self.defense * 0.45
                + self.military * 0.10
                + self.settlement * 0.20
                + self.survival * 0.25,
            self.combat * 0.50
                + self.military * 0.10
                + self.expansion * 0.15
                + self.survival * 0.25,
        ]
    }

    pub fn niche_quality(self, niche: StrategyNiche) -> f32 {
        self.niche_scores()[niche.index()]
    }

    pub fn qualifies_for(self, niche: StrategyNiche) -> bool {
        if !self.eligible {
            return false;
        }
        match niche {
            StrategyNiche::Survivor => true,
            StrategyNiche::Builder => self.settlement >= 0.35 && self.expansion >= 0.10,
            StrategyNiche::Cooperator => {
                self.cooperation >= 0.08
                    || (self.logistics >= 0.08
                        && self.reserve_security >= 0.05
                        && self.task_coverage >= 0.25)
            }
            StrategyNiche::Defender => self.defense >= 0.55,
            StrategyNiche::Raider => self.combat >= 0.03,
        }
    }

    /// Survival dominates selection. Routing balance is only a small tie-shaper
    /// among already viable strategies, never permission to sacrifice a clan.
    pub fn selection_score(self) -> f32 {
        if !self.eligible {
            return self.robust_survival * 400.0
                + self.survival * 200.0
                + self.security * 100.0
                + ((self.robust_fairness + 1.0) * 0.5).clamp(0.0, 1.0) * 100.0
                + self.fitness * 0.02;
        }
        // `routing_entropy` is retained as a compatibility field name, but now
        // already contains the full contextual-specialization composite. Do not
        // multiply coverage into it again: coverage is one of its five factors.
        1_000_000.0 + self.fitness * (0.92 + self.routing_entropy * 0.08)
    }
}

/// Score one clan's outcome. A missing/disbanded clan is an explicit survival
/// failure, which prevents a flashy but extinct strategy from entering an archive.
pub fn score_clan(w: &World, cid: i32) -> QualityScore {
    let Some(c) = w.clan_by_id(cid) else {
        return QualityScore::default();
    };

    let pop = w.clan_population(cid) as f32;
    let food = (c.food.max(0) + c.reserve_food.max(0)) as f32;
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
    let starving_fraction = if c.stats.pop_tick_sum > 0 {
        c.stats.starving_ticks as f32 / c.stats.pop_tick_sum as f32
    } else {
        1.0
    };
    let settled = if c.stats.pop_tick_sum > 0 {
        c.stats.on_terr_tick_sum as f32 / c.stats.pop_tick_sum as f32
    } else {
        0.0
    };
    let reserve_per_cap = food / pop.max(1.0);
    let member_ticks = c.stats.pop_tick_sum.max(1) as f32;
    let rate_per_1k = |events: u32| events as f32 * 1000.0 / member_ticks;
    let delivered = c
        .stats
        .food_delivered
        .saturating_add(c.stats.wood_delivered);
    let hauling_rate = rate_per_1k(delivered);
    let hauling_throughput = saturating_rate(hauling_rate, 6.0);

    // Construction has no value by itself. Credit roads only when members use
    // them and the movement ledger records an actual cost saving. The final
    // usefulness factor makes speculative road spam dilute, rather than raise,
    // the score until traffic repays the construction footprint.
    let road_utility = score_road_utility(
        c.stats.road_steps,
        c.stats.road_cost_saved_milli,
        c.stats.roads_built,
        c.stats.pop_tick_sum.max(1),
    );
    let logistics = (hauling_throughput * 0.60 + road_utility * 0.40).clamp(0.0, 1.0);
    let reserve_flow_rate = rate_per_1k(
        c.stats
            .reserve_deposited
            .saturating_add(c.stats.reserve_released),
    );
    let protected_food = c.reserve_food.max(0) as f32 / pop.max(1.0);
    let reserve_coverage = (protected_food / 3.0).clamp(0.0, 1.0);
    let reserve_flow = reserve_flow_rate / (reserve_flow_rate + 3.0);
    let reserve_security = (reserve_coverage * 0.70 + reserve_flow * 0.30).clamp(0.0, 1.0);
    let care = if c.stats.incapacitations == 0 {
        0.0
    } else {
        c.stats.rescues as f32 / c.stats.incapacitations as f32
    };
    let trade_events = c
        .stats
        .trade_food_sent
        .saturating_add(c.stats.trade_food_received)
        .saturating_add(c.stats.trade_wood_sent)
        .saturating_add(c.stats.trade_wood_received);
    let trade = saturating_rate(rate_per_1k(trade_events), 2.0);
    let settlement_state = if w.community_settlement {
        w.settlements
            .iter()
            .find(|state| state.clan_id == cid)
            .copied()
            .unwrap_or_default()
    } else {
        crate::settlement::ClanSettlement::default()
    };
    let development = if w.community_settlement {
        crate::settlement::development_score(&w.buildings, cid) as f32
    } else {
        0.0
    };
    let useful_events = settlement_state.stats.granary_food_stored as f32
        + settlement_state.stats.market_material_delivered as f32
        + settlement_state.stats.shelter_healing_milli as f32 / 1000.0
        + settlement_state.stats.wall_damage_prevented_milli as f32 / 1000.0;
    let development_value = saturating_rate(development * 1000.0 / member_ticks, 0.5);
    let useful_value = saturating_rate(useful_events * 1000.0 / member_ticks, 1.0);
    let infrastructure = (development_value * 0.4 + useful_value * 0.6).clamp(0.0, 1.0);
    let technology =
        settlement_state.tech.level as f32 / crate::settlement::MAX_TECH_LEVEL.max(1) as f32;
    let military_state = if w.community_military {
        w.militaries
            .iter()
            .find(|state| state.clan_id == cid)
            .copied()
            .unwrap_or_default()
    } else {
        crate::military::ClanMilitary::default()
    };
    let equipped = if w.community_military {
        w.equipment
            .iter()
            .filter(|loadout| {
                (loadout.weapon.is_some() || loadout.armor.is_some())
                    && w.entity_by_id(loadout.entity_id)
                        .is_some_and(|entity| entity.clan == cid && entity.is_active())
            })
            .count() as f32
    } else {
        0.0
    };
    let readiness = (equipped / pop.max(1.0)).clamp(0.0, 1.0);
    let causal_combat_milli = military_state
        .stats
        .bonus_damage_milli
        .saturating_add(military_state.stats.damage_prevented_milli)
        as f32;
    let causal_use = saturating_rate(causal_combat_milli / member_ticks, 25.0);
    let military = (readiness * 0.8 + causal_use * 0.2).clamp(0.0, 1.0);

    let role_total = c.stats.role_tick_sum.iter().sum::<u64>() as f32;
    let task_coverage = if role_total > 0.0 {
        let mut entropy = 0.0;
        let mut meaningful = 0usize;
        for &ticks in &c.stats.role_tick_sum {
            let share = ticks as f32 / role_total;
            if share >= 0.03 {
                meaningful += 1;
            }
            if share > 0.0 {
                entropy -= share * share.ln();
            }
        }
        let entropy = (entropy / (N_MODES as f32).ln().max(1e-6)).clamp(0.0, 1.0);
        let breadth = meaningful as f32 / N_MODES as f32;
        let gather_present = c.stats.role_tick_sum[ClanMode::Gather.index()] > 0;
        let defend_present = c.stats.role_tick_sum[ClanMode::Defend.index()] > 0;
        let safety_core = (gather_present as u8 as f32 + defend_present as u8 as f32) * 0.5;
        (breadth * 0.45 + entropy * 0.35 + safety_core * 0.20).clamp(0.0, 1.0)
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
    let settled_score = settled * avg_pop * 8.0;
    let land_score = c.fertile_capacity.sqrt() * 8.0 + terr.sqrt() * 2.0;
    let reserve_score = reserve_per_cap.min(12.0) * 5.0 + avg_food.sqrt() * 2.0;
    let cooperation_score = recruits * 5.0 + infrastructure * 20.0 + technology * 10.0;
    let combat_score = kills * 1.2;
    let hunger_penalty = avg_hunger * 40.0 + c.stats.starving_ticks as f32 * 0.06;
    let loss_penalty = losses * 18.0;
    let fitness = ((survival_score
        + population_score
        + settled_score
        + land_score
        + reserve_score
        + cooperation_score
        + combat_score)
        * group_multiplier
        - hunger_penalty
        - loss_penalty)
        .max(0.0);

    let security = ((1.0 - avg_hunger.clamp(0.0, 1.0)) * (1.0 - starving_fraction.clamp(0.0, 1.0)))
        .clamp(0.0, 1.0);
    let expansion = (c.fertile_capacity.sqrt() / 14.0 + terr.sqrt() / 160.0).clamp(0.0, 1.0);
    let recruitment = (recruits / (recruits + 8.0)).clamp(0.0, 1.0);
    let cooperation = (recruitment * 0.12
        + logistics * 0.18
        + reserve_security * 0.14
        + task_coverage * 0.14
        + care * 0.14
        + trade * 0.14
        + infrastructure * 0.14)
        .clamp(0.0, 1.0);
    let defense = (survival * (1.0 - losses / (losses + pop + 1.0))).clamp(0.0, 1.0);
    let combat = if kills <= 0.0 {
        0.0
    } else {
        (kills / (kills + 8.0) * (kills + 1.0) / (kills + losses + 1.0)).clamp(0.0, 1.0)
    };

    QualityScore {
        fitness,
        survival,
        robust_survival: survival,
        security,
        fairness: 0.0,
        robust_fairness: 0.0,
        settlement: settled.clamp(0.0, 1.0),
        expansion,
        cooperation,
        logistics,
        hauling_throughput,
        road_utility,
        reserve_security,
        task_coverage,
        care,
        trade,
        infrastructure,
        technology,
        military,
        defense,
        combat,
        routing_entropy: 0.0,
        expert_coverage: 0.0,
        eligible: survival >= SURVIVAL_FLOOR && security >= SECURITY_FLOOR,
    }
}

fn saturating_rate(value: f32, half_saturation: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    value / (value + half_saturation.max(f32::EPSILON))
}

fn score_road_utility(
    road_steps: u64,
    road_savings_milli: u64,
    roads_built: u32,
    member_ticks: u64,
) -> f32 {
    let road_steps = road_steps as f32;
    let road_savings_milli = road_savings_milli as f32;
    let member_ticks = member_ticks.max(1) as f32;
    let road_use = saturating_rate(road_steps * 1000.0 / member_ticks, 30.0);
    let road_savings = saturating_rate(road_savings_milli / member_ticks, 30.0);
    let saving_per_step = if road_steps > 0.0 {
        (road_savings_milli / road_steps / 1_500.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let construction_debt = roads_built as f32 * 10_000.0;
    let usefulness = if road_savings_milli > 0.0 {
        road_savings_milli / (road_savings_milli + construction_debt)
    } else {
        0.0
    };
    ((road_use * 0.35 + road_savings * 0.45 + saving_per_step * 0.20) * usefulness).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_work_qualifies_a_viable_cooperator_without_recruitment() {
        let quality = QualityScore {
            survival: 1.0,
            robust_survival: 1.0,
            security: 0.8,
            logistics: 0.35,
            reserve_security: 0.30,
            task_coverage: 0.65,
            eligible: true,
            ..QualityScore::default()
        };
        assert!(quality.qualifies_for(StrategyNiche::Cooperator));
    }

    #[test]
    fn public_work_never_bypasses_the_survival_gate() {
        let quality = QualityScore {
            logistics: 1.0,
            reserve_security: 1.0,
            task_coverage: 1.0,
            cooperation: 1.0,
            eligible: false,
            ..QualityScore::default()
        };
        assert!(!quality.qualifies_for(StrategyNiche::Cooperator));
        assert!(quality.selection_score() < 1_000_000.0);
    }

    #[test]
    fn roads_score_only_observed_transport_value_and_penalize_spam() {
        assert_eq!(score_road_utility(0, 0, 100, 10_000), 0.0);
        let useful = score_road_utility(600, 300_000, 4, 10_000);
        let spammed = score_road_utility(600, 300_000, 40, 10_000);
        let more_value = score_road_utility(1200, 600_000, 4, 10_000);
        assert!(useful > spammed, "unused construction must dilute utility");
        assert!(
            more_value > useful,
            "more observed savings must raise utility"
        );
    }

    #[test]
    fn uniform_routing_is_balanced_but_not_contextually_specialized() {
        let gates = vec![[1.0 / N_EXPERTS as f32; N_EXPERTS]; 4];
        let outputs = divergent_expert_outputs(gates.len());
        let metrics = measure_contextual_specialization(&gates, &outputs);

        assert!(metrics.utilization_balance > 0.99);
        assert!(metrics.decisiveness < 0.01);
        assert!(metrics.contextual_mutual_information < 0.01);
        assert_eq!(metrics.contextual_top1_coverage, 0.25);
        assert!(!metrics.qualifies());
    }

    #[test]
    fn one_expert_collapse_is_decisive_but_not_contextually_specialized() {
        let gates = vec![[0.997, 0.001, 0.001, 0.001]; 4];
        let outputs = divergent_expert_outputs(gates.len());
        let metrics = measure_contextual_specialization(&gates, &outputs);

        assert!(metrics.decisiveness > 0.95);
        assert!(metrics.utilization_balance < 0.05);
        assert!(metrics.contextual_mutual_information < 0.01);
        assert_eq!(metrics.contextual_top1_coverage, 0.25);
        assert!(!metrics.qualifies());
    }

    #[test]
    fn decisive_three_expert_contextual_delegation_qualifies() {
        let mut gates = vec![[0.01; N_EXPERTS]; 4];
        for (gate, top1) in gates.iter_mut().zip([0, 1, 2, 0]) {
            gate[top1] = 0.97;
        }
        let outputs = divergent_expert_outputs(gates.len());
        let metrics = measure_contextual_specialization(&gates, &outputs);

        assert!(metrics.utilization_balance >= MIN_SPECIALIZATION_UTILIZATION_BALANCE);
        assert!(metrics.decisiveness >= MIN_SPECIALIZATION_DECISIVENESS);
        assert!(metrics.contextual_mutual_information >= MIN_SPECIALIZATION_MUTUAL_INFORMATION);
        assert_eq!(metrics.contextual_top1_coverage, 0.75);
        assert!(metrics.expert_output_divergence > 0.30);
        assert!(metrics.qualifies());
    }

    #[test]
    fn specialization_cannot_bypass_survival_eligibility() {
        let ineligible = QualityScore {
            fitness: 10_000.0,
            routing_entropy: 1.0,
            expert_coverage: 1.0,
            eligible: false,
            ..QualityScore::default()
        };
        let eligible = QualityScore {
            fitness: 1.0,
            routing_entropy: 0.1,
            expert_coverage: 0.75,
            eligible: true,
            ..QualityScore::default()
        };

        assert!(ineligible.selection_score() < 1_000_000.0);
        assert!(eligible.selection_score() >= 1_000_000.0);
    }

    fn divergent_expert_outputs(contexts: usize) -> Vec<[[f32; crate::brain::N_OUT]; N_EXPERTS]> {
        let outputs = std::array::from_fn(|expert| {
            [expert as f32 / (N_EXPERTS - 1) as f32; crate::brain::N_OUT]
        });
        vec![outputs; contexts]
    }
}

/// Compatibility projection for the existing trainer integration. The first
/// value is now the contextual-specialization composite, not routing entropy;
/// the second is contextual top-1 coverage, not average soft-share coverage.
pub fn routing_metrics(brain: &Brain) -> (f32, f32) {
    let metrics = contextual_specialization_metrics(brain);
    (
        metrics.specialization_score(),
        metrics.contextual_top1_coverage,
    )
}

/// Probe deterministic representative situations and distinguish genuine
/// contextual delegation from both uniform mixing and single-expert collapse.
pub fn contextual_specialization_metrics(brain: &Brain) -> ContextualSpecializationMetrics {
    let probes = routing_probes();
    let mut gates = Vec::with_capacity(probes.len());
    let mut outputs = Vec::with_capacity(probes.len());
    for probe in &probes {
        let (_, gate) = brain.evaluate(probe);
        gates.push(gate);
        let expert_outputs = brain.expert_outputs(probe);
        outputs.push(std::array::from_fn(|expert| expert_outputs[expert]));
    }
    measure_contextual_specialization(&gates, &outputs)
}

fn routing_probes() -> [[f32; N_IN]; 16] {
    let mut probes = [[0.0f32; N_IN]; 16];
    for probe in &mut probes {
        probe[0] = 0.30;
        probe[1] = 0.50;
        probe[31] = 1.0;
    }
    probes[0][4] = 0.8; // open land / growth
    probes[0][11] = 1.0;
    probes[1][1] = 0.05; // famine / winter
    probes[1][2] = 0.90;
    probes[1][15] = -1.0;
    probes[2][6] = 0.90; // enemy pressure
    probes[2][8] = 0.70;
    probes[2][12] = 1.0;
    probes[3][0] = 0.95; // crowded
    probes[3][3] = 0.95;
    probes[3][4] = 0.05;
    probes[4][7] = 0.90; // recruits nearby
    probes[4][4] = 0.60;
    probes[5][26] = 0.90; // depleted soil
    probes[6][27] = 1.0; // disaster
    probes[6][2] = 0.60;
    probes[7][13] = 1.0; // rich summer
    probes[7][15] = 1.0;
    probes[8][22] = 0.50; // neutral new trade relationship
    probes[8][23] = 0.25;
    probes[9][22] = 0.90; // trusted, high-volume trade network
    probes[9][23] = 1.0;
    probes[9][24] = 0.80;
    probes[10][6] = 0.70; // hostile relation threatening a trade route
    probes[10][13] = 1.0;
    probes[10][22] = 0.10;
    probes[10][23] = 0.50;
    probes[10][24] = 0.40;
    probes[11][17] = 0.90; // developed settlement with room to specialize
    probes[11][4] = 0.55;
    probes[12][18] = 0.90; // technologically mature settlement under pressure
    probes[12][6] = 0.55;
    probes[12][8] = 0.45;
    probes[13][17] = 0.60; // food-secure workshop with ore opportunity
    probes[13][30] = 0.90;
    probes[14][6] = 0.80; // under-armed defense pressure
    probes[14][13] = 1.0;
    probes[14][19] = 0.05;
    probes[15][6] = 0.80; // armed defense pressure
    probes[15][13] = 1.0;
    probes[15][19] = 0.90;

    probes
}

fn measure_contextual_specialization(
    gates: &[[f32; N_EXPERTS]],
    expert_outputs: &[[[f32; crate::brain::N_OUT]; N_EXPERTS]],
) -> ContextualSpecializationMetrics {
    if gates.is_empty() || gates.len() != expert_outputs.len() {
        return ContextualSpecializationMetrics::default();
    }

    let mut mean_gate = [0.0f32; N_EXPERTS];
    let mut conditional_entropy = 0.0;
    let mut top1_used = [false; N_EXPERTS];
    let log_n = (N_EXPERTS as f32).ln().max(1e-6);
    let context_weight = 1.0 / gates.len() as f32;
    for gate in gates {
        let sum = gate
            .iter()
            .copied()
            .filter(|value| value.is_finite() && *value > 0.0)
            .sum::<f32>()
            .max(f32::EPSILON);
        let mut top1 = 0usize;
        let mut top1_probability = f32::NEG_INFINITY;
        for expert in 0..N_EXPERTS {
            let probability = if gate[expert].is_finite() {
                gate[expert].max(0.0) / sum
            } else {
                0.0
            };
            mean_gate[expert] += probability * context_weight;
            if probability > 0.0 {
                conditional_entropy -= probability * probability.ln() / log_n * context_weight;
            }
            if probability > top1_probability {
                top1_probability = probability;
                top1 = expert;
            }
        }
        top1_used[top1] = true;
    }

    let utilization_balance = normalized_entropy(&mean_gate, log_n);
    let decisiveness = (1.0 - conditional_entropy).clamp(0.0, 1.0);
    let contextual_mutual_information = (utilization_balance - conditional_entropy).clamp(0.0, 1.0);
    let contextual_top1_coverage =
        top1_used.iter().filter(|&&used| used).count() as f32 / N_EXPERTS as f32;

    let mut divergence_sum = 0.0;
    let mut divergence_pairs = 0usize;
    for context_outputs in expert_outputs {
        for left in 0..N_EXPERTS {
            if !top1_used[left] {
                continue;
            }
            for right in (left + 1)..N_EXPERTS {
                if !top1_used[right] {
                    continue;
                }
                let distance = context_outputs[left]
                    .iter()
                    .zip(context_outputs[right].iter())
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f32>()
                    / crate::brain::N_OUT as f32;
                divergence_sum += distance;
                divergence_pairs += 1;
            }
        }
    }
    let expert_output_divergence = if divergence_pairs == 0 {
        0.0
    } else {
        (divergence_sum / divergence_pairs as f32).clamp(0.0, 1.0)
    };

    ContextualSpecializationMetrics {
        utilization_balance,
        decisiveness,
        contextual_mutual_information,
        contextual_top1_coverage,
        expert_output_divergence,
    }
}

fn normalized_entropy(distribution: &[f32; N_EXPERTS], log_n: f32) -> f32 {
    (-distribution
        .iter()
        .copied()
        .filter(|probability| *probability > 0.0)
        .map(|probability| probability * probability.ln())
        .sum::<f32>()
        / log_n)
        .clamp(0.0, 1.0)
}
