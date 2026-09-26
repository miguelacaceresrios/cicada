use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use cicada_core::note::midi_to_hz;
use cicada_core::{EventKind, Program, Rational, Sound, rng};

use crate::AudioError;
use crate::fx::{self, CHANNELS, Delay, Frame, Limiter, Reverb, SvfCoeffs, TrackFilter};
use crate::voice::{Kind, Voice};

/// Cuánto se empuja la mezcla contra el limitador. El hardcore vive alto, y
/// el limitador con escucha previa aguanta ese empuje sin chasquear.
const DRIVE: f32 = 1.25;
/// Tiempo del delay en ciclos. Tres dieciseisavos caen a contratiempo del
/// pulso, que es lo que hace que el eco empuje en vez de emborronar.
const DELAY_CYCLES: f64 = 3.0 / 16.0;
const DELAY_FEEDBACK: f32 = 0.38;
/// Nivel de la reverb en el master. La suma de peines de Freeverb sale muy
/// baja, así que sin esto `reverb 1` casi no se oía.
const REVERB_LEVEL: f32 = 1.6;

/// Los efectos de una pista, ya resueltos a coeficientes.
struct Bus {
    filter: TrackFilter,
    lpf: Option<SvfCoeffs>,
    hpf: Option<SvfCoeffs>,
    delay: f32,
    reverb: f32,
    /// Suma de las voces de la pista en esta muestra.
    sum: Frame,
    /// Una pista sin efectos va directa al master, sin tocar filtro ni envíos.
    plain: bool,
}

/// Convierte eventos en muestras estéreo. Procesa bloques de cualquier tamaño
/// y no sabe si escribe a un archivo o a la tarjeta de sonido: `render` y
/// `play` comparten exactamente este código.
///
/// ```text
/// voces --pan--> bus de pista --lpf/hpf--> master --> limitador
///                             \--envíos--> delay + reverb --^
/// ```
pub struct Engine {
    /// Voces pendientes, ordenadas por la muestra en la que empiezan.
    scheduled: Vec<(u64, usize, Voice)>,
    next: usize,
    /// Voces sonando, cada una con el índice de su pista.
    active: Vec<(usize, Voice)>,
    buses: Vec<Bus>,
    delay: Delay,
    reverb: Reverb,
    /// Si ninguna pista manda a un efecto, no se procesa: una reverb son dos
    /// docenas de filtros por muestra, y hacerlos con la entrada a cero es
    /// trabajo tirado.
    uses_delay: bool,
    uses_reverb: bool,
    limiter: Limiter,
    /// Nivel de salida, después del limitador. Aplicarlo antes no serviría:
    /// el limitador recupera lo que se le quite y el archivo sale al mismo
    /// nivel, solo que menos apretado.
    master: f32,
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

        let buses = program
            .tracks
            .iter()
            .map(|t| Bus {
                filter: TrackFilter::default(),
                lpf: t.fx.lpf.map(|hz| SvfCoeffs::new(hz, 0.707, sr)),
                hpf: t.fx.hpf.map(|hz| SvfCoeffs::new(hz, 0.707, sr)),
                delay: t.fx.delay,
                reverb: t.fx.reverb,
                sum: [0.0; CHANNELS],
                plain: t.fx.is_none(),
            })
            .collect();

        let events = program.query(Rational::ZERO, Rational::int(program.length()));
        let scheduled = events
            .into_iter()
            .map(|ev| {
                let at = (ev.start.to_f64() * samples_per_cycle).round() as u64;
                let kind = match (&program.tracks[ev.track].sound, ev.kind) {
                    (Sound::Kick, _) => Kind::kick(sr),
                    (Sound::Snare, _) => Kind::snare(sr),
                    (Sound::Hat, _) => Kind::hat(sr),
                    (Sound::Synth(wave), EventKind::Note { midi }) => {
                        let seconds = (ev.duration.to_f64() * program.seconds_per_cycle()) as f32;
                        Kind::synth(*wave, midi_to_hz(midi), seconds, sr)
                    }
                    (Sound::Synth(_), EventKind::Hit) => {
                        unreachable!("el análisis semántico impide golpes en un synth")
                    }
                    (Sound::Sample(_), _) => Kind::Sample {
                        data: samples[&ev.track].clone(),
                    },
                };
                // La semilla sale de la posición musical, no de la muestra:
                // así el ruido del snare y del hat suena igual a 44,1 kHz que
                // a la frecuencia de la tarjeta, y la vista previa de Studio
                // coincide con lo que se exporta.
                let noise_seed = rng::hash(&[
                    program.seed,
                    ev.start.num() as u64,
                    ev.start.den() as u64,
                    ev.track as u64,
                ]);
                let voice =
                    Voice::new(kind, ev.velocity * ev.gain, ev.pan, ev.dist, sr, noise_seed);
                (at, ev.track, voice)
            })
            .collect();

