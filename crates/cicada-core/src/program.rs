//! Programa validado y su consulta de eventos.

use serde::Serialize;

use crate::ast::{Atom, Wave};
use crate::diag::Pos;
use crate::pattern::Slot;
use crate::rng;
use crate::time::Rational;

#[derive(Clone, Debug)]
pub struct Program {
    pub bpm: f64,
    /// Beats por ciclo.
    pub beats: u32,
    pub seed: u64,
    pub tracks: Vec<Track>,
    /// Consecutivas y sin huecos, empezando en el ciclo 0.
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub name: String,
    pub sound: Sound,
    pub params: Params,
    pub fx: Fx,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sound {
    Kick,
    Snare,
    Hat,
    Synth(Wave),
    Sample(String),
}

impl Sound {
    pub fn describe(&self) -> String {
        match self {
            Sound::Kick => "kick".into(),
            Sound::Snare => "snare".into(),
            Sound::Hat => "hat".into(),
            Sound::Synth(w) => format!("synth {}", format!("{w:?}").to_lowercase()),
            Sound::Sample(p) => format!("sample \"{p}\""),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub gain: f32,
    pub dist: f32,
    /// Posición estéreo: -1 izquierda, 0 centro, 1 derecha.
    pub pan: f32,
    /// Probabilidad de los golpes `?`.
    pub prob: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gain: 1.0,
            dist: 0.0,
            pan: 0.0,
            prob: 0.5,
        }
    }
}

/// Efectos de una pista. Se aplican a la suma de sus voces, no a cada golpe:
/// un filtro o un delay por golpe no tendría sentido musical ni sería estable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    /// Corte del paso bajo en Hz; `None` si la pista no lo lleva.
    pub lpf: Option<f32>,
    pub hpf: Option<f32>,
    /// Cuánto se manda al delay y a la reverb comunes, de 0 a 1.
    pub delay: f32,
    pub reverb: f32,
}

impl Fx {
    pub const NONE: Fx = Fx {
        lpf: None,
        hpf: None,
        delay: 0.0,
        reverb: 0.0,
    };

    pub fn is_none(&self) -> bool {
        *self == Fx::NONE
    }

    /// Cómo se escribiría en el código, para el inspector de Studio.
    pub fn describe(&self) -> String {
        let mut out = Vec::new();
        if let Some(hz) = self.lpf {
            out.push(format!("lpf {hz:.0}"));
        }
        if let Some(hz) = self.hpf {
            out.push(format!("hpf {hz:.0}"));
        }
        for (name, v) in [("delay", self.delay), ("reverb", self.reverb)] {
            if v > 0.0 {
                out.push(format!("{name} {v}"));
            }
        }
        out.join(" ")
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub kind: SectionKind,
    pub start: i64,
    pub length: i64,
    pub energy: f32,
    pub layers: Vec<Layer>,
    /// Línea de la cabecera en el código; 0 para la sección implícita.
    pub line: usize,
}

/// Qué papel cumple una sección en la pieza. Se deduce del nombre y es lo que
/// el motor visual usa para cambiar de escena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionKind {
    Intro,
    Build,
    Drop,
    Break,
    Outro,
    /// La sección implícita de un archivo sin secciones.
    Main,
    Custom,
}

impl SectionKind {
    /// `drop`, `drop2` y `drop_b` son todas drops.
    pub fn from_name(name: &str) -> SectionKind {
        match name {
            n if n.starts_with("intro") => SectionKind::Intro,
            n if n.starts_with("build") => SectionKind::Build,
            n if n.starts_with("drop") => SectionKind::Drop,
            n if n.starts_with("break") => SectionKind::Break,
            n if n.starts_with("outro") => SectionKind::Outro,
            _ => SectionKind::Custom,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub track: usize,
    pub slots: Vec<Slot>,
    pub params: Params,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EventKind {
    Hit,
    Note { midi: u8 },
}

/// Un evento de la CICADA IR. El tiempo va en ciclos.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    pub start: Rational,
    pub duration: Rational,
    pub track: usize,
    pub section: usize,
    #[serde(flatten)]
    pub kind: EventKind,
    pub velocity: f32,
    pub gain: f32,
    pub dist: f32,
    pub pan: f32,
    /// Posición en el código del paso que lo genera: el editor y el vídeo la
    /// usan para iluminar el código mientras suena.
    pub src: Pos,
}

impl Program {
    /// Duración total en ciclos.
    pub fn length(&self) -> i64 {
        self.sections.last().map_or(0, |s| s.start + s.length)
    }

    pub fn seconds_per_cycle(&self) -> f64 {
        self.beats as f64 * 60.0 / self.bpm
    }

    pub fn duration_seconds(&self) -> f64 {
        self.length() as f64 * self.seconds_per_cycle()
    }

    fn section_at(&self, cycle: i64) -> Option<(usize, &Section)> {
        self.sections
            .iter()
            .enumerate()
            .find(|(_, s)| (s.start..s.start + s.length).contains(&cycle))
    }

    /// Todos los eventos que empiezan en `[from, to)`, ordenados por tiempo.
    ///
    /// Es una función pura: consultar un rango da lo mismo que consultar sus
    /// partes por separado y concatenarlas. En eso se apoyan el tiempo real,
    /// el render offline y, más adelante, el live coding.
    pub fn query(&self, from: Rational, to: Rational) -> Vec<Event> {
        let mut events = Vec::new();
        for cycle in from.floor().max(0)..to.ceil().min(self.length()) {
            let Some((section_index, section)) = self.section_at(cycle) else {
                continue;
            };
            for (layer_index, layer) in section.layers.iter().enumerate() {
                let track = &self.tracks[layer.track];
                for (step, slot) in layer.slots.iter().enumerate() {
                    let start = Rational::int(cycle) + slot.offset;
                    if start < from || start >= to {
                        continue;
                    }
                    let (kind, velocity) = match slot.atom {
                        Atom::Hit => (EventKind::Hit, 0.8),
                        Atom::Accent => (EventKind::Hit, 1.0),
                        Atom::Maybe => {
                            let roll = rng::unit(rng::hash(&[
                                self.seed,
                                rng::hash_str(&track.name),
                                cycle as u64,
                                layer_index as u64,
                                step as u64,
                            ]));
                            if roll >= layer.params.prob as f64 {
                                continue;
                            }
                            (EventKind::Hit, 0.8)
                        }
                        Atom::Note(midi) => (EventKind::Note { midi }, 0.8),
                        Atom::Rest => continue,
                    };
                    events.push(Event {
                        start,
                        duration: slot.duration,
                        track: layer.track,
                        section: section_index,
                        kind,
                        velocity,
                        gain: layer.params.gain,
                        dist: layer.params.dist,
                        pan: layer.params.pan,
                        src: slot.src,
                    });
                }
            }
        }
        events.sort_by(|a, b| a.start.cmp(&b.start).then(a.track.cmp(&b.track)));
        events
    }
}
