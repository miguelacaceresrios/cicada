//! Exportación a MP4: los frames van por una tubería a FFmpeg, que los
//! codifica en H.264 y les une el audio en AAC.

use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use crate::{VisualError, Visualizer};

/// Cuántos frames pueden esperar a que FFmpeg los consuma. Con dos basta para
/// que dibujar y codificar se solapen; más solo gastaría memoria, porque a
/// 1080p cada frame ocupa 8 MB.
const QUEUE: usize = 2;

/// Ajustes de codificación.
#[derive(Clone, Debug)]
pub struct Encoding {
    /// Calidad H.264: menos es mejor. 18 es visualmente sin pérdida.
    pub crf: u8,
    /// Preajuste de x264: `veryfast`, `faster`, `medium`, `slow`…
    pub preset: String,
}

impl Default for Encoding {
    fn default() -> Self {
        // `medium` tardaba el doble que `faster` para una diferencia de SSIM
        // de 0.988, que no se distingue a simple vista y que YouTube borra al
        // recodificar de todas formas.
        Encoding {
            crf: 18,
            preset: "faster".into(),
        }
    }
}

/// Codifica el vídeo completo. `progress` recibe (frames hechos, total).
pub fn encode(
    vis: &mut Visualizer,
    wav: &Path,
    out: &Path,
    ffmpeg: &Path,
    encoding: &Encoding,
    mut progress: impl FnMut(usize, usize),
) -> Result<(), VisualError> {
    let opts = vis.options();
    let mut command = Command::new(ffmpeg);
    // Desde una app de ventana, Windows abriría una consola para FFmpeg.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = command
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{}x{}", opts.width, opts.height)])
        .args(["-r", &opts.fps.to_string(), "-i", "-"])
        .arg("-i")
        .arg(wav)
        .args(["-c:v", "libx264", "-preset", &encoding.preset])
        .args(["-crf", &encoding.crf.to_string(), "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "320k", "-shortest", "-movflags", "+faststart"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => VisualError(format!(
                "no se encontró FFmpeg en `{}`. Instálalo (en Windows: `winget install Gyan.FFmpeg`) \
                 o indica su ruta con la variable CICADA_FFMPEG",
                ffmpeg.display()
            )),
            _ => VisualError(format!("no se pudo iniciar FFmpeg: {e}")),
        })?;

    let mut stdin = child.stdin.take().expect("stdin configurado como piped");
    let total = vis.frame_count();
    let fps = opts.fps as f64;

    // Un hilo aparte alimenta la tubería. Antes, dibujar esperaba a que
    // FFmpeg tragara cada frame; así la GPU trabaja en el siguiente mientras
    // el codificador se ocupa del anterior.
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
    // Los búferes vuelven por aquí para reutilizarse: a 1080p cada frame son
    // 8 MB, y pedirle uno nuevo al asignador por frame son decenas de
    // gigabytes de ida y vuelta en un vídeo de un minuto.
    let (recycle_tx, recycle_rx) = mpsc::channel::<Vec<u8>>();
    let writer = std::thread::spawn(move || {
        for frame in rx {
            if let Err(e) = stdin.write_all(&frame) {
                // FFmpeg cerró la tubería; su propio error dirá por qué.
                return Some(e);
            }
            // Si el otro lado ya terminó, el búfer se descarta sin más.
            let _ = recycle_tx.send(frame);
        }
        drop(stdin);
        None
    });

    let mut sent = 0;
    for i in 0..total {
        let frame = vis.render_at(i as f64 / fps)?;
        let mut buffer = recycle_rx.try_recv().unwrap_or_default();
        buffer.clear();
        buffer.extend_from_slice(frame);
        if tx.send(buffer).is_err() {
            break;
        }
        sent += 1;
        progress(sent, total);
    }
    drop(tx);

    let write_error = writer.join().unwrap_or(None);
    let mut log = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut log);
    }
    let status = child
        .wait()
        .map_err(|e| VisualError(format!("FFmpeg: {e}")))?;
    match (status.success() && sent == total, write_error) {
        (true, None) => Ok(()),
        (_, Some(e)) if log.trim().is_empty() => {
            Err(VisualError(format!("FFmpeg dejó de aceptar frames: {e}")))
        }
        _ if log.trim().is_empty() => Err(VisualError(
            "FFmpeg terminó antes de recibir todos los frames".into(),
        )),
        _ => Err(VisualError(format!("FFmpeg falló:\n{}", log.trim()))),
    }
}