        // Un tempo muy lento daría un delay de minutos y un búfer enorme.
        let uses_delay = program.tracks.iter().any(|t| t.fx.delay > 0.0);
        let uses_reverb = program.tracks.iter().any(|t| t.fx.reverb > 0.0);
        let delay_seconds = (DELAY_CYCLES * program.seconds_per_cycle()).min(4.0) as f32;
        Ok(Engine {
            scheduled,
            next: 0,
            active: Vec::with_capacity(256),
            buses,
            delay: Delay::new(delay_seconds, DELAY_FEEDBACK, sr),
            reverb: Reverb::new(sr),
            uses_delay,
            uses_reverb,
            limiter: Limiter::new(sr),
            master: program.master,
            pos: 0,
            end: (program.length() as f64 * samples_per_cycle).round() as u64,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Retraso que añade el limitador, en muestras por canal.
    pub fn latency(&self) -> usize {
        self.limiter.latency()
    }

    /// Terminó la pieza y se apagó la última voz.
    pub fn is_finished(&self) -> bool {
        self.pos >= self.end && self.next == self.scheduled.len() && self.active.is_empty()
    }

    /// Rellena `out` con muestras estéreo intercaladas: izquierda, derecha,
    /// izquierda... Su longitud debe ser par.
    pub fn process(&mut self, out: &mut [f32]) {
        // Un bloque impar dejaría un frame a medias: se ignora esa muestra
        // suelta en vez de entrar en pánico dentro del hilo de audio.
        for frame in out.as_chunks_mut::<CHANNELS>().0 {
            while let Some((at, track, voice)) = self.scheduled.get(self.next) {
                if *at > self.pos {
                    break;
                }
                self.active.push((*track, voice.clone()));
                self.next += 1;
            }

            for bus in &mut self.buses {
                bus.sum = [0.0; CHANNELS];
            }
            for (track, voice) in &mut self.active {
                let sample = voice.next();
                let bus = &mut self.buses[*track];
                bus.sum = fx::mix(bus.sum, fx::pan(sample, voice.pan()), 1.0);
            }

            let mut dry = [0.0; CHANNELS];
            let mut to_delay = [0.0; CHANNELS];
            let mut to_reverb = [0.0; CHANNELS];
            for bus in &mut self.buses {
                if bus.plain {
                    dry = fx::mix(dry, bus.sum, 1.0);
                    continue;
                }
                let wet = bus.filter.run(bus.sum, bus.lpf, bus.hpf);
                dry = fx::mix(dry, wet, 1.0);
                to_delay = fx::mix(to_delay, wet, bus.delay);
                to_reverb = fx::mix(to_reverb, wet, bus.reverb);
            }

            let mut mixed = dry;
            if self.uses_delay {
                mixed = fx::mix(mixed, self.delay.run(to_delay), 1.0);
            }
            if self.uses_reverb {
                mixed = fx::mix(mixed, self.reverb.run(to_reverb), REVERB_LEVEL);
            }
            let limited = self.limiter.run([mixed[0] * DRIVE, mixed[1] * DRIVE]);
            *frame = [limited[0] * self.master, limited[1] * self.master];
            self.pos += 1;
        }
        self.active.retain(|(_, v)| !v.is_done());
    }
}

/// Renderiza la pieza completa, incluida la cola de las últimas voces.
pub fn render(
    program: &Program,
    sample_rate: u32,
    base_dir: &Path,
) -> Result<Vec<f32>, AudioError> {
    let mut engine = Engine::new(program, sample_rate, base_dir)?;
    let mut out = Vec::with_capacity(
        ((program.duration_seconds() + 3.0) * sample_rate as f64) as usize * CHANNELS,
    );
    let mut block = [0.0; 512 * CHANNELS];
    while !engine.is_finished() {
        engine.process(&mut block);
        out.extend_from_slice(&block);
    }
    // Cola: las repeticiones del delay y la reverb siguen sonando cuando la
    // última voz ya se apagó.
    let latency = engine.latency() * CHANNELS;
    let mut silence = vec![0.0; 3 * sample_rate as usize * CHANNELS + latency];
    engine.process(&mut silence);
    out.extend_from_slice(&silence);

    // El limitador escucha por delante, así que su salida va retrasada. Al
    // arrancar con la línea de retardo en silencio, descartar esas primeras
    // muestras devuelve el audio a su sitio: cada evento vuelve a caer en su
    // muestra exacta y el vídeo sigue cuadrando al frame.
    out.drain(..latency.min(out.len()));

    // La pieza dura al menos lo que declara `length`; lo que se recorta es
    // solo el silencio posterior a la cola.
    let declared = (program.duration_seconds() * sample_rate as f64) as usize * CHANNELS;
    let min_len = declared.min(out.len());
    trim_silence(&mut out, min_len);
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

/// Recorta el silencio del final, para que la cola dure lo que suena y no un
/// tiempo fijo. Nunca baja de `min_len`.
fn trim_silence(out: &mut Vec<f32>, min_len: usize) {
    // Por debajo de un bit de 16 bits no queda nada audible en el WAV.
    const FLOOR: f32 = 1.0 / 32_768.0;
    let last = out
        .iter()
        .rposition(|v| v.abs() > FLOOR)
        .map_or(0, |i| i + 1);
    let keep = last.max(min_len).next_multiple_of(CHANNELS).min(out.len());
    out.truncate(keep);
}
