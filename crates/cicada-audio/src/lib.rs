//! Audio de CICADA: eventos de la IR → muestras.

mod engine;
mod fx;
#[cfg(feature = "realtime")]
pub mod player;
mod voice;

use std::fmt;
use std::path::Path;

pub use engine::{Engine, render};
pub use fx::CHANNELS;

#[derive(Debug)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

/// Escribe un WAV estéreo de 16 bits. `samples` viene intercalado:
/// izquierda, derecha, izquierda...
///
/// El redondeo usa ruido de dispersión triangular, que reparte el error de
/// cuantización en vez de dejarlo correlacionado con la señal; sin él, las
/// colas largas de la reverb se apagan con un zumbido áspero.
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), AudioError> {
    let fail = |e: hound::Error| AudioError(format!("no se pudo escribir {}: {e}", path.display()));
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(fail)?;
    let mut noise: u32 = 0x1234_5678;
    let mut rand = || {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        noise as f32 / u32::MAX as f32 - 0.5
    };
    for &s in samples {
        let dither = rand() + rand();
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32 + dither).round();
        writer
            .write_sample(v.clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .map_err(fail)?;
    }
    writer.finalize().map_err(fail)
}
