//! El lexer trabaja línea a línea: CICADA es un lenguaje orientado a líneas y
//! la indentación solo indica si una línea pertenece a una sección.

use crate::diag::{Diagnostic, Pos};

#[derive(Clone, Debug, PartialEq)]
pub enum TokKind {
    /// Identificadores, notas y patrones de golpes: `kick`, `F#2`, `x.x?`.
    Word(String),
    Number(f64),
    Str(String),
    Eq,
    Colon,
    LBracket,
    RBracket,
    Star,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokKind,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub indent: usize,
    /// Nunca está vacío: las líneas en blanco y los comentarios se descartan.
    pub tokens: Vec<Token>,
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '.' | '?' | '~')
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '?' | '~' | '#')
}

pub fn lex(src: &str) -> (Vec<Line>, Vec<Diagnostic>) {
    let mut lines = Vec::new();
    let mut diags = Vec::new();

    for (index, raw) in src.lines().enumerate() {
        let chars: Vec<char> = raw.chars().collect();
        let at = |start: usize, end: usize| Pos {
            line: index + 1,
            col: start + 1,
            len: end - start,
        };
        let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
        let mut tokens = Vec::new();
        let mut j = 0;

        while j < chars.len() {
            let c = chars[j];
            let start = j;
            if c.is_whitespace() {
                j += 1;
                continue;
            }
            // `#` solo abre un comentario al inicio de una palabra; dentro de `F#2` es un sostenido.
            if c == '#' {
                break;
            }
            let kind = match c {
                '=' | ':' | '[' | ']' | '*' => {
                    j += 1;
                    match c {
                        '=' => TokKind::Eq,
                        ':' => TokKind::Colon,
                        '[' => TokKind::LBracket,
                        ']' => TokKind::RBracket,
                        _ => TokKind::Star,
                    }
                }
                '"' => {
                    j += 1;
                    while j < chars.len() && chars[j] != '"' {
                        j += 1;
                    }
                    if j == chars.len() {
                        diags.push(Diagnostic::new(at(start, j), "falta cerrar las comillas"));
                        break;
                    }
                    j += 1;
                    TokKind::Str(chars[start + 1..j - 1].iter().collect())
                }
                // El signo solo abre un número: `pan -1`. En cualquier otro
                // sitio no significa nada en CICADA.
                c if c.is_ascii_digit()
                    || (c == '-' && chars.get(j + 1).is_some_and(char::is_ascii_digit)) =>
                {
                    j += usize::from(c == '-');
                    while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                        j += 1;
                    }
                    let text: String = chars[start..j].iter().collect();
                    match text.parse() {
                        Ok(n) => TokKind::Number(n),
                        Err(_) => {
                            diags.push(Diagnostic::new(
                                at(start, j),
                                format!("número inválido `{text}`"),
                            ));
                            continue;
                        }
                    }
                }
                c if is_word_start(c) => {
                    while j < chars.len() && is_word_char(chars[j]) {
                        j += 1;
                    }
                    TokKind::Word(chars[start..j].iter().collect())
                }
                _ => {
                    j += 1;
                    diags.push(Diagnostic::new(
                        at(start, j),
                        format!("carácter inesperado `{c}`"),
                    ));
                    continue;
                }
            };
            tokens.push(Token {
                kind,
                pos: at(start, j),
            });
        }

        if !tokens.is_empty() {
            lines.push(Line { indent, tokens });
        }
    }

    (lines, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        let (lines, diags) = lex(src);
        assert!(diags.is_empty(), "{diags:?}");
        lines
            .into_iter()
            .flat_map(|l| l.tokens)
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn sharps_are_not_comments() {
        assert_eq!(
            kinds("bass F#2 C1 # comentario"),
            vec![
                TokKind::Word("bass".into()),
                TokKind::Word("F#2".into()),
                TokKind::Word("C1".into()),
            ]
        );
    }

    #[test]
    fn hit_patterns_are_single_words() {
        assert_eq!(
            kinds("hat ..x? x*4"),
            vec![
                TokKind::Word("hat".into()),
                TokKind::Word("..x?".into()),
                TokKind::Word("x".into()),
                TokKind::Star,
                TokKind::Number(4.0),
            ]
        );
    }

    #[test]
    fn negative_numbers() {
        assert_eq!(
            kinds("hat x pan -0.5"),
            vec![
                TokKind::Word("hat".into()),
                TokKind::Word("x".into()),
                TokKind::Word("pan".into()),
                TokKind::Number(-0.5),
            ]
        );
        // Un guión suelto sigue siendo un error, con su posición.
        let (_, diags) = lex("hat x pan -");
        assert_eq!(diags[0].message, "carácter inesperado `-`");
    }

    #[test]
    fn indentation_and_blank_lines() {
        let (lines, _) = lex("drop 16:\n\n  energy 1\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].indent, 0);
        assert_eq!(lines[1].indent, 2);
        assert_eq!(
            lines[1].tokens[0].pos,
            Pos {
                line: 3,
                col: 3,
                len: 6
            }
        );
    }
}
