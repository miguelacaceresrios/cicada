//! Texto → AST. Cada línea se clasifica por su forma:
//!
//! ```text
//! bpm 195                    ajuste
//! bass = synth saw gain 0.8  sonido
//! drop 16:                   sección (el cuerpo va indentado)
//! kick x.x.x.x. dist 0.8     patrón
//! ```

use crate::ast::*;
use crate::diag::{Diagnostic, Pos, Spanned};
use crate::lexer::{Line, TokKind, Token, lex};
use crate::note::parse_note;

pub const SETTINGS: &[&str] = &["bpm", "seed", "beats", "length", "master"];
/// Datos de la pieza: van entre comillas y acaban en la cabecera del vídeo.
pub const META: &[&str] = &["title", "artist"];
/// Modificadores de voz: valen tanto en la declaración de un sonido como en
/// una línea de patrón, donde afectan solo a esa línea.
pub const VOICE_MODIFIERS: &[&str] = &["gain", "dist", "pan", "prob"];
/// Modificadores de efectos: solo en la declaración de la pista, porque
/// actúan sobre toda su cadena y no sobre un golpe suelto.
pub const FX_MODIFIERS: &[&str] = &["lpf", "hpf", "delay", "reverb"];

pub fn is_modifier(name: &str) -> bool {
    VOICE_MODIFIERS.contains(&name) || FX_MODIFIERS.contains(&name)
}

const PATTERN_HELP: &str = "golpes: x X ? .  ·  notas: C1 F#2 Bb0  ·  silencio: ~";

pub fn parse(src: &str) -> Result<File, Vec<Diagnostic>> {
    let (lines, diags) = lex(src);
    let mut p = Parser { lines, i: 0, diags };
    let items = p.items();
    if p.diags.is_empty() {
        Ok(File { items })
    } else {
        Err(p.diags)
    }
}

struct Parser {
    lines: Vec<Line>,
    i: usize,
    diags: Vec<Diagnostic>,
}

/// `None` significa que el error ya quedó registrado en `diags`.
type P<T> = Option<T>;

impl Parser {
    fn error(&mut self, pos: Pos, msg: impl Into<String>) {
        self.diags.push(Diagnostic::new(pos, msg));
    }

