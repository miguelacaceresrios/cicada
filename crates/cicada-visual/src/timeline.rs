//! Línea de tiempo visual: la IR convertida en señales continuas (impulsos,
//! energía, escena, destellos) para cualquier instante.
//!
//! Es pura: el estado del segundo 42 se calcula igual lo pidas primero o
//! último, así que el vídeo es determinista y la sincronía, exacta.

use cicada_core::diag::Pos;
use cicada_core::{EventKind, Program, Rational, SectionKind, Sound, Track};

/// Nunca más de 3 destellos por segundo: a 200 bpm un estrobo en cada kick
/// es un riesgo de fotosensibilidad.
pub const MIN_FLASH_GAP: f64 = 1.0 / 3.0;
/// Duración del fundido entre escenas, en segundos.
const TRANSITION: f64 = 0.4;
/// Cuánto hacia atrás se miran los eventos para calcular impulsos.
const LOOKBACK: f64 = 1.0;
const LEVEL_BLOCK: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Kick,
    Snare,
    Hat,
    Melodic,
}

impl Role {
    pub fn of(track: &Track) -> Role {
        match &track.sound {
            Sound::Kick => Role::Kick,
            Sound::Snare => Role::Snare,
            Sound::Hat => Role::Hat,
            Sound::Synth(_) => Role::Melodic,
            // Para los samples, el nombre de la pista es la mejor pista.
            Sound::Sample(_) => {
                let name = track.name.to_lowercase();
                if name.contains("kick") || name.contains("bd") {
                    Role::Kick
                } else if name.contains("hat") || name.contains("hh") {
                    Role::Hat
                } else {
                    Role::Snare
                }
            }
        }
    }

    fn decay(self) -> f64 {
        match self {
            Role::Kick => 0.14,
            Role::Snare => 0.10,
            Role::Hat => 0.05,
            Role::Melodic => 0.20,
        }
    }
}

struct Hit {
    time: f64,
    end: f64,
    role: Role,
    velocity: f32,
    midi: Option<u8>,
    src: Pos,
}

struct Span {
    start: f64,
    end: f64,
    kind: SectionKind,
    energy: f32,
}

/// Un paso del código que está sonando ahora.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Active {
    pub src: Pos,
    pub role: Role,
    pub intensity: f32,
}

#[derive(Clone, Debug)]
pub struct FrameState {
    pub time: f64,
    pub cycle: f64,
    pub section: usize,
    pub scene: SectionKind,
    pub prev_scene: SectionKind,
    /// 0 al empezar la sección, 1 cuando terminó el fundido.
    pub transition: f32,
    /// Avance dentro de la sección, de 0 a 1.
    pub progress: f32,
    /// Energía de la sección, suavizada entre secciones.
    pub energy: f32,
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    pub melodic: f32,
    /// Altura de la nota más reciente, de 0 a 1.
    pub note: f32,
    pub flash: f32,
    /// Nivel RMS del audio renderizado.
    pub level: f32,
    pub active: Vec<Active>,
}

pub struct Timeline {
    hits: Vec<Hit>,
    spans: Vec<Span>,
    flashes: Vec<f64>,
    level: Vec<f32>,
    level_rate: f64,
    pub duration: f64,
    pub seconds_per_cycle: f64,
}

