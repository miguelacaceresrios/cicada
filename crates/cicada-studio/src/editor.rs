//! Resaltado del editor: los mismos colores que el código del vídeo, más los
//! pasos que suenan iluminados y los errores subrayados.

use cicada_core::Diagnostic;
use cicada_visual::palette::{INK, TEXT, is_section_header, role_color, syntax_colors};
use cicada_visual::timeline::Active;
use eframe::egui::{Color32, FontId, Stroke, TextFormat, text::LayoutJob};

use crate::theme::{self, ACCENT};

pub fn layout_job(text: &str, font: FontId, active: &[Active], errors: &[Diagnostic]) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut run_format: Option<TextFormat> = None;

    for (index, line) in text.split_inclusive('\n').enumerate() {
        let line_no = index + 1;
        let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
        let colors = syntax_colors(&chars, is_section_header(&chars));

        for (k, ch) in line.chars().enumerate() {
            let col = k + 1;
            let mut format = TextFormat {
                font_id: font.clone(),
                color: theme::color(colors.get(k).copied().unwrap_or(TEXT)),
                ..Default::default()
            };
            let lit = active
                .iter()
                .filter(|a| {
                    a.src.line == line_no && (a.src.col..a.src.col + a.src.len).contains(&col)
                })
                .max_by(|a, b| a.intensity.total_cmp(&b.intensity));
            if let Some(a) = lit {
                let [r, g, b] = role_color(a.role).map(|v| (v * 255.0) as u8);
                format.background =
                    Color32::from_rgba_unmultiplied(r, g, b, (a.intensity * 230.0) as u8);
                if a.intensity > 0.45 {
                    format.color = theme::color(INK);
                }
            }
            let error = errors.iter().any(|d| {
                d.pos.line == line_no && (d.pos.col..d.pos.col + d.pos.len.max(1)).contains(&col)
            });
            if error {
                format.background = Color32::from_rgb(90, 14, 12);
                format.underline = Stroke::new(1.5, ACCENT);
            }

            if run_format.as_ref() != Some(&format) {
                if let Some(f) = run_format.take() {
                    job.append(&run, 0.0, f);
                }
                run.clear();
                run_format = Some(format);
            }
            run.push(ch);
        }
    }
    match run_format {
        Some(f) => job.append(&run, 0.0, f),
        // Un archivo vacío también necesita la fuente para colocar el cursor.
        None => job.append(
            "",
            0.0,
            TextFormat {
                font_id: font,
                ..Default::default()
            },
        ),
    }
    job
}

/// Columna de números de línea. Se dibuja como un solo bloque de texto con la
/// misma fuente que el código, así las dos columnas quedan alineadas sin
/// calcular alturas a mano. Las líneas con error salen en rojo.
pub fn gutter_job(
    text: &str,
    font: FontId,
    errors: &[Diagnostic],
    current: Option<usize>,
) -> LayoutJob {
    let total = text.lines().count().max(1);
    let width = total.to_string().len();
    let mut job = LayoutJob::default();
    for line in 1..=total {
        let failed = errors.iter().any(|d| d.pos.line == line);
        let color = match (failed, current == Some(line)) {
            (true, _) => ACCENT,
            (_, true) => theme::TEXT,
            _ => theme::GUTTER,
        };
        job.append(
            &format!("{line:>width$}{}", if line == total { "" } else { "\n" }),
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}
