//! Capa de texto sobre cada frame: el código con los pasos que suenan
//! iluminados, el HUD con sección, ciclo y beat, y una cabecera de título.
//! Se compone en CPU sobre el frame que devuelve la GPU.

use std::collections::HashMap;

use cicada_core::Program;
use fontdue::{Font, FontSettings, Metrics};

use crate::palette::{ACCENT, BOLD, DIM, INK, REGULAR, Rgb, TEXT, role_color, syntax_colors};
use crate::timeline::FrameState;

struct Glyph {
    metrics: Metrics,
    bitmap: Vec<u8>,
}

struct Face {
    font: Font,
    /// Tamaño base, sin encoger.
    base: f32,
    size: f32,
    cache: HashMap<char, Glyph>,
}

impl Face {
    fn new(bytes: &[u8], size: f32) -> Face {
        let font =
            Font::from_bytes(bytes, FontSettings::default()).expect("fuente incluida válida");
        Face {
            font,
            base: size,
            size,
            cache: HashMap::new(),
        }
    }

    /// Reescala la fuente sobre su tamaño base. Vacía la caché, así que solo
    /// se usa en textos cortos como el título.
    fn scale(&mut self, factor: f32) {
        let size = self.base * factor;
        if size != self.size {
            self.size = size;
            self.cache.clear();
        }
    }

    fn glyph(&mut self, c: char) -> &Glyph {
        self.cache.entry(c).or_insert_with(|| {
            let (metrics, bitmap) = self.font.rasterize(c, self.size);
            Glyph { metrics, bitmap }
        })
    }

    fn advance(&mut self) -> f32 {
        self.glyph('M').metrics.advance_width
    }

    fn width(&mut self, text: &str) -> f32 {
        text.chars().count() as f32 * self.advance()
    }
}

/// Un frame RGBA en memoria sobre el que se pinta.
struct Canvas<'a> {
    px: &'a mut [u8],
    width: i32,
    height: i32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || a <= 0.0 {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        for (px, c) in self.px[i..i + 3].iter_mut().zip(c) {
            let dst = *px as f32 / 255.0;
            *px = ((dst + (c - dst) * a.min(1.0)) * 255.0).round() as u8;
        }
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
        for py in y.round() as i32..(y + h).round() as i32 {
            for px in x.round() as i32..(x + w).round() as i32 {
                self.blend(px, py, c, a);
            }
        }
    }

    /// Dibuja texto con la línea base en `y`. Devuelve la x final.
    fn text(&mut self, face: &mut Face, text: &str, x: f32, y: f32, c: Rgb, a: f32) -> f32 {
        let mut pen = x;
        for ch in text.chars() {
            pen = self.glyph(face, ch, pen, y, c, a);
        }
        pen
    }

    fn glyph(&mut self, face: &mut Face, ch: char, x: f32, y: f32, c: Rgb, a: f32) -> f32 {
        let g = face.glyph(ch);
        let m = g.metrics;
        let gx = x.round() as i32 + m.xmin;
        let gy = y.round() as i32 - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for col in 0..m.width {
                let cov = g.bitmap[row * m.width + col] as f32 / 255.0;
                self.blend(gx + col as i32, gy + row as i32, c, cov * a);
            }
        }
        x + m.advance_width
    }
}

pub struct Overlay {
    code: Face,
    code_bold: Face,
    hud: Face,
    hud_bold: Face,
    title: Face,
    lines: Vec<Vec<char>>,
    colors: Vec<Vec<Rgb>>,
    headers: Vec<bool>,
    /// Sección a la que pertenece cada línea; `None` para las globales.
    owner: Vec<Option<usize>>,
    section_lines: Vec<usize>,
    section_names: Vec<String>,
    /// Lo que se ve en pantalla: el `title` de la pieza si lo lleva, y si no
    /// el nombre del archivo.
    name: String,
    artist: Option<String>,
    bpm: f64,
    beats: u32,
    length: i64,
    scale: f32,
    width: u32,
    height: u32,
}

