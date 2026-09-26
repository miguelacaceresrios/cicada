//! Colores y tipografía compartidos por el vídeo y por CICADA Studio, para
//! que el código se vea igual en los dos.

use crate::timeline::Role;

pub static REGULAR: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
pub static BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.ttf");

pub type Rgb = [f32; 3];

pub const TEXT: Rgb = [0.93, 0.91, 0.89];
pub const DIM: Rgb = [0.55, 0.55, 0.58];
pub const COMMENT: Rgb = [0.40, 0.40, 0.45];
pub const KEYWORD: Rgb = [0.47, 0.78, 0.86];
pub const NUMBER: Rgb = [1.0, 0.63, 0.47];
pub const ACCENT: Rgb = [1.0, 0.18, 0.14];
pub const INK: Rgb = [0.02, 0.02, 0.02];

const KEYWORDS: &[&str] = &[
    "bpm", "seed", "beats", "length", "energy", "gain", "dist", "prob", "synth", "sample", "saw",
    "square", "sine",
];

pub fn role_color(role: Role) -> Rgb {
    match role {
        Role::Kick => [1.0, 0.18, 0.14],
        Role::Snare => [1.0, 1.0, 1.0],
        Role::Hat => [1.0, 0.8, 0.25],
        Role::Melodic => [0.3, 0.86, 1.0],
    }
}

/// Colores de sintaxis por carácter.
pub fn syntax_colors(line: &[char], header: bool) -> Vec<Rgb> {
    let mut colors = vec![TEXT; line.len()];
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            colors[i..].fill(COMMENT);
            break;
        }
        let start = i;
        while i < line.len() && !line[i].is_whitespace() {
            i += 1;
        }
        let word: String = line[start..i].iter().collect();
        let color = if header && start == line.iter().position(|c| !c.is_whitespace()).unwrap_or(0)
        {
            ACCENT
        } else if KEYWORDS.contains(&word.as_str()) {
            KEYWORD
        } else if word.starts_with(|c: char| c.is_ascii_digit()) || word.starts_with('"') {
            NUMBER
        } else {
            TEXT
        };
        colors[start..i].fill(color);
    }
    colors
}

/// Una cabecera de sección (`drop 16:`) sin indentar. Sirve para colorear
/// código que todavía no compila.
pub fn is_section_header(line: &[char]) -> bool {
    let text: String = line.iter().collect();
    let code = text.split(" #").next().unwrap_or("").trim_end();
    !line.first().is_some_and(|c| c.is_whitespace())
        && !code.starts_with('#')
        && code.ends_with(':')
}
