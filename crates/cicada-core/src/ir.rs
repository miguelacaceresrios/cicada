//! CICADA IR: el contrato entre el núcleo y los motores de salida (audio,
//! visuales, editor). Es serializable para poder inspeccionarla y testearla.

use serde::Serialize;

use crate::program::{Event, Program, SectionKind};
use crate::time::Rational;

#[derive(Clone, Debug, Serialize)]
pub struct Ir {
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    pub bpm: f64,
    pub beats: u32,
    pub seed: u64,
    pub length_cycles: i64,
    pub seconds_per_cycle: f64,
    pub tracks: Vec<IrTrack>,
    pub sections: Vec<IrSection>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IrTrack {
    pub name: String,
    pub sound: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct IrSection {
    pub name: String,
    pub kind: SectionKind,
    pub start: i64,
    pub length: i64,
    pub energy: f32,
}

impl Program {
    pub fn to_ir(&self) -> Ir {
        Ir {
            version: env!("CARGO_PKG_VERSION"),
            title: self.title.clone(),
            artist: self.artist.clone(),
            bpm: self.bpm,
            beats: self.beats,
            seed: self.seed,
            length_cycles: self.length(),
            seconds_per_cycle: self.seconds_per_cycle(),
            tracks: self
                .tracks
                .iter()
                .map(|t| IrTrack {
                    name: t.name.clone(),
                    sound: t.sound.describe(),
                })
                .collect(),
            sections: self
                .sections
                .iter()
                .map(|s| IrSection {
                    name: s.name.clone(),
                    kind: s.kind,
                    start: s.start,
                    length: s.length,
                    energy: s.energy,
                })
                .collect(),
            events: self.query(Rational::ZERO, Rational::int(self.length())),
        }
    }
}
