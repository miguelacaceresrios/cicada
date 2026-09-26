//! CICADA Studio: editor con resaltado, visuales en vivo, reproducción con
//! recarga en caliente y exportación a WAV y MP4.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod jobs;
mod screenshot;
mod theme;

use std::path::PathBuf;

use eframe::egui;

/// Lo que se abre la primera vez: el primero de los tutoriales.
const EXAMPLE: &str = include_str!("../../../examples/01-primeros-pasos.cicada");
static ICON: &[u8] = include_bytes!("../assets/icon.png");

/// `Documentos\CICADA`, donde viven las piezas del usuario.
pub fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("Documents").join("CICADA"))
}

/// Sin archivo en la línea de comandos se abre el ejemplo de
/// `Documentos\CICADA`, que se crea la primera vez.
pub fn default_file() -> Option<PathBuf> {
    let dir = documents_dir()?;
    let path = dir.join("01-primeros-pasos.cicada");
    if !path.exists() {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, EXAMPLE).ok()?;
    }
    Some(path)
}

fn decode_icon() -> (Vec<u8>, u32, u32) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON))
        .read_info()
        .expect("icono PNG válido");
    let mut rgba = vec![0; reader.output_buffer_size().expect("tamaño del icono")];
    let info = reader.next_frame(&mut rgba).expect("icono PNG válido");
    rgba.truncate(info.buffer_size());
    (rgba, info.width, info.height)
}

pub fn icon_image() -> egui::ColorImage {
    let (rgba, w, h) = decode_icon();
    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
}

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let (rgba, width, height) = decode_icon();
    let viewport = egui::ViewportBuilder::default()
        .with_title("CICADA Studio")
        .with_app_id("cicada-studio")
        .with_inner_size([1440.0, 880.0])
        .with_min_inner_size([980.0, 620.0])
        .with_icon(egui::IconData {
            rgba,
            width,
            height,
        });
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let screenshot = screenshot::Screenshot::from_env();
    eframe::run_native(
        "CICADA Studio",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::Studio::new(file, screenshot)))
        }),
    )
}
