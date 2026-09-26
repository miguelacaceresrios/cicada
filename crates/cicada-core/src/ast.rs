//! Árbol sintáctico: lo que el usuario escribió, con posiciones, sin validar.

use serde::Serialize;

use crate::diag::Spanned;

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// `bpm 195`
    Setting {
        name: Spanned<String>,
        value: Spanned<f64>,
    },
    /// `bass = synth saw gain 0.8`
    Sound(SoundDecl),
    /// `drop 16:` seguido de líneas indentadas
    Section(SectionDecl),
    /// Patrón fuera de cualquier sección: suena en toda la pieza.
    Pattern(PatternLine),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundDecl {
    pub track: Spanned<String>,
    pub source: Spanned<Source>,
    pub mods: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Kick,
    Snare,
    Hat,
    Synth(Wave),
    Sample(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Wave {
    Saw,
    Square,
    Sine,
}

impl Wave {
    pub fn from_name(name: &str) -> Option<Wave> {
        match name {
            "saw" => Some(Wave::Saw),
            "square" => Some(Wave::Square),
            "sine" => Some(Wave::Sine),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionDecl {
    pub name: Spanned<String>,
    pub cycles: Spanned<f64>,
    pub energy: Option<Spanned<f64>>,
    pub patterns: Vec<PatternLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatternLine {
    pub track: Spanned<String>,
    pub steps: Vec<Step>,
    pub mods: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Atom(Spanned<Atom>),
    /// `[a b c]`: ocupa un paso y lo subdivide.
    Group(Vec<Step>),
    /// `a*n`: repite `a` n veces dentro de su paso.
    Repeat(Box<Step>, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atom {
    Hit,
    Accent,
    Maybe,
    Rest,
    Note(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub name: Spanned<String>,
    pub value: Spanned<f64>,
}
