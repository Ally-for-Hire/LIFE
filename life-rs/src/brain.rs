//! LeaderBrain: a hierarchical **mixture-of-experts** policy.
//!
//! A clan leader is a *master controller* (the gate) plus several *sub-minds*
//! (experts). Each sub-mind is a small feed-forward net that proposes a full
//! action vector (mode utilities + an aggression dial). The master reads the
//! same situation and outputs a soft routing weight over the sub-minds; the
//! leader's decision is the gate-weighted blend of its sub-minds' proposals.
//!
//! Nothing about the sub-minds is hardcoded — there is no "famine net" or "war
//! net" baked in. Evolution is free to specialise the experts (one may become a
//! survival mind, another a war mind, a third a settler) *and* to learn when the
//! master should delegate to each. So the whole strategy — what to do and which
//! sub-mind to trust in a given situation — is discovered, not scripted. This is
//! the substrate for "master control AIs with sub-minds" that handle famine,
//! seasons, war, and growth.
//!
//! The outputs ARE the action utilities (no value thresholds gate them): the
//! clan picks the highest-utility *physically feasible* mode, and reads the
//! aggression output directly. Weights are flat `Vec<f32>` for cache-friendly
//! evaluation and trivial crossover/mutation.

use crate::rng::Rng;

// Inputs: 16 live situation features (0..15), then a block of RESERVED slots
// (16..30) for planned future systems, then a bias (31). Reserved slots read 0.0
// until the world wires them up — so adding roads/buildings/tech/trade/etc. later
// only changes the *world*, never the brain's dimensions, and a champion trained
// today keeps loading after those features ship. See `world.rs` for the layout.
pub const N_IN: usize = 32;
pub const N_HID: usize = 12;
/// Outputs: indices 0..N_MODES are clan-mode utilities (order matches
/// `ClanMode::from_index`); the last output is the aggression dial in [0,1].
pub const N_MODES: usize = 6;
pub const N_OUT: usize = N_MODES + 1;
/// Sub-minds the master routes between. More experts = more strategies the
/// leader can keep "on call" for different situations.
pub const N_EXPERTS: usize = 4;
pub const N_GATE_HID: usize = 10;

/// Output labels for the inspector (the 6 modes + the aggression dial).
pub const OUT_LABELS: [&str; N_OUT] = [
    "recruit",
    "expand",
    "gather",
    "attack",
    "defend",
    "scout",
    "aggression",
];
/// Sub-mind labels for the inspector. Purely cosmetic — the experts are free to
/// specialise however evolution finds best.
pub const SUBMIND_LABELS: [&str; N_EXPERTS] = ["mind α", "mind β", "mind γ", "mind δ"];

#[inline]
fn rand_vec(rng: &mut Rng, n: usize) -> Vec<f32> {
    (0..n).map(|_| rng.f32() * 2.0 - 1.0).collect()
}

#[inline]
fn mix(x: &[f32], y: &[f32], rng: &mut Rng) -> Vec<f32> {
    x.iter()
        .zip(y.iter())
        .map(|(&p, &q)| if rng.f32() < 0.5 { p } else { q })
        .collect()
}

/// One expert: N_IN → N_HID (tanh) → N_OUT (sigmoid).
#[derive(Clone)]
struct SubMind {
    w_ih: Vec<f32>,
    b_h: Vec<f32>,
    w_ho: Vec<f32>,
    b_o: Vec<f32>,
}

impl SubMind {
    fn random(rng: &mut Rng) -> Self {
        SubMind {
            w_ih: rand_vec(rng, N_HID * N_IN),
            b_h: rand_vec(rng, N_HID),
            w_ho: rand_vec(rng, N_OUT * N_HID),
            b_o: rand_vec(rng, N_OUT),
        }
    }
    fn forward(&self, inputs: &[f32; N_IN]) -> [f32; N_OUT] {
        let mut hidden = [0f32; N_HID];
        for h in 0..N_HID {
            let mut sum = self.b_h[h];
            let base = h * N_IN;
            for i in 0..N_IN {
                sum += self.w_ih[base + i] * inputs[i];
            }
            hidden[h] = sum.tanh();
        }
        let mut out = [0f32; N_OUT];
        for o in 0..N_OUT {
            let mut sum = self.b_o[o];
            let base = o * N_HID;
            for h in 0..N_HID {
                sum += self.w_ho[base + h] * hidden[h];
            }
            out[o] = 1.0 / (1.0 + (-sum).exp());
        }
        out
    }
    fn params_mut(&mut self) -> impl Iterator<Item = &mut f32> {
        self.w_ih
            .iter_mut()
            .chain(self.b_h.iter_mut())
            .chain(self.w_ho.iter_mut())
            .chain(self.b_o.iter_mut())
    }
    fn crossover(a: &SubMind, b: &SubMind, rng: &mut Rng) -> SubMind {
        SubMind {
            w_ih: mix(&a.w_ih, &b.w_ih, rng),
            b_h: mix(&a.b_h, &b.b_h, rng),
            w_ho: mix(&a.w_ho, &b.w_ho, rng),
            b_o: mix(&a.b_o, &b.b_o, rng),
        }
    }
}

