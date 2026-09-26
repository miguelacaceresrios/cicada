//! Trabajos en segundo plano: render de audio para la vista previa y
//! exportaciones. La interfaz nunca se bloquea esperándolos.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};

use cicada_core::Program;
use eframe::egui;

pub const EXPORT_SAMPLE_RATE: u32 = 44_100;

/// Audio renderizado de una versión concreta del código.
pub struct Loaded {
    pub generation: u64,
    pub program: Program,
    pub source: String,
    pub audio: Arc<[f32]>,
}

pub fn render_preview(
    ctx: &egui::Context,
    generation: u64,
    program: Program,
    source: String,
    sample_rate: u32,
    base_dir: PathBuf,
) -> Receiver<Result<Loaded, String>> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = cicada_audio::render(&program, sample_rate, &base_dir)
            .map(|audio| Loaded {
                generation,
                program,
                source,
                audio: audio.into(),
            })
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

pub struct Export {
    pub label: &'static str,
    pub done: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    pub result: Receiver<Result<PathBuf, String>>,
}

impl Export {
    pub fn progress(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.done.load(Ordering::Relaxed) as f32 / total as f32
        }
    }
}

fn spawn(
    ctx: &egui::Context,
    label: &'static str,
    work: impl FnOnce(&Arc<AtomicUsize>, &Arc<AtomicUsize>, &egui::Context) -> Result<PathBuf, String>
    + Send
    + 'static,
) -> Export {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let (d, t, ctx) = (done.clone(), total.clone(), ctx.clone());
    std::thread::spawn(move || {
        let _ = tx.send(work(&d, &t, &ctx));
        ctx.request_repaint();
    });
    Export {
        label,
        done,
        total,
        result: rx,
    }
}

pub fn export_wav(
    ctx: &egui::Context,
    program: Program,
    base_dir: PathBuf,
    out: PathBuf,
) -> Export {
    spawn(ctx, "Exportando WAV", move |_, _, _| {
        let audio = cicada_audio::render(&program, EXPORT_SAMPLE_RATE, &base_dir)
            .map_err(|e| e.to_string())?;
        cicada_audio::write_wav(&out, &audio, EXPORT_SAMPLE_RATE).map_err(|e| e.to_string())?;
        Ok(out)
    })
}

pub struct VideoJob {
    pub program: Program,
    pub source: String,
    pub name: String,
    pub base_dir: PathBuf,
    pub out: PathBuf,
    pub ffmpeg: PathBuf,
}

pub fn export_video(ctx: &egui::Context, job: VideoJob) -> Export {
    spawn(ctx, "Exportando vídeo", move |done, total, ctx| {
        let sr = EXPORT_SAMPLE_RATE;
        let audio =
            cicada_audio::render(&job.program, sr, &job.base_dir).map_err(|e| e.to_string())?;
        let wav = std::env::temp_dir().join(format!("cicada-studio-{}.wav", std::process::id()));
        cicada_audio::write_wav(&wav, &audio, sr).map_err(|e| e.to_string())?;
        let result = (|| {
            let options = cicada_visual::Options::default();
            let mut vis = cicada_visual::Visualizer::new(
                &job.program,
                &job.source,
                &job.name,
                &audio,
                sr,
                options,
            )
            .map_err(|e| e.to_string())?;
            total.store(vis.frame_count(), Ordering::Relaxed);
            let encoding = cicada_visual::Encoding::default();
            cicada_visual::encode(&mut vis, &wav, &job.out, &job.ffmpeg, &encoding, |d, _| {
                done.store(d, Ordering::Relaxed);
                if d % 15 == 0 {
                    ctx.request_repaint();
                }
            })
            .map_err(|e| e.to_string())
        })();
        let _ = std::fs::remove_file(&wav);
        result.map(|_| job.out)
    })
}

/// Busca FFmpeg: variable `CICADA_FFMPEG`, junto al ejecutable, en el `PATH`
/// o donde lo deja `winget` (el `PATH` de una app abierta antes de instalarlo
/// no se actualiza).
pub fn find_ffmpeg() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Some(p) = std::env::var_os("CICADA_FFMPEG") {
        return Some(p.into());
    }
    // Con `?` aquí, un fallo al leer la ruta del ejecutable se saltaba el
    // resto de la búsqueda y CICADA decía que FFmpeg no estaba instalado.
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(exe_name)));
    if let Some(path) = beside.filter(|p| p.is_file()) {
        return Some(path);
    }
    if let Some(path) = std::env::var_os("PATH")
        && let Some(found) = std::env::split_paths(&path)
            .map(|d| d.join(exe_name))
            .find(|p| p.is_file())
    {
        return Some(found);
    }
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("Microsoft")
        .join("WinGet");
    let link = local.join("Links").join(exe_name);
    if link.is_file() {
        return Some(link);
    }
    find_in_winget_packages(&local.join("Packages"), exe_name)
}

fn find_in_winget_packages(packages: &Path, exe: &str) -> Option<PathBuf> {
    for pkg in std::fs::read_dir(packages).ok()?.flatten() {
        if !pkg.file_name().to_string_lossy().starts_with("Gyan.FFmpeg") {
            continue;
        }
        for build in std::fs::read_dir(pkg.path()).ok()?.flatten() {
            let candidate = build.path().join("bin").join(exe);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Abre una consola con `winget` instalando FFmpeg, a la vista del usuario.
pub fn install_ffmpeg() -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args([
            "/c",
            "start",
            "Instalar FFmpeg",
            "winget",
            "install",
            "-e",
            "--id",
            "Gyan.FFmpeg",
        ])
        .spawn()
        .map(|_| ())
}
