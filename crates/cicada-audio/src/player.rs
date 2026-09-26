//! Reproducción en tiempo real de un render ya calculado.
//!
//! El audio se renderiza entero antes (es rápido: un minuto de música tarda
//! menos de un segundo), así que el hilo de audio solo copia muestras. Eso
//! hace triviales la pausa, los saltos y la recarga en caliente, y garantiza
//! que lo que suena es exactamente lo que exporta `render`.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::AudioError;
use crate::fx::CHANNELS;

/// `buffer` viene intercalado (izquierda, derecha) y `pos` cuenta muestras
/// sueltas dentro de él, no frames.
struct State {
    buffer: Arc<[f32]>,
    pos: usize,
    playing: bool,
}

pub struct Player {
    state: Arc<Mutex<State>>,
    sample_rate: u32,
    _stream: cpal::Stream,
}

fn err(e: impl std::fmt::Display) -> AudioError {
    AudioError(format!("audio: {e}"))
}

impl Player {
    /// Abre la salida de audio por defecto. El render debe hacerse a
    /// `sample_rate()` para que suene a la velocidad correcta.
    pub fn new() -> Result<Player, AudioError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| AudioError("no hay ningún dispositivo de salida de audio".into()))?;
        let supported = device.default_output_config().map_err(err)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let state = Arc::new(Mutex::new(State {
            buffer: Arc::from([]),
            pos: 0,
            playing: false,
        }));

        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, &state),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, &state),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, &state),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, &state),
            other => {
                return Err(AudioError(format!(
                    "formato de audio no soportado: {other:?}"
                )));
            }
        }?;
        stream.play().map_err(err)?;
        Ok(Player {
            state,
            sample_rate: config.sample_rate,
            _stream: stream,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        // Si el hilo de audio entró en pánico el estado sigue siendo válido.
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Cambia lo que suena sin mover la posición: recarga en caliente.
    pub fn set_buffer(&self, buffer: Arc<[f32]>) {
        debug_assert!(
            buffer.len().is_multiple_of(CHANNELS),
            "el búfer debe ser estéreo"
        );
        self.with(|s| {
            s.pos = s.pos.min(buffer.len());
            s.buffer = buffer;
        });
    }

    pub fn play(&self) {
        self.with(|s| {
            if s.pos >= s.buffer.len() {
                s.pos = 0;
            }
            s.playing = true;
        });
    }

    pub fn pause(&self) {
        self.with(|s| s.playing = false);
    }

    pub fn is_playing(&self) -> bool {
        self.with(|s| s.playing)
    }

    pub fn seek(&self, seconds: f64) {
        let sr = self.sample_rate as f64;
        self.with(|s| {
            let frame = (seconds.max(0.0) * sr) as usize;
            s.pos = (frame * CHANNELS).min(s.buffer.len() - s.buffer.len() % CHANNELS);
        });
    }

    /// Posición actual en segundos.
    pub fn position(&self) -> f64 {
        self.seconds(self.with(|s| s.pos))
    }

    pub fn duration(&self) -> f64 {
        self.seconds(self.with(|s| s.buffer.len()))
    }

    fn seconds(&self, samples: usize) -> f64 {
        samples as f64 / CHANNELS as f64 / self.sample_rate as f64
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: &Arc<Mutex<State>>,
) -> Result<cpal::Stream, AudioError> {
    let channels = config.channels as usize;
    let state = Arc::clone(state);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                // Nunca bloquear el hilo de audio: si la interfaz tiene el
                // estado tomado, este bloque suena en silencio.
                let Ok(mut s) = state.try_lock() else {
                    data.fill(T::from_sample(0.0f32));
                    return;
                };
                for frame in data.chunks_mut(channels) {
                    let stereo = if s.playing && s.pos + CHANNELS <= s.buffer.len() {
                        s.pos += CHANNELS;
                        [s.buffer[s.pos - CHANNELS], s.buffer[s.pos - CHANNELS + 1]]
                    } else {
                        s.playing = false;
                        [0.0; CHANNELS]
                    };
                    // Si el dispositivo es mono se suman; si tiene más de dos
                    // canales, los de más repiten el par estéreo.
                    if channels == 1 {
                        frame[0] = T::from_sample((stereo[0] + stereo[1]) * 0.5);
                    } else {
                        for (i, out) in frame.iter_mut().enumerate() {
                            *out = T::from_sample(stereo[i % CHANNELS]);
                        }
                    }
                }
            },
            |e| eprintln!("error de audio: {e}"),
            None,
        )
        .map_err(err)
}