/// The master controller: N_IN → N_GATE_HID (tanh) → N_EXPERTS, softmaxed into
/// routing weights over the sub-minds.
#[derive(Clone)]
struct Gate {
    w_ih: Vec<f32>,
    b_h: Vec<f32>,
    w_ho: Vec<f32>,
    b_o: Vec<f32>,
}

impl Gate {
    fn random(rng: &mut Rng) -> Self {
        Gate {
            w_ih: rand_vec(rng, N_GATE_HID * N_IN),
            b_h: rand_vec(rng, N_GATE_HID),
            w_ho: rand_vec(rng, N_EXPERTS * N_GATE_HID),
            b_o: rand_vec(rng, N_EXPERTS),
        }
    }
    fn weights(&self, inputs: &[f32; N_IN]) -> [f32; N_EXPERTS] {
        let mut hidden = [0f32; N_GATE_HID];
        for h in 0..N_GATE_HID {
            let mut sum = self.b_h[h];
            let base = h * N_IN;
            for i in 0..N_IN {
                sum += self.w_ih[base + i] * inputs[i];
            }
            hidden[h] = sum.tanh();
        }
        let mut logits = [0f32; N_EXPERTS];
        for o in 0..N_EXPERTS {
            let mut sum = self.b_o[o];
            let base = o * N_GATE_HID;
            for h in 0..N_GATE_HID {
                sum += self.w_ho[base + h] * hidden[h];
            }
            logits[o] = sum;
        }
        // softmax (numerically stable)
        let mut mx = f32::MIN;
        for &l in &logits {
            if l > mx {
                mx = l;
            }
        }
        let mut sum = 0.0;
        let mut w = [0f32; N_EXPERTS];
        for o in 0..N_EXPERTS {
            let e = (logits[o] - mx).exp();
            w[o] = e;
            sum += e;
        }
        let inv = 1.0 / sum.max(1e-9);
        for o in 0..N_EXPERTS {
            w[o] *= inv;
        }
        w
    }
    fn params_mut(&mut self) -> impl Iterator<Item = &mut f32> {
        self.w_ih
            .iter_mut()
            .chain(self.b_h.iter_mut())
            .chain(self.w_ho.iter_mut())
            .chain(self.b_o.iter_mut())
    }
    fn crossover(a: &Gate, b: &Gate, rng: &mut Rng) -> Gate {
        Gate {
            w_ih: mix(&a.w_ih, &b.w_ih, rng),
            b_h: mix(&a.b_h, &b.b_h, rng),
            w_ho: mix(&a.w_ho, &b.w_ho, rng),
            b_o: mix(&a.b_o, &b.b_o, rng),
        }
    }
}

#[derive(Clone)]
pub struct Brain {
    experts: Vec<SubMind>,
    gate: Gate,
    pub generation: u32,
    /// Last blended outputs and last gate routing — kept for the inspector.
    pub last_out: [f32; N_OUT],
    pub last_gate: [f32; N_EXPERTS],
}

impl Brain {
    pub fn random(rng: &mut Rng) -> Self {
        Brain {
            experts: (0..N_EXPERTS).map(|_| SubMind::random(rng)).collect(),
            gate: Gate::random(rng),
            generation: 0,
            last_out: [0.0; N_OUT],
            last_gate: [1.0 / N_EXPERTS as f32; N_EXPERTS],
        }
    }

    /// Evaluate the master + sub-minds: returns the gate-weighted action vector
    /// and the routing weights (which sub-mind the master delegated to).
    pub fn evaluate(&self, inputs: &[f32; N_IN]) -> ([f32; N_OUT], [f32; N_EXPERTS]) {
        let w = self.gate.weights(inputs);
        let mut out = [0f32; N_OUT];
        for (e, sm) in self.experts.iter().enumerate() {
            let oe = sm.forward(inputs);
            let we = w[e];
            for k in 0..N_OUT {
                out[k] += we * oe[k];
            }
        }
        (out, w)
    }

    /// Each sub-mind's raw proposed action vector for the given situation (for
    /// inspecting specialisation — what each expert "wants" before the gate blend).
    #[cfg(test)]
    pub fn expert_outputs(&self, inputs: &[f32; N_IN]) -> Vec<[f32; N_OUT]> {
        self.experts.iter().map(|sm| sm.forward(inputs)).collect()
    }

    pub fn mutate(&mut self, rng: &mut Rng, rate: f32, strength: f32) {
        for sm in self.experts.iter_mut() {
            for v in sm.params_mut() {
                if rng.f32() < rate {
                    *v = (*v + rng.gaussian() * strength).clamp(-4.0, 4.0);
                }
            }
        }
        for v in self.gate.params_mut() {
            if rng.f32() < rate {
                *v = (*v + rng.gaussian() * strength).clamp(-4.0, 4.0);
            }
        }
    }