    fn error_help(&mut self, pos: Pos, msg: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::new(pos, msg).with_help(help));
    }

    fn next_line(&mut self) -> Option<Line> {
        let line = self.lines.get(self.i).cloned();
        self.i += line.is_some() as usize;
        line
    }

    fn next_is_indented(&self) -> bool {
        self.lines.get(self.i).is_some_and(|l| l.indent > 0)
    }

    fn items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        while let Some(line) = self.next_line() {
            if line.indent > 0 {
                self.error_help(
                    line.tokens[0].pos,
                    "indentación inesperada",
                    "solo el contenido de una sección va indentado",
                );
                continue;
            }
            if let Some(item) = self.item(&line) {
                items.push(item);
            }
        }
        items
    }

    fn item(&mut self, line: &Line) -> P<Item> {
        let toks = &line.tokens;
        if toks.last().map(|t| &t.kind) == Some(&TokKind::Colon) {
            return self.section(line).map(Item::Section);
        }
        if toks.get(1).map(|t| &t.kind) == Some(&TokKind::Eq) {
            return self.sound(line).map(Item::Sound);
        }
        let head = self.word(
            &toks[0],
            "se esperaba un ajuste, un sonido, una sección o una pista",
        )?;
        if SETTINGS.contains(&head.value.as_str()) {
            let value = self.single_number(line, &head)?;
            return Some(Item::Setting { name: head, value });
        }
        if META.contains(&head.value.as_str()) {
            let value = self.single_string(line, &head)?;
            return Some(Item::Meta { name: head, value });
        }
        self.pattern(head, &toks[1..]).map(Item::Pattern)
    }

    fn word(&mut self, tok: &Token, msg: &str) -> P<Spanned<String>> {
        match &tok.kind {
            TokKind::Word(w) => Some(Spanned {
                value: w.clone(),
                pos: tok.pos,
            }),
            _ => {
                self.error(tok.pos, msg);
                None
            }
        }
    }

    fn number(&mut self, tok: Option<&Token>, prev: &Token, what: &str) -> P<Spanned<f64>> {
        match tok {
            Some(Token {
                kind: TokKind::Number(n),
                pos,
            }) => Some(Spanned {
                value: *n,
                pos: *pos,
            }),
            Some(t) => {
                self.error(t.pos, format!("se esperaba un número para {what}"));
                None
            }
            None => {
                self.error(prev.pos.after(), format!("falta el número de {what}"));
                None
            }
        }
    }

    /// `nombre número` y nada más.
    fn single_number(&mut self, line: &Line, head: &Spanned<String>) -> P<Spanned<f64>> {
        let toks = &line.tokens;
        let value = self.number(toks.get(1), &toks[0], &format!("`{}`", head.value))?;
        if let Some(extra) = toks.get(2) {
            self.error(extra.pos, "sobra texto al final de la línea");
            return None;
        }
        Some(value)
    }

    /// `nombre "texto"` y nada más.
    fn single_string(&mut self, line: &Line, head: &Spanned<String>) -> P<Spanned<String>> {
        let toks = &line.tokens;
        let value = match toks.get(1) {
            Some(Token {
                kind: TokKind::Str(text),
                pos,
            }) => Spanned {
                value: text.clone(),
                pos: *pos,
            },
            Some(t) => {
                self.error_help(
                    t.pos,
                    format!("`{}` necesita un texto entre comillas", head.value),
                    format!("por ejemplo: {} \"Mi track\"", head.value),
                );
                return None;
            }
            None => {
                self.error(
                    toks[0].pos.after(),
                    format!("falta el texto de `{}`", head.value),
                );
                return None;
            }
        };
        if let Some(extra) = toks.get(2) {
            self.error(extra.pos, "sobra texto al final de la línea");
            return None;
        }
        Some(value)
    }

    fn section(&mut self, line: &Line) -> P<SectionDecl> {
        let toks = &line.tokens;
        let header = match toks.as_slice() {
            [name, cycles, _colon] => {
                let name = self.word(name, "se esperaba el nombre de la sección");
                let cycles = self.number(Some(cycles), &toks[0], "ciclos de la sección");
                name.zip(cycles)
            }
            _ => {
                self.error_help(
                    toks[0].pos,
                    "cabecera de sección inválida",
                    "formato: `nombre ciclos:`, por ejemplo `drop 16:`",
                );
                None
            }
        };

        // El cuerpo se consume aunque la cabecera falle, para no arrastrar errores.
        let mut energy = None;
        let mut patterns = Vec::new();
        while self.next_is_indented() {
            let body = self.next_line().unwrap();
            let Some(head) = self.word(&body.tokens[0], "se esperaba `energy` o una pista") else {
                continue;
            };
            if head.value == "energy" {
                energy = self.single_number(&body, &head).or(energy);
            } else if let Some(p) = self.pattern(head, &body.tokens[1..]) {
                patterns.push(p);
            }
        }

        let (name, cycles) = header?;
        Some(SectionDecl {
            name,
            cycles,
            energy,
            patterns,
        })
    }

    fn sound(&mut self, line: &Line) -> P<SoundDecl> {
        let toks = &line.tokens;
        let track = self.word(&toks[0], "se esperaba el nombre de la pista")?;
        let Some(src) = toks.get(2) else {
            self.error(toks[1].pos.after(), "falta el sonido después de `=`");
            return None;
        };
        let kind = self.word(src, "se esperaba un sonido")?;
        let mut rest = 3;
        let source = match kind.value.as_str() {
            "kick" => Source::Kick,
            "snare" => Source::Snare,
            "hat" => Source::Hat,
            "synth" => match toks.get(3).map(|t| (&t.kind, t.pos)) {
                Some((TokKind::Word(w), pos)) if !is_modifier(w) => {
                    rest = 4;
                    let Some(wave) = Wave::from_name(w) else {
                        self.error_help(
                            pos,
                            format!("onda desconocida `{w}`"),
                            "usa saw, square o sine",
                        );
                        return None;
                    };
                    Source::Synth(wave)
                }
                _ => Source::Synth(Wave::Saw),
            },
            "sample" => match toks.get(3).map(|t| &t.kind) {
                Some(TokKind::Str(path)) => {
                    rest = 4;
                    Source::Sample(path.clone())
                }
                _ => {
                    self.error_help(
                        src.pos.after(),
                        "falta la ruta del sample",
                        "por ejemplo: sample \"samples/kick.wav\"",
                    );
                    return None;
                }
            },
            other => {
                self.error_help(
                    src.pos,
                    format!("sonido desconocido `{other}`"),
                    "usa kick, snare, hat, synth saw|square|sine o sample \"ruta.wav\"",
                );
                return None;
            }
        };
        let mods = self.modifiers(&toks[rest..])?;
        Some(SoundDecl {
            track,
            source: Spanned {
                value: source,
                pos: src.pos,
            },
            mods,
        })
    }

    fn modifiers(&mut self, toks: &[Token]) -> P<Vec<Modifier>> {
        let mut mods = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let name = match &toks[i].kind {
                TokKind::Word(w) if is_modifier(w) => Spanned {
                    value: w.clone(),
                    pos: toks[i].pos,
                },
                _ => {
                    self.error_help(
                        toks[i].pos,
                        "se esperaba un modificador",
                        "sonido: gain, dist, pan, prob · efectos: lpf, hpf, delay, reverb",
                    );
                    return None;
                }
            };
            let value = self.number(toks.get(i + 1), &toks[i], &format!("`{}`", name.value))?;
            mods.push(Modifier { name, value });
            i += 2;
        }
        Some(mods)
    }

    /// `pista pasos... [modificadores...]`. El patrón termina en el primer modificador.
    fn pattern(&mut self, track: Spanned<String>, toks: &[Token]) -> P<PatternLine> {
        let split = toks
            .iter()
            .position(|t| matches!(&t.kind, TokKind::Word(w) if is_modifier(w)))
            .unwrap_or(toks.len());
        if split == 0 {
            self.error_help(
                track.pos.after(),
                format!("falta el patrón de `{}`", track.value),
                "por ejemplo: `kick x.x.x.x.`",
            );
            return None;
        }
        let steps = self.steps(&toks[..split], &mut 0, None)?;
        let mods = self.modifiers(&toks[split..])?;
        Some(PatternLine { track, steps, mods })
    }

    fn steps(&mut self, toks: &[Token], i: &mut usize, open: Option<Pos>) -> P<Vec<Step>> {
        let mut out = Vec::new();
        while let Some(tok) = toks.get(*i) {
            *i += 1;
            match &tok.kind {
                TokKind::RBracket if open.is_some() => return Some(out),
                TokKind::RBracket => {
                    self.error(tok.pos, "`]` sin su `[`");
                    return None;
                }
                TokKind::LBracket => {
                    let inner = self.steps(toks, i, Some(tok.pos))?;
                    if inner.is_empty() {
                        self.error(tok.pos, "grupo vacío");
                        return None;
                    }
                    out.push(Step::Group(inner));
                }
                TokKind::Word(w) => {
                    let steps = self.word_steps(w, tok.pos)?;
                    if let Some(star) = toks
                        .get(*i)
                        .filter(|t| t.kind == TokKind::Star && steps.len() > 1)
                    {
                        self.error_help(
                            star.pos,
                            "`*` repite un solo paso",
                            format!("agrupa los pasos: [{w}]*n"),
                        );
                        return None;
                    }
                    out.extend(steps);
                }
                TokKind::Star => {
                    let Some(prev) = out.pop() else {
                        self.error(tok.pos, "`*` necesita un paso delante");
                        return None;
                    };
                    let n = self.number(toks.get(*i), tok, "repeticiones")?;
                    *i += 1;
                    if !(1.0..=64.0).contains(&n.value) || n.value.fract() != 0.0 {
                        self.error(n.pos, "las repeticiones deben ser un entero entre 1 y 64");
                        return None;
                    }
                    out.push(Step::Repeat(Box::new(prev), n.value as u32));
                }
                _ => {
                    self.error_help(
                        tok.pos,
                        "esto no puede ir dentro de un patrón",
                        PATTERN_HELP,
                    );
                    return None;
                }
            }
        }
        if let Some(pos) = open {
            self.error(pos, "falta cerrar este `[`");
            return None;
        }
        Some(out)
    }

    /// Una nota es un paso; una palabra de golpes (`x.x?`) es un paso por carácter.
    fn word_steps(&mut self, w: &str, pos: Pos) -> P<Vec<Step>> {
        if let Some(midi) = parse_note(w) {
            return Some(vec![Step::Atom(Spanned {
                value: Atom::Note(midi),
                pos,
            })]);
        }
        let mut out = Vec::new();
        for (k, c) in w.chars().enumerate() {
            let pos = Pos {
                col: pos.col + k,
                len: 1,
                ..pos
            };
            let atom = match c {
                'x' => Atom::Hit,
                'X' => Atom::Accent,
                '?' => Atom::Maybe,
                '.' | '~' => Atom::Rest,
                _ => {
                    self.error_help(
                        pos,
                        format!("`{c}` no es válido en un patrón"),
                        PATTERN_HELP,
                    );
                    return None;
                }
            };
            out.push(Step::Atom(Spanned { value: atom, pos }));
        }
        Some(out)
    }
}