impl Overlay {
    pub fn new(program: &Program, source: &str, name: &str, width: u32, height: u32) -> Overlay {
        let s = height as f32 / 1080.0;
        let lines: Vec<Vec<char>> = source
            .lines()
            // Un tabulador ocupa una columna, igual que para el lexer: así el
            // resaltado de los pasos cae en el carácter correcto.
            .map(|l| l.replace('\t', " ").chars().collect())
            .collect();
        let section_lines: Vec<usize> = program.sections.iter().map(|sec| sec.line).collect();

        let mut owner = Vec::with_capacity(lines.len());
        let mut headers = Vec::with_capacity(lines.len());
        let mut current = None;
        for (i, line) in lines.iter().enumerate() {
            let header = section_lines.iter().position(|&l| l == i + 1);
            let blank = line.iter().all(|c| c.is_whitespace());
            let indented = line.first().is_some_and(|c| c.is_whitespace());
            if header.is_some() {
                current = header;
            } else if !blank && !indented {
                current = None;
            }
            owner.push(current);
            headers.push(header.is_some());
        }

        Overlay {
            code: Face::new(REGULAR, 21.0 * s),
            code_bold: Face::new(BOLD, 21.0 * s),
            hud: Face::new(REGULAR, 19.0 * s),
            hud_bold: Face::new(BOLD, 22.0 * s),
            title: Face::new(BOLD, 120.0 * s),
            colors: lines
                .iter()
                .zip(&headers)
                .map(|(l, &h)| syntax_colors(l, h))
                .collect(),
            lines,
            headers,
            owner,
            section_lines,
            section_names: program
                .sections
                .iter()
                .map(|sec| sec.name.to_uppercase())
                .collect(),
            name: program.title.clone().unwrap_or_else(|| name.to_string()),
            artist: program.artist.clone(),
            bpm: program.bpm,
            beats: program.beats,
            length: program.length(),
            scale: s,
            width,
            height,
        }
    }

    pub fn draw(&mut self, px: &mut [u8], state: &FrameState, duration: f64) {
        let mut canvas = Canvas {
            px,
            width: self.width as i32,
            height: self.height as i32,
        };
        self.draw_code(&mut canvas, state);
        self.draw_hud(&mut canvas, state, duration);
        self.draw_title(&mut canvas, state.time);
    }

    fn draw_code(&mut self, canvas: &mut Canvas, state: &FrameState) {
        let s = self.scale;
        let adv = self.code.advance();
        let line_h = 21.0 * s * 1.5;
        let (left, top, pad) = (64.0 * s, 72.0 * s, 22.0 * s);
        let bottom = self.height as f32 - 130.0 * s;
        let visible = ((bottom - top) / line_h).floor().max(1.0);

        // Desplazamiento: la cabecera de la sección actual queda arriba, con
        // un fundido desde la posición de la sección anterior.
        let max_scroll = (self.lines.len() as f32 - visible).max(0.0);
        let target = |section: usize| {
            let line = self.section_lines.get(section).copied().unwrap_or(0);
            (line as f32 - 3.0).clamp(0.0, max_scroll)
        };
        let from = target(state.section.saturating_sub(1));
        let blend = state.transition * state.transition * (3.0 - 2.0 * state.transition);
        let scroll = from + (target(state.section) - from) * blend;

        // Los comentarios de línea completa no cuentan para el ancho: son largos
        // y casi nunca están a la vista. Lo que no cabe se recorta.
        let longest = self
            .lines
            .iter()
            .filter(|l| l.iter().find(|c| !c.is_whitespace()) != Some(&'#'))
            .map(|l| l.len())
            .max()
            .unwrap_or(0) as f32;
        let panel_w = (longest * adv + 2.0 * pad).min(self.width as f32 * 0.42);
        let clip = left - pad + panel_w - adv * 0.5;
        let panel_h = visible.min(self.lines.len() as f32) * line_h + pad * 1.4;
        canvas.rect(left - pad, top - pad, panel_w, panel_h, INK, 0.6);
        let bar = if state.energy > 0.75 { ACCENT } else { DIM };
        canvas.rect(left - pad, top - pad, 4.0 * s, panel_h, bar, 0.9);

        let first = scroll.floor() as usize;
        for i in first..(first + visible as usize + 1).min(self.lines.len()) {
            let y = top + (i as f32 - scroll) * line_h;
            let baseline = y + line_h * 0.72;
            // Las líneas que entran o salen del panel se desvanecen.
            let edge = ((y - top + line_h) / line_h)
                .min((top + visible * line_h - y) / line_h)
                .clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let focus = match self.owner[i] {
                Some(sec) if sec == state.section => 1.0,
                Some(_) => 0.32,
                None => 0.75,
            };

            // Fondo de los pasos que están sonando en esta línea.
            let mut lit = vec![0.0f32; self.lines[i].len()];
            for a in state.active.iter().filter(|a| a.src.line == i + 1) {
                let x = left + (a.src.col - 1) as f32 * adv;
                let w = a.src.len as f32 * adv;
                let c = role_color(a.role);
                let glow = 3.0 * s;
                canvas.rect(
                    x - glow,
                    y + 2.0 * s - glow,
                    w + 2.0 * glow,
                    line_h - 4.0 * s + 2.0 * glow,
                    c,
                    0.25 * a.intensity * edge,
                );
                canvas.rect(
                    x,
                    y + 2.0 * s,
                    w,
                    line_h - 4.0 * s,
                    c,
                    0.92 * a.intensity * edge,
                );
                for k in (a.src.col - 1)..(a.src.col - 1 + a.src.len).min(lit.len()) {
                    lit[k] = lit[k].max(a.intensity);
                }
            }

            let face = if self.headers[i] {
                &mut self.code_bold
            } else {
                &mut self.code
            };
            let mut x = left;
            for (k, &ch) in self.lines[i].iter().enumerate() {
                if x + adv > clip {
                    break;
                }
                let base = self.colors[i][k];
                let on = lit[k];
                let c = [0, 1, 2].map(|j| base[j] + (INK[j] - base[j]) * on);
                let alpha = (focus + (1.0 - focus) * on) * edge;
                x = canvas.glyph(face, ch, x, baseline, c, alpha);
            }
        }
    }

