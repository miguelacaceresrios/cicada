//! Voces: cada evento se convierte en una voz que genera muestras hasta
//! apagarse. Todo es DSP propio en `f32`, sin depender del backend.

use std::f32::consts::TAU;
use std::sync::Arc;

use cicada_core::Wave;

#[derive(Clone)]
pub enum Kind {
    /// Seno con barrido de pitch: la base del kick gabber. Con `dist` se
    /// convierte en el característico bombo saturado.
    Kick {
        phase: f32,
    },
    Snare {
        phase: f32,
        lp: f32,
    },
    Hat {
        lp: f32,
    },
    Synth {
        wave: Wave,
        freq: f32,
        phase: f32,
        gate: u32,
    },
    Sample {
        data: Arc<[f32]>,
    },
}

#[derive(Clone)]
pub struct Voice {
    kind: Kind,
    t: u32,
    sr: f32,
    amp: f32,
    drive: f32,
    noise: u64,
    done: bool,
}

impl Voice {
    pub fn new(kind: Kind, amp: f32, dist: f32, sr: f32, noise_seed: u64) -> Self {
        Voice {
            kind,
            t: 0,
            sr,
            amp,
            drive: 1.0 + dist * 24.0,
            noise: noise_seed | 1,
            done: false,
        }
    }

    pub fn synth(wave: Wave, freq: f32, seconds: f32, sr: f32) -> Kind {
        Kind::Synth {
            wave,
            freq,
            phase: 0.0,
            gate: (seconds * sr) as u32,
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Ruido blanco determinista (xorshift), sembrado por evento.
    fn white(&mut self) -> f32 {
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 7;
        self.noise ^= self.noise << 17;
        (self.noise >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }

    pub fn next(&mut self) -> f32 {
        if self.done {
            return 0.0;
        }
        let sr = self.sr;
        let t = self.t as f32 / sr;
        self.t += 1;
        let n = self.white();

        let (raw, alive) = match &mut self.kind {
            Kind::Kick { phase } => {
                let freq = 48.0 + 380.0 * (-t / 0.028).exp();
                *phase = (*phase + freq / sr).fract();
                ((*phase * TAU).sin() * (-t / 0.22).exp(), t < 0.6)
            }
            Kind::Snare { phase, lp } => {
                *phase = (*phase + 190.0 / sr).fract();
                let tone = (*phase * TAU).sin() * (-t / 0.04).exp() * 0.6;
                let (lp_new, hp) = high_pass(*lp, n, 1500.0, sr);
                *lp = lp_new;
                (tone + hp * (-t / 0.07).exp() * 0.8, t < 0.35)
            }
            Kind::Hat { lp } => {
                let (lp_new, hp) = high_pass(*lp, n, 7000.0, sr);
                *lp = lp_new;
                (hp * (-t / 0.018).exp(), t < 0.12)
            }
            Kind::Synth {
                wave,
                freq,
                phase,
                gate,
            } => {
                let dt = *freq / sr;
                *phase = (*phase + dt).fract();
                let osc = match wave {
                    Wave::Sine => (*phase * TAU).sin(),
                    Wave::Saw => 2.0 * *phase - 1.0 - poly_blep(*phase, dt),
                    Wave::Square => {
                        let naive = if *phase < 0.5 { 1.0 } else { -1.0 };
                        naive + poly_blep(*phase, dt) - poly_blep((*phase + 0.5).fract(), dt)
                    }
                };
                let gate_s = *gate as f32 / sr;
                let attack = (t / 0.004).min(1.0);
                let release = if t < gate_s {
                    1.0
                } else {
                    (-(t - gate_s) / 0.03).exp()
                };
                (osc * attack * release * 0.4, t < gate_s + 0.25)
            }
            Kind::Sample { data } => {
                let i = self.t as usize - 1;
                (data.get(i).copied().unwrap_or(0.0), i + 1 < data.len())
            }
        };

        self.done = !alive;
        let shaped = if self.drive > 1.0 {
            (raw * self.drive).tanh() / self.drive.tanh()
        } else {
            raw
        };
        shaped * self.amp
    }
}

/// Filtro paso alto de un polo: la señal menos su versión suavizada.
fn high_pass(lp: f32, x: f32, cutoff: f32, sr: f32) -> (f32, f32) {
    let a = 1.0 - (-TAU * cutoff / sr).exp();
    let lp = lp + a * (x - lp);
    (lp, x - lp)
}

/// Corrección PolyBLEP para osciladores sin aliasing audible.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}
