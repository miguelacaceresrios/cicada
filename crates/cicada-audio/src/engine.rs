use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use cicada_core::note::midi_to_hz;
use cicada_core::{EventKind, Program, Rational, Sound, rng};

use crate::AudioError;
use crate::voice::{Kind, Voice};

const MASTER: f32 = 0.6;

/// Convierte eventos en muestras. Procesa bloques de cualquier tamaño y no
/// sabe si escribe a un archivo o a la tarjeta de sonido: `render` y `play`
/// comparten exactamente este código.
pub struct Engine {
    /// Voces pendientes, ordenadas por la muestra en la que empiezan.
    scheduled: Vec<(u64, Voice)>,
    next: usize,
    active: Vec<Voice>,
    pos: u64,
    end: u64,
    sample_rate: u32,
}

impl Engine {
    /// `base_dir` es la carpeta del `.cicada`: las rutas de los samples son relativas a ella.
    pub fn new(program: &Program, sample_rate: u32, base_dir: &Path) -> Result<Self, AudioError> {
        let sr = sample_rate as f32;
        let samples_per_cycle = program.seconds_per_cycle() * sample_rate as f64;

        let mut samples = HashMap::new();
        for (i, track) in program.tracks.iter().enumerate() {
            if let Sound::Sample(path) = &track.sound {
                samples.insert(i, load_sample(&base_dir.join(path), sample_rate)?);
            }
        }

        let events = program.query(Rational::ZERO, Rational::int(program.length()));
        let scheduled = events
            .into_iter()
            .map(|ev| {
                let at = (ev.start.to_f64() * samples_per_cycle).round() as u64;
                let kind = match (&program.tracks[ev.track].sound, ev.kind) {
                    (Sound::Kick, _) => Kind::Kick { phase: 0.0 },
                    (Sound::Snare, _) => Kind::Snare {
                        phase: 0.0,
                        lp: 0.0,
                    },
                    (Sound::Hat, _) => Kind::Hat { lp: 0.0 },
                    (Sound::Synth(wave), EventKind::Note { midi }) => {
                        let seconds = (ev.duration.to_f64() * program.seconds_per_cycle()) as f32;
                        Voice::synth(*wave, midi_to_hz(midi), seconds, sr)
                    }
                    (Sound::Synth(_), EventKind::Hit) => {
                        unreachable!("el análisis semántico impide golpes en un synth")
                    }
                    (Sound::Sample(_), _) => Kind::Sample {
                        data: samples[&ev.track].clone(),
                    },
                };
                let noise_seed = rng::hash(&[program.seed, at, ev.track as u64]);
                (
                    at,
                    Voice::new(kind, ev.velocity * ev.gain, ev.dist, sr, noise_seed),
                )
            })
            .collect();

        Ok(Engine {
            scheduled,
            next: 0,
            active: Vec::with_capacity(256),
            pos: 0,
            end: (program.length() as f64 * samples_per_cycle).round() as u64,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Terminó la pieza y se apagó la última voz.
    pub fn is_finished(&self) -> bool {
        self.pos >= self.end && self.next == self.scheduled.len() && self.active.is_empty()
    }

    /// Rellena `out` (mono) con las siguientes muestras.
    pub fn process(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            while let Some((at, voice)) = self.scheduled.get(self.next) {
                if *at > self.pos {
                    break;
                }
                self.active.push(voice.clone());
                self.next += 1;
            }
            let mix: f32 = self.active.iter_mut().map(Voice::next).sum();
            *sample = (mix * MASTER).tanh();
            self.pos += 1;
        }
        self.active.retain(|v| !v.is_done());
    }
}

/// Renderiza la pieza completa, incluida la cola de las últimas voces.
pub fn render(
    program: &Program,
    sample_rate: u32,
    base_dir: &Path,
) -> Result<Vec<f32>, AudioError> {
    let mut engine = Engine::new(program, sample_rate, base_dir)?;
    let mut out =
        Vec::with_capacity(((program.duration_seconds() + 1.0) * sample_rate as f64) as usize);
    let mut block = [0.0; 512];
    while !engine.is_finished() {
        engine.process(&mut block);
        out.extend_from_slice(&block);
    }
    Ok(out)
}

fn load_sample(path: &Path, sample_rate: u32) -> Result<Arc<[f32]>, AudioError> {
    let fail =
        |e: hound::Error| AudioError(format!("no se pudo leer el sample {}: {e}", path.display()));
    let mut reader = hound::WavReader::open(path).map_err(fail)?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(fail)?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(fail)?
        }
    };
    let channels = spec.channels as usize;
    let mono: Vec<f32> = raw
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(resample(&mono, spec.sample_rate as f64 / sample_rate as f64).into())
}

/// Remuestreo lineal. Suficiente para la 0.1; más adelante, sinc.
fn resample(x: &[f32], ratio: f64) -> Vec<f32> {
    if ratio == 1.0 || x.is_empty() {
        return x.to_vec();
    }
    let len = (x.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let p = i as f64 * ratio;
            let j = p as usize;
            let frac = (p - j as f64) as f32;
            let a = x[j];
            let b = x.get(j + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}
