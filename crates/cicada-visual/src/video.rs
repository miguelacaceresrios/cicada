//! Exportación a MP4: los frames van por una tubería a FFmpeg, que los
//! codifica en H.264 y les une el audio en AAC.

use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{VisualError, Visualizer};

/// Codifica el vídeo completo. `progress` recibe (frames hechos, total).
pub fn encode(
    vis: &mut Visualizer,
    wav: &Path,
    out: &Path,
    ffmpeg: &Path,
    crf: u8,
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
        .args(["-c:v", "libx264", "-preset", "medium", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"])
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
    let mut write_error = None;
    for i in 0..total {
        let frame = vis.render_at(i as f64 / fps)?;
        if let Err(e) = stdin.write_all(frame) {
            // FFmpeg cerró la tubería: su propio mensaje de error explica por qué.
            write_error = Some(e);
            break;
        }
        progress(i + 1, total);
    }
    drop(stdin);

    let mut log = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut log);
    }
    let status = child
        .wait()
        .map_err(|e| VisualError(format!("FFmpeg: {e}")))?;
    match (status.success(), write_error) {
        (true, None) => Ok(()),
        (_, Some(e)) if log.trim().is_empty() => {
            Err(VisualError(format!("FFmpeg dejó de aceptar frames: {e}")))
        }
        _ => Err(VisualError(format!("FFmpeg falló:\n{}", log.trim()))),
    }
}
