//! Motor visual de CICADA: la IR convertida en frames.
//!
//! ```text
//! Program ─► Timeline ─► FrameState(t) ─► GPU (shader) ─► overlay (código, HUD) ─► RGBA
//! ```
//!
//! Cada frame se calcula a partir de su instante `t = frame / fps`, así que la
//! sincronía con el audio es exacta y el vídeo es reproducible.

mod gpu;
mod overlay;
pub mod palette;
pub mod timeline;
mod video;

use std::fmt;

use cicada_core::Program;

pub use timeline::{FrameState, Timeline};
pub use video::encode;

#[derive(Debug)]
pub struct VisualError(pub String);

impl fmt::Display for VisualError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VisualError {}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Mostrar el código con los pasos activos iluminados.
    pub code: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            width: 1920,
            height: 1080,
            fps: 60,
            code: true,
        }
    }
}

pub struct Visualizer {
    timeline: Timeline,
    gpu: gpu::Gpu,
    overlay: Option<overlay::Overlay>,
    options: Options,
    frame: Vec<u8>,
}

impl Visualizer {
    /// `audio` es el render mono de la pieza: fija la duración del vídeo y
    /// aporta el nivel real de la mezcla.
    pub fn new(
        program: &Program,
        source: &str,
        name: &str,
        audio: &[f32],
        sample_rate: u32,
        options: Options,
    ) -> Result<Visualizer, VisualError> {
        if !options.width.is_multiple_of(2) || !options.height.is_multiple_of(2) {
            return Err(VisualError(
                "el ancho y el alto del vídeo deben ser pares".into(),
            ));
        }
        Ok(Visualizer {
            timeline: Timeline::new(program, audio, sample_rate),
            gpu: gpu::Gpu::new(options.width, options.height)?,
            overlay: options.code.then(|| {
                overlay::Overlay::new(program, source, name, options.width, options.height)
            }),
            options,
            frame: Vec::new(),
        })
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn adapter_name(&self) -> &str {
        &self.gpu.adapter_name
    }

    pub fn frame_count(&self) -> usize {
        (self.timeline.duration * self.options.fps as f64).ceil() as usize
    }

    /// Cambia de pieza sin volver a abrir la GPU (recarga en caliente).
    pub fn load(
        &mut self,
        program: &Program,
        source: &str,
        name: &str,
        audio: &[f32],
        sample_rate: u32,
    ) {
        self.timeline = Timeline::new(program, audio, sample_rate);
        if self.options.code {
            self.overlay = Some(overlay::Overlay::new(
                program,
                source,
                name,
                self.options.width,
                self.options.height,
            ));
        }
    }

    /// Qué está pasando en el instante `t`: impulsos, escena, pasos activos.
    pub fn state_at(&self, t: f64) -> FrameState {
        self.timeline.state_at(t)
    }

    /// El frame de un estado ya calculado, como RGBA.
    pub fn render_state(&mut self, state: &FrameState) -> Result<&[u8], VisualError> {
        self.gpu.render(state, &mut self.frame)?;
        if let Some(overlay) = &mut self.overlay {
            overlay.draw(&mut self.frame, state, self.timeline.duration);
        }
        Ok(&self.frame)
    }

    /// El frame del instante `t` (en segundos), como RGBA.
    pub fn render_at(&mut self, t: f64) -> Result<&[u8], VisualError> {
        let state = self.timeline.state_at(t);
        self.render_state(&state)
    }
}