    fn draw_hud(&mut self, canvas: &mut Canvas, state: &FrameState, duration: f64) {
        let s = self.scale;
        let (w, h) = (self.width as f32, self.height as f32);
        let baseline = h - 44.0 * s;
        let left = 64.0 * s - 22.0 * s;

        // Degradado oscuro para que el HUD se lea incluso durante un destello.
        let band = 110.0 * s;
        let steps = 24;
        for k in 0..steps {
            let y = h - band + k as f32 * band / steps as f32;
            canvas.rect(
                0.0,
                y,
                w,
                band / steps as f32 + 1.0,
                INK,
                0.75 * (k as f32 / steps as f32),
            );
        }

        let x = canvas.text(&mut self.hud_bold, "CICADA", left, baseline, ACCENT, 1.0);
        let section = self
            .section_names
            .get(state.section)
            .map_or("", String::as_str);
        let info = format!("   {}   {} BPM   ", self.name, self.bpm);
        let x = canvas.text(&mut self.hud, &info, x, baseline, DIM, 1.0);
        canvas.text(&mut self.hud_bold, section, x, baseline, TEXT, 1.0);

        // Derecha: beats del ciclo, ciclo actual y tiempo.
        let cycle = (state.cycle.floor() as i64 + 1).min(self.length);
        let counter = format!(
            "{cycle:>3}/{}   {} / {}",
            self.length,
            clock(state.time),
            clock(duration)
        );
        let counter_w = self.hud.width(&counter);
        let right = w - 42.0 * s;
        canvas.text(
            &mut self.hud,
            &counter,
            right - counter_w,
            baseline,
            DIM,
            1.0,
        );

        let beat = ((state.cycle.fract()) * self.beats as f64).floor() as u32;
        let size = 12.0 * s;
        let gap = 7.0 * s;
        let start = right - counter_w - 24.0 * s - self.beats as f32 * (size + gap);
        for b in 0..self.beats {
            let bx = start + b as f32 * (size + gap);
            let (c, a) = if b == beat {
                (ACCENT, 1.0)
            } else {
                (TEXT, 0.18)
            };
            canvas.rect(bx, baseline - size, size, size, c, a);
        }

        let progress = (state.time / duration).clamp(0.0, 1.0) as f32;
        let bar_h = 4.0 * s;
        canvas.rect(0.0, h - bar_h, w, bar_h, TEXT, 0.12);
        canvas.rect(0.0, h - bar_h, w * progress, bar_h, ACCENT, 0.95);
    }

    /// Cabecera de los primeros segundos: nombre grande que se desvanece.
    fn draw_title(&mut self, canvas: &mut Canvas, t: f64) {
        let fade = 1.0 - ((t as f32 - 1.6) / 1.0).clamp(0.0, 1.0);
        if fade <= 0.0 {
            return;
        }
        let a = fade * fade * (3.0 - 2.0 * fade);
        let (w, h) = (self.width as f32, self.height as f32);
        canvas.rect(0.0, 0.0, w, h, INK, 0.35 * a);

        // El nombre de la pieza en grande, y debajo quien la firma. Un título
        // largo se encoge para que quepa entre los márgenes.
        let title = self.name.to_uppercase();
        let natural = self.title.width(&title);
        let room = w * 0.84;
        let squeeze = (room / natural).min(1.0);
        self.title.scale(squeeze);
        let tw = self.title.width(&title);
        canvas.text(&mut self.title, &title, (w - tw) / 2.0, h * 0.5, TEXT, a);
        self.title.scale(1.0);

        let sub = self
            .artist
            .clone()
            .unwrap_or_else(|| "CICADA".into())
            .to_uppercase();
        let sw = self.hud_bold.width(&sub);
        canvas.text(
            &mut self.hud_bold,
            &sub,
            (w - sw) / 2.0,
            h * 0.5 + 56.0 * self.scale,
            ACCENT,
            a,
        );
    }
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
