//! Voces: cada evento se convierte en una voz que genera muestras hasta
//! apagarse. Todo es DSP propio en `f32`, sin depender del backend.

use std::f32::consts::TAU;
use std::sync::Arc;

use cicada_core::Wave;

/// Por debajo de esto una señal de audio es inaudible, pero seguir
/// multiplicándola lleva al rango subnormal, donde la CPU se arrastra.
pub(crate) const DENORMAL: f32 = 1e-20;

/// Caída exponencial calculada por multiplicación.
///
/// La forma directa, `exp(-t / tau)`, cuesta una función transcendental por
/// muestra, y el kick usa cinco envolventes a la vez. Multiplicar por una
/// constante da el mismo valor y cuesta una operación.
#[derive(Clone, Copy)]
pub(crate) struct Decay {
    value: f32,
    coeff: f32,
}

impl Decay {
    fn new(tau: f32, sr: f32) -> Decay {
        Decay {
            value: 1.0,
            coeff: (-1.0 / (tau * sr)).exp(),
        }
    }

    /// El valor de esta muestra; avanza a la siguiente.
    fn next(&mut self) -> f32 {
        let v = self.value;
        self.value *= self.coeff;
        // Una caída de 1 ms llega a números subnormales en centésimas de
        // segundo, y en x86 operar con ellos cuesta decenas de ciclos. Por
        // debajo de este umbral ya no se oye nada, así que se corta en seco.
        if self.value < DENORMAL {
            self.value = 0.0;
        }
        v
    }
}

#[derive(Clone)]
pub enum Kind {
    /// Dos barridos de pitch, chasquido de ataque y distorsión: el kick
    /// gabber. El paso alto posterior es lo que lo deja seco.
    Kick {
        phase: f32,
        pitch_slow: Decay,
        pitch_fast: Decay,
        click: Decay,
        body: Decay,
        attack: Decay,
    },
    Snare {
        phase: f32,
        hp: OnePole,
        tone: Decay,
        noise: Decay,
    },
    Hat {
        hp: OnePole,
        env: Decay,
    },
    Synth {
        wave: Wave,
        freq: f32,
        phase: f32,
        gate: u32,
        release: Decay,
    },
    Sample {
        data: Arc<[f32]>,
    },
}

impl Kind {
    pub fn kick(sr: f32) -> Kind {
        Kind::Kick {
            phase: 0.0,
            pitch_slow: Decay::new(0.085, sr),
            pitch_fast: Decay::new(0.011, sr),
            click: Decay::new(0.0018, sr),
            body: Decay::new(0.24, sr),
            attack: Decay::new(0.001, sr),
        }
    }

    pub fn snare(sr: f32) -> Kind {
        Kind::Snare {
            phase: 0.0,
            hp: OnePole::new(1_500.0, sr),
            tone: Decay::new(0.04, sr),
            noise: Decay::new(0.07, sr),
        }
    }

    pub fn hat(sr: f32) -> Kind {
        Kind::Hat {
            hp: OnePole::new(7_000.0, sr),
            env: Decay::new(0.018, sr),
        }
    }

    pub fn synth(wave: Wave, freq: f32, seconds: f32, sr: f32) -> Kind {
        Kind::Synth {
            wave,
            freq,
            phase: 0.0,
            gate: (seconds * sr) as u32,
            release: Decay::new(0.03, sr),
        }
    }

    /// Cuánto dura la voz, en segundos.
    fn lifetime(&self, sr: f32) -> f32 {
        match self {
            Kind::Kick { .. } => 0.7,
            Kind::Snare { .. } => 0.35,
            Kind::Hat { .. } => 0.12,
            Kind::Synth { gate, .. } => *gate as f32 / sr + 0.25,
            Kind::Sample { data } => data.len() as f32 / sr,
        }
    }
}

#[derive(Clone)]
pub struct Voice {
    kind: Kind,
    t: u32,
    /// Muestra en la que la voz se apaga. Comparar contra esto evita
    /// recalcular el tiempo en segundos a cada paso.
    end: u32,
    inv_sr: f32,
    amp: f32,
    pan: f32,
    drive: f32,
    /// Paso alto después de la distorsión. Saturar un grave genera mucha
    /// energía por debajo de la fundamental: quitarla es lo que convierte un
    /// zumbido en el golpe seco del kick gabber.
    post_hp: Option<OnePole>,
    /// `tanh(drive)` para normalizar la saturación. Es constante, pero en la
    /// versión anterior se recalculaba en cada muestra.
    drive_norm: f32,
    noise: u64,
    done: bool,
}