    /// All weights flattened in a fixed order (experts then gate) — for saving.
    fn flat(&self) -> Vec<f32> {
        let mut v = Vec::new();
        for sm in &self.experts {
            v.extend_from_slice(&sm.w_ih);
            v.extend_from_slice(&sm.b_h);
            v.extend_from_slice(&sm.w_ho);
            v.extend_from_slice(&sm.b_o);
        }
        v.extend_from_slice(&self.gate.w_ih);
        v.extend_from_slice(&self.gate.b_h);
        v.extend_from_slice(&self.gate.w_ho);
        v.extend_from_slice(&self.gate.b_o);
        v
    }

    /// Save the brain to disk: a tiny header (magic, dims, generation) then all
    /// weights as little-endian f32. No external dependencies.
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"LFB1");
        for d in [N_IN, N_HID, N_OUT, N_EXPERTS, N_GATE_HID] {
            buf.extend_from_slice(&(d as u32).to_le_bytes());
        }
        buf.extend_from_slice(&self.generation.to_le_bytes());
        for f in self.flat() {
            buf.extend_from_slice(&f.to_le_bytes());
        }
        // Durable, atomic write: write a temp file, flush it to *physical disk*
        // (sync_all), then rename over the target. A reader never sees a partial
        // file, and a power loss can't leave a corrupt champion.
        use std::io::Write;
        let tmp = format!("{path}.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&buf)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// Load a brain saved by `save`. Errors if the file is missing, malformed, or
    /// was written for different network dimensions.
    pub fn load(path: &str) -> std::io::Result<Brain> {
        use std::io::{Error, ErrorKind};
        let bytes = std::fs::read(path)?;
        let bad = |m: &str| Error::new(ErrorKind::InvalidData, m.to_string());
        if bytes.len() < 4 + 5 * 4 + 4 || &bytes[0..4] != b"LFB1" {
            return Err(bad("not a LIFE brain file"));
        }
        let mut p = 4;
        let u32_at = |p: &mut usize| -> u32 {
            let v = u32::from_le_bytes([bytes[*p], bytes[*p + 1], bytes[*p + 2], bytes[*p + 3]]);
            *p += 4;
            v
        };
        let dims = [
            u32_at(&mut p),
            u32_at(&mut p),
            u32_at(&mut p),
            u32_at(&mut p),
            u32_at(&mut p),
        ];
        if dims
            != [
                N_IN as u32,
                N_HID as u32,
                N_OUT as u32,
                N_EXPERTS as u32,
                N_GATE_HID as u32,
            ]
        {
            return Err(bad("brain file dimensions don't match this build"));
        }
        let generation = u32_at(&mut p);
        let mut floats: Vec<f32> = Vec::new();
        while p + 4 <= bytes.len() {
            floats.push(f32::from_le_bytes([
                bytes[p],
                bytes[p + 1],
                bytes[p + 2],
                bytes[p + 3],
            ]));
            p += 4;
        }
        let mut cur = 0usize;
        let mut take = |n: usize| -> std::io::Result<Vec<f32>> {
            if cur + n > floats.len() {
                return Err(Error::new(ErrorKind::InvalidData, "brain file truncated"));
            }
            let s = floats[cur..cur + n].to_vec();
            cur += n;
            Ok(s)
        };
        let mut experts = Vec::with_capacity(N_EXPERTS);
        for _ in 0..N_EXPERTS {
            experts.push(SubMind {
                w_ih: take(N_HID * N_IN)?,
                b_h: take(N_HID)?,
                w_ho: take(N_OUT * N_HID)?,
                b_o: take(N_OUT)?,
            });
        }
        let gate = Gate {
            w_ih: take(N_GATE_HID * N_IN)?,
            b_h: take(N_GATE_HID)?,
            w_ho: take(N_EXPERTS * N_GATE_HID)?,
            b_o: take(N_EXPERTS)?,
        };
        Ok(Brain {
            experts,
            gate,
            generation,
            last_out: [0.0; N_OUT],
            last_gate: [1.0 / N_EXPERTS as f32; N_EXPERTS],
        })
    }

    pub fn crossover(a: &Brain, b: &Brain, rng: &mut Rng) -> Brain {
        let experts = (0..N_EXPERTS)
            .map(|e| SubMind::crossover(&a.experts[e], &b.experts[e], rng))
            .collect();
        Brain {
            experts,
            gate: Gate::crossover(&a.gate, &b.gate, rng),
            generation: a.generation.max(b.generation) + 1,
            last_out: [0.0; N_OUT],
            last_gate: [1.0 / N_EXPERTS as f32; N_EXPERTS],
        }
    }
}
