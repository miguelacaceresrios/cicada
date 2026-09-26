//! AST → Program. Aquí se resuelven los nombres, se validan los rangos y se
//! comprueba que cada patrón encaja con su sonido.

use std::collections::HashMap;

use crate::ast::{self, Atom, File, Item, PatternLine, Source, Step};
use crate::diag::{Diagnostic, Pos, Spanned};
use crate::pattern::flatten;
use crate::program::{Fx, Layer, Params, Program, Section, SectionKind, Sound, Track};

pub fn analyze(file: &File) -> Result<Program, Vec<Diagnostic>> {
    Analyzer::default().run(file)
}

#[derive(Default)]
struct Analyzer {
    diags: Vec<Diagnostic>,
    settings: HashMap<String, Spanned<f64>>,
    tracks: Vec<Track>,
    by_name: HashMap<String, usize>,
}

impl Analyzer {
    fn error(&mut self, pos: Pos, msg: impl Into<String>) {
        self.diags.push(Diagnostic::new(pos, msg));
    }

    fn error_help(&mut self, pos: Pos, msg: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::new(pos, msg).with_help(help));
    }

    fn run(mut self, file: &File) -> Result<Program, Vec<Diagnostic>> {
        // Primera pasada: ajustes y sonidos, para que el orden de las líneas no importe.
        for item in &file.items {
            match item {
                Item::Setting { name, value } => {
                    if self
                        .settings
                        .insert(name.value.clone(), value.clone())
                        .is_some()
                    {
                        self.error(
                            name.pos,
                            format!("`{}` está definido dos veces", name.value),
                        );
                    }
                }
                Item::Sound(decl) => self.sound(decl),
                _ => {}
            }
        }

        // Segunda pasada: patrones y secciones.
        let mut globals = Vec::new();
        let mut sections = Vec::new();
        let mut cursor = 0;
        for item in &file.items {
            match item {
                Item::Pattern(line) => globals.extend(self.layer(line)),
                Item::Section(decl) => {
                    if let Some(section) = self.section(decl, cursor) {
                        cursor += section.length;
                        sections.push(section);
                    }
                }
                _ => {}
            }
        }

        let bpm = self.setting(
            "bpm",
            120.0,
            |v| v > 0.0 && v <= 1000.0,
            "mayor que 0 y como mucho 1000",
        );
        let seed = self.setting(
            "seed",
            0.0,
            |v| v >= 0.0 && v.fract() == 0.0,
            "un entero positivo",
        );
        let beats = self.setting(
            "beats",
            4.0,
            |v| (1.0..=32.0).contains(&v) && v.fract() == 0.0,
            "un entero entre 1 y 32",
        );
        let length = self.setting(
            "length",
            8.0,
            |v| (1.0..=10_000.0).contains(&v) && v.fract() == 0.0,
            "un entero entre 1 y 10000",
        );

        if !self.diags.is_empty() {
            return Err(self.diags);
        }

        // Los patrones de fuera de las secciones suenan en toda la pieza.
        if sections.is_empty() {
            sections.push(Section {
                name: "main".into(),
                kind: SectionKind::Main,
                start: 0,
                length: length as i64,
                energy: 0.5,
                layers: globals,
                line: 0,
            });
        } else {
            for section in &mut sections {
                section.layers.extend(globals.iter().cloned());
            }
        }

        Ok(Program {
            bpm,
            beats: beats as u32,
            seed: seed as u64,
            tracks: self.tracks,
            sections,
        })
    }

    fn setting(
        &mut self,
        name: &str,
        default: f64,
        valid: impl Fn(f64) -> bool,
        expected: &str,
    ) -> f64 {
        match self.settings.get(name).cloned() {
            None => default,
            Some(v) if valid(v.value) => v.value,
            Some(v) => {
                self.error(v.pos, format!("`{name}` debe ser {expected}"));
                default
            }
        }
    }

    fn sound(&mut self, decl: &ast::SoundDecl) {
        let name = &decl.track.value;
        if self.by_name.contains_key(name) {
            self.error(
                decl.track.pos,
                format!("`{name}` ya tiene un sonido declarado"),
            );
            return;
        }
        let sound = match &decl.source.value {
            Source::Kick => Sound::Kick,
            Source::Snare => Sound::Snare,
            Source::Hat => Sound::Hat,
            Source::Synth(wave) => Sound::Synth(*wave),
            Source::Sample(path) => Sound::Sample(path.clone()),
        };
        let mut params = Params::default();
        let mut fx = Fx::NONE;
        self.apply_mods(&decl.mods, &mut params, Some(&mut fx));
        self.add_track(Track {
            name: name.clone(),
            sound,
            params,
            fx,
        });
    }

    fn add_track(&mut self, track: Track) -> usize {
        let index = self.tracks.len();
        self.by_name.insert(track.name.clone(), index);
        self.tracks.push(track);
        index
    }

    /// Busca la pista; `kick`, `snare` y `hat` existen sin declararlas.
    fn track(&mut self, name: &Spanned<String>) -> Option<usize> {
        if let Some(&index) = self.by_name.get(&name.value) {
            return Some(index);
        }
        let sound = match name.value.as_str() {
            "kick" => Sound::Kick,
            "snare" => Sound::Snare,
            "hat" => Sound::Hat,
            other => {
                self.error_help(
                    name.pos,
                    format!("la pista `{other}` no tiene sonido"),
                    format!("declárala, por ejemplo: `{other} = synth saw`"),
                );
                return None;
            }
        };
        Some(self.add_track(Track {
            name: name.value.clone(),
            sound,
            params: Params::default(),
            fx: Fx::NONE,
        }))
    }

    /// Reparte los modificadores. `fx` es `None` en una línea de patrón: allí
    /// los efectos no valen, porque actúan sobre toda la pista.
    fn apply_mods(&mut self, mods: &[ast::Modifier], params: &mut Params, mut fx: Option<&mut Fx>) {
        for m in mods {
            let name = m.name.value.as_str();
            let value = m.value.value;
            let Some(fx) = fx.as_deref_mut() else {
                if crate::parser::FX_MODIFIERS.contains(&name) {
                    self.error_help(
                        m.name.pos,
                        format!("`{name}` se aplica a toda la pista, no a una línea"),
                        format!("ponlo donde declaras el sonido, por ejemplo: `bass = synth saw {name} {value}`"),
                    );
                    continue;
                }
                self.voice_mod(m, params);
                continue;
            };
            match name {
                "lpf" | "hpf" => {
                    if (20.0..=20_000.0).contains(&value) {
                        let slot = if name == "lpf" {
                            &mut fx.lpf
                        } else {
                            &mut fx.hpf
                        };
                        *slot = Some(value as f32);
                    } else {
                        self.error_help(
                            m.value.pos,
                            format!("`{name}` debe estar entre 20 y 20000"),
                            "se indica en hercios, por ejemplo: lpf 800",
                        );
                    }
                }
                "delay" | "reverb" => {
                    if (0.0..=1.0).contains(&value) {
                        let slot = if name == "delay" {
                            &mut fx.delay
                        } else {
                            &mut fx.reverb
                        };
                        *slot = value as f32;
                    } else {
                        self.error(m.value.pos, format!("`{name}` debe estar entre 0 y 1"));
                    }
                }
                _ => self.voice_mod(m, params),
            }
        }
    }

    fn voice_mod(&mut self, m: &ast::Modifier, params: &mut Params) {
        let value = m.value.value;
        let (slot, range) = match m.name.value.as_str() {
            "gain" => (&mut params.gain, 0.0..=4.0),
            "dist" => (&mut params.dist, 0.0..=1.0),
            "pan" => (&mut params.pan, -1.0..=1.0),
            _ => (&mut params.prob, 0.0..=1.0),
        };
        if range.contains(&value) {
            *slot = value as f32;
        } else {
            self.error(
                m.value.pos,
                format!(
                    "`{}` debe estar entre {} y {}",
                    m.name.value,
                    range.start(),
                    range.end()
                ),
            );
        }
    }

    fn layer(&mut self, line: &PatternLine) -> Option<Layer> {
        let track = self.track(&line.track)?;
        let name = &line.track.value;
        let (mut hits, mut notes) = (false, false);
        atom_kinds(&line.steps, &mut hits, &mut notes);
        let is_synth = matches!(self.tracks[track].sound, Sound::Synth(_));

        if hits && notes {
            self.error(
                line.track.pos,
                format!("el patrón de `{name}` mezcla golpes y notas"),
            );
            return None;
        }
        if is_synth && hits {
            self.error_help(
                line.track.pos,
                format!("`{name}` es un synth: su patrón necesita notas"),
                "por ejemplo: C1 C1 G0 C1",
            );
            return None;
        }
        if !is_synth && notes {
            self.error_help(
                line.track.pos,
                format!("`{name}` suena con golpes, no con notas"),
                format!("usa x X ? . o declara `{name} = synth saw`"),
            );
            return None;
        }

        let mut params = self.tracks[track].params;
        self.apply_mods(&line.mods, &mut params, None);
        Some(Layer {
            track,
            slots: flatten(&line.steps),
            params,
        })
    }

    fn section(&mut self, decl: &ast::SectionDecl, start: i64) -> Option<Section> {
        let cycles = decl.cycles.value;
        if cycles < 1.0 || cycles.fract() != 0.0 {
            self.error(
                decl.cycles.pos,
                "los ciclos de una sección deben ser un entero mayor que 0",
            );
            return None;
        }
        let energy = match &decl.energy {
            None => 0.5,
            Some(e) if (0.0..=1.0).contains(&e.value) => e.value as f32,
            Some(e) => {
                self.error(e.pos, "`energy` debe estar entre 0 y 1");
                0.5
            }
        };
        let layers = decl.patterns.iter().filter_map(|p| self.layer(p)).collect();
        Some(Section {
            name: decl.name.value.clone(),
            kind: SectionKind::from_name(&decl.name.value),
            start,
            length: cycles as i64,
            energy,
            layers,
            line: decl.name.pos.line,
        })
    }
}

fn atom_kinds(steps: &[Step], hits: &mut bool, notes: &mut bool) {
    for s in steps {
        match s {
            Step::Atom(a) => match a.value {
                Atom::Note(_) => *notes = true,
                Atom::Rest => {}
                _ => *hits = true,
            },
            Step::Group(inner) => atom_kinds(inner, hits, notes),
            Step::Repeat(inner, _) => atom_kinds(std::slice::from_ref(inner), hits, notes),
        }
    }
}