impl Voice {
    pub fn new(kind: Kind, amp: f32, pan: f32, dist: f32, sr: f32, noise_seed: u64) -> Self {
        let post_hp = matches!(kind, Kind::Kick { .. }).then(|| OnePole::new(36.0, sr));
        let drive = 1.0 + dist.powf(0.7) * 28.0;
        Voice {
            t: 0,
            end: (kind.lifetime(sr) * sr) as u32,
            amp: amp * base_level(&kind),
            kind,
            inv_sr: 1.0 / sr,
            pan,
            // La curva no es lineal: los primeros pasos de `dist` ya se notan.
            drive,
            drive_norm: 1.0 / drive.tanh(),
            post_hp,
            noise: noise_seed | 1,
            done: false,
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
        let inv_sr = self.inv_sr;
        let index = self.t;
        self.t += 1;
        self.done = self.t >= self.end;
        let n = self.white();

        let raw = match &mut self.kind {
            Kind::Kick {
                phase,
                pitch_slow,
                pitch_fast,
                click,
                body,
                attack,
            } => {
                // Dos barridos superpuestos: uno muy rápido que da el golpe
                // seco del ataque y otro lento que lleva el tono al grave.
                let freq = 44.0 + 80.0 * pitch_slow.next() + 620.0 * pitch_fast.next();
                *phase = (*phase + freq * inv_sr).fract();
                // Chasquido de los primeros milisegundos: es lo que hace que
                // el kick se oiga por encima de la mezcla.
                let hit = (*phase * TAU).sin() + n * click.next() * 0.55;
                // Ataque de 1 ms para que el primer ciclo no suene a recorte.
                hit * body.next() * (1.0 - attack.next())
            }
            Kind::Snare {
                phase,
                hp,
                tone,
                noise,
            } => {
                *phase = (*phase + 190.0 * inv_sr).fract();
                (*phase * TAU).sin() * tone.next() * 0.6 + hp.high(n) * noise.next() * 0.8
            }
            Kind::Hat { hp, env } => hp.high(n) * env.next(),
            Kind::Synth {
                wave,
                freq,
                phase,
                gate,
                release,
            } => {
                let dt = *freq * inv_sr;
                *phase = (*phase + dt).fract();
                let osc = match wave {
                    Wave::Sine => (*phase * TAU).sin(),
                    Wave::Saw => 2.0 * *phase - 1.0 - poly_blep(*phase, dt),
                    Wave::Square => {
                        let naive = if *phase < 0.5 { 1.0 } else { -1.0 };
                        naive + poly_blep(*phase, dt) - poly_blep((*phase + 0.5).fract(), dt)
                    }
                };
                let attack = (index as f32 * inv_sr / 0.004).min(1.0);
                // La caída solo corre cuando la nota ya se ha soltado.
                let tail = if index >= *gate { release.next() } else { 1.0 };
                osc * attack * tail * 0.4
            }
            Kind::Sample { data } => data.get(index as usize).copied().unwrap_or(0.0),
        };

        let mut out = if self.drive > 1.0 {
            (raw * self.drive).tanh() * self.drive_norm
        } else {
            raw
        };
        if let Some(hp) = &mut self.post_hp {
            out = hp.high(out);
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
///
/// El coeficiente depende solo del corte y de la frecuencia de muestreo, así
/// que se calcula una vez; antes salía un `exp` por muestra y por filtro.
#[derive(Clone, Copy)]
pub(crate) struct OnePole {
    lp: f32,
    coeff: f32,
}

impl OnePole {
    fn new(cutoff: f32, sr: f32) -> OnePole {
        OnePole {
            lp: 0.0,
            coeff: 1.0 - (-TAU * cutoff / sr).exp(),
        }
    }

    fn high(&mut self, x: f32) -> f32 {
        self.lp += self.coeff * (x - self.lp);
        if self.lp.abs() < DENORMAL {
            self.lp = 0.0;
        }
        x - self.lp
    }
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