impl Timeline {
    /// `audio` es el render mono de la pieza; su duración manda sobre la del vídeo.
    pub fn new(program: &Program, audio: &[f32], sample_rate: u32) -> Timeline {
        let spc = program.seconds_per_cycle();
        let seconds = |r: Rational| r.to_f64() * spc;

        let hits: Vec<Hit> = program
            .query(Rational::ZERO, Rational::int(program.length()))
            .into_iter()
            .map(|e| Hit {
                time: seconds(e.start),
                end: seconds(e.start + e.duration),
                role: Role::of(&program.tracks[e.track]),
                velocity: e.velocity * e.gain.min(1.5),
                midi: match e.kind {
                    EventKind::Note { midi } => Some(midi),
                    EventKind::Hit => None,
                },
                src: e.src,
            })
            .collect();

        let spans: Vec<Span> = program
            .sections
            .iter()
            .map(|s| Span {
                start: s.start as f64 * spc,
                end: (s.start + s.length) as f64 * spc,
                kind: s.kind,
                energy: s.energy,
            })
            .collect();

        // Nivel de la mezcla con una media móvil de ~0,3 s: sigue la
        // intensidad de la sección sin latir con cada golpe.
        let blocks: Vec<f32> = audio
            .chunks(LEVEL_BLOCK)
            .map(|b| (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt())
            .collect();
        let window = (0.3 * sample_rate as f64 / LEVEL_BLOCK as f64) as usize;
        let level = (0..blocks.len())
            .map(|i| {
                let from = i.saturating_sub(window);
                blocks[from..=i].iter().sum::<f32>() / (i - from + 1) as f32
            })
            .collect();

        Timeline {
            flashes: flash_times(&hits, &spans),
            hits,
            spans,
            level,
            level_rate: sample_rate as f64 / LEVEL_BLOCK as f64,
            duration: audio.len() as f64 / sample_rate as f64,
            seconds_per_cycle: spc,
        }
    }

    pub fn state_at(&self, t: f64) -> FrameState {
        let index = self.spans.iter().rposition(|s| s.start <= t).unwrap_or(0);
        let span = &self.spans[index];
        let prev = &self.spans[index.saturating_sub(1)];
        let transition = if index == 0 {
            1.0
        } else {
            ((t - span.start) / TRANSITION).clamp(0.0, 1.0)
        };
        let blend = smoothstep(transition) as f32;

        let mut state = FrameState {
            time: t,
            cycle: t / self.seconds_per_cycle,
            section: index,
            scene: span.kind,
            prev_scene: prev.kind,
            transition: transition as f32,
            progress: ((t - span.start) / (span.end - span.start)).clamp(0.0, 1.0) as f32,
            energy: prev.energy + (span.energy - prev.energy) * blend,
            kick: 0.0,
            snare: 0.0,
            hat: 0.0,
            melodic: 0.0,
            note: 0.0,
            flash: 0.0,
            level: 0.0,
            active: Vec::new(),
        };

        let (mut kick, mut snare, mut hat, mut melodic) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut latest_note = None;
        let upto = self.hits.partition_point(|h| h.time <= t);
        for hit in self.hits[..upto]
            .iter()
            .rev()
            .take_while(|h| h.time >= t - LOOKBACK)
        {
            let since_end = (t - hit.end).max(0.0);
            // Las notas se sostienen mientras suenan; los golpes solo decaen.
            let env = match hit.midi {
                Some(_) => (-since_end / hit.role.decay()).exp(),
                None => (-(t - hit.time) / hit.role.decay()).exp(),
            } * hit.velocity as f64;
            match hit.role {
                Role::Kick => kick += env,
                Role::Snare => snare += env,
                Role::Hat => hat += env,
                Role::Melodic => melodic += env,
            }
            if let Some(midi) = hit.midi {
                latest_note.get_or_insert(midi);
            }
            let glow = match hit.midi {
                Some(_) => (-since_end / 0.12).exp(),
                None => (-(t - hit.time) / 0.15).exp(),
            } as f32;
            if glow > 0.03 {
                match state.active.iter_mut().find(|a| a.src == hit.src) {
                    Some(a) => a.intensity = a.intensity.max(glow),
                    None => state.active.push(Active {
                        src: hit.src,
                        role: hit.role,
                        intensity: glow,
                    }),
                }
            }
        }

        let squash = |x: f64| (1.0 - (-x).exp()) as f32;
        state.kick = squash(kick * 1.2);
        state.snare = squash(snare * 1.2);
        state.hat = squash(hat);
        state.melodic = squash(melodic);
        state.note = latest_note.map_or(0.0, |m| ((m as f32 - 24.0) / 60.0).clamp(0.0, 1.0));

        let last_flash = self.flashes.partition_point(|&f| f <= t);
        if last_flash > 0 {
            state.flash = (-(t - self.flashes[last_flash - 1]) / 0.07).exp() as f32;
        }
        state.level = self
            .level
            .get((t * self.level_rate) as usize)
            .copied()
            .unwrap_or(0.0);
        state
    }
}

/// Destellos: el inicio de cada drop y los kicks acentuados de las secciones
/// intensas, siempre separados por `MIN_FLASH_GAP` como mínimo.
fn flash_times(hits: &[Hit], spans: &[Span]) -> Vec<f64> {
    let mut candidates: Vec<f64> = spans
        .iter()
        .filter(|s| s.kind == SectionKind::Drop)
        .map(|s| s.start)
        .collect();
    for hit in hits {
        let intense = spans
            .iter()
            .any(|s| s.energy >= 0.8 && (s.start..s.end).contains(&hit.time));
        if hit.role == Role::Kick && hit.velocity >= 1.0 && intense {
            candidates.push(hit.time);
        }
    }
    candidates.sort_by(f64::total_cmp);

    let mut flashes: Vec<f64> = Vec::new();
    for c in candidates {
        if flashes.last().is_none_or(|&last| c - last >= MIN_FLASH_GAP) {
            flashes.push(c);
        }
    }
    flashes
}

fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(src: &str) -> Timeline {
        let program = cicada_core::compile(src).unwrap();
        let seconds = program.duration_seconds() + 0.5;
        Timeline::new(&program, &vec![0.0; (seconds * 1000.0) as usize], 1000)
    }

    #[test]
    fn flashes_never_exceed_three_per_second() {
        // Kicks acentuados en cada semicorchea a 240 bpm: 16 candidatos por segundo.
        let t = timeline("bpm 240\ndrop 8:\n  energy 1\n  kick XXXXXXXXXXXXXXXX\n");
        assert!(t.flashes.len() > 10);
        assert!(t.flashes.windows(2).all(|w| w[1] - w[0] >= MIN_FLASH_GAP));
    }

    #[test]
    fn kick_impulse_follows_the_hits() {
        // 120 bpm: un ciclo dura 2 s y el kick suena en 0 y en 1 s.
        let t = timeline("bpm 120\nkick xx\nlength 1\n");
        assert!(t.state_at(0.001).kick > 0.5);
        assert!(t.state_at(0.9).kick < 0.01);
        assert!(t.state_at(1.001).kick > 0.5);
        assert_eq!(t.state_at(0.001).snare, 0.0);
    }

    #[test]
    fn sections_crossfade() {
        let t =
            timeline("bpm 120\nintro 1:\n  energy 0\n  kick x\ndrop 1:\n  energy 1\n  kick x\n");
        let before = t.state_at(1.9);
        let start = t.state_at(2.0);
        let later = t.state_at(3.0);
        assert_eq!((before.scene, before.energy), (SectionKind::Intro, 0.0));
        assert_eq!(
            (start.scene, start.prev_scene, start.transition),
            (SectionKind::Drop, SectionKind::Intro, 0.0)
        );
        assert_eq!((later.transition, later.energy), (1.0, 1.0));
    }

    #[test]
    fn active_steps_point_at_the_code() {
        let t = timeline("bpm 120\nkick x.x.\n");
        let now = t.state_at(1.0 + 0.01);
        assert_eq!(now.active.len(), 1);
        assert_eq!((now.active[0].src.line, now.active[0].src.col), (2, 8));
    }
}
