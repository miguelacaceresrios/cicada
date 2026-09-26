//! Audio de CICADA: eventos de la IR → muestras.

mod engine;
#[cfg(feature = "realtime")]
pub mod player;
mod voice;

use std::fmt;
use std::path::Path;

pub use engine::{Engine, render};

#[derive(Debug)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

/// Escribe WAV estéreo de 16 bits (el mismo canal mono en ambos lados).
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), AudioError> {
    let fail = |e: hound::Error| AudioError(format!("no se pudo escribir {}: {e}", path.display()));
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(fail)?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(v).map_err(fail)?;
        writer.write_sample(v).map_err(fail)?;
    }
    writer.finalize().map_err(fail)
}
