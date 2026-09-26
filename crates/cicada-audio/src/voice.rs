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
    pan: f32,
    drive: f32,
    /// Paso alto después de la distorsión. Saturar un grave genera mucha
    /// energía por debajo de la fundamental: quitarla es lo que convierte un
    /// zumbido en el golpe seco del kick gabber.
    post_hp: Option<(f32, f32)>,
    noise: u64,
    done: bool,
}

impl Voice {
    pub fn new(kind: Kind, amp: f32, pan: f32, dist: f32, sr: f32, noise_seed: u64) -> Self {
        let post_hp = matches!(kind, Kind::Kick { .. }).then_some((36.0, 0.0));
        let amp = amp * base_level(&kind);
        Voice {
            kind,
            t: 0,
            sr,
            amp,
            pan,
            // La curva no es lineal: los primeros pasos de `dist` ya se notan.
            drive: 1.0 + dist.powf(0.7) * 28.0,
            post_hp,
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

    pub fn pan(&self) -> f32 {
        self.pan
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
                // Dos barridos superpuestos: uno muy rápido que da el golpe
                // seco del ataque y otro lento que lleva el tono al grave.
                let freq = 44.0 + 80.0 * (-t / 0.085).exp() + 620.0 * (-t / 0.011).exp();
                *phase = (*phase + freq / sr).fract();
                let body = (*phase * TAU).sin();
                // Chasquido de los primeros milisegundos: es lo que hace que
                // el kick se oiga por encima de la mezcla.
                let click = n * (-t / 0.0018).exp() * 0.55;
                // Ataque de 1 ms para que el primer ciclo no suene a recorte.
                let env = (-t / 0.24).exp() * (1.0 - (-t / 0.001).exp());
                ((body + click) * env, t < 0.7)
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
        let mut out = if self.drive > 1.0 {
            (raw * self.drive).tanh() / self.drive.tanh()
        } else {
            raw
        };
        if let Some((cutoff, lp)) = &mut self.post_hp {
            let (lp_new, hp) = high_pass(*lp, out, *cutoff, sr);
            *lp = lp_new;
            out = hp;
        }
        out * self.amp
    }
}

/// Nivel de cada sonido para que `gain 1` signifique lo mismo en todos.
///
/// Sin esto, cada voz sale al nivel que le toque por su síntesis: el snare
/// pegaba más fuerte que el kick y los synths quedaban enterrados, así que
/// `gain` no servía para comparar pistas. Los valores dejan el kick como
/// elemento más fuerte, que es lo que pide el género.
fn base_level(kind: &Kind) -> f32 {
    match kind {
        Kind::Kick { .. } => 1.42,
        Kind::Snare { .. } => 0.79,
        Kind::Hat { .. } => 1.13,
        Kind::Synth { .. } => 1.96,
        // Un sample suena como está grabado: normalizarlo sería falsearlo.
        Kind::Sample { .. } => 1.0,
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
