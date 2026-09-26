//! Captura automática de la ventana, para documentar y verificar la interfaz
//! sin intervención: `CICADA_STUDIO_SCREENSHOT="captura.png@47"` abre la app,
//! salta al segundo 47, guarda la captura y cierra.

use std::path::PathBuf;

use cicada_audio::player::Player;
use eframe::egui;

pub struct Screenshot {
    path: PathBuf,
    at: f64,
    frames: u32,
}

impl Screenshot {
    pub fn from_env() -> Option<Screenshot> {
        let spec = std::env::var("CICADA_STUDIO_SCREENSHOT").ok()?;
        let (path, at) = spec.rsplit_once('@').unwrap_or((&spec, "0"));
        Some(Screenshot {
            path: path.into(),
            at: at.parse().unwrap_or(0.0),
            frames: 0,
        })
    }

    /// Avanza un frame; devuelve `true` cuando la captura está guardada.
    pub fn step(&mut self, ctx: &egui::Context, player: Option<&Player>, ready: bool) -> bool {
        if !ready {
            return false;
        }
        if self.frames == 0
            && let Some(p) = player
        {
            p.seek(self.at);
        }
        self.frames += 1;
        if self.frames == 20 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else { return false };
        let rgba: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect();
        if let Err(e) = save_png(
            &self.path,
            &rgba,
            image.size[0] as u32,
            image.size[1] as u32,
        ) {
            eprintln!("no se pudo guardar la captura: {e}");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        true
    }
}

fn save_png(
    path: &PathBuf,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
