use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use cicada_core::Program;

#[derive(Parser)]
#[command(
    name = "cicada",
    version,
    about = "Música electrónica escrita como código"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Comprueba un archivo y resume la pieza, sin generar audio
    Check { file: PathBuf },
    /// Renderiza la pieza a WAV
    Render {
        file: PathBuf,
        /// Archivo de salida (por defecto, el mismo nombre con .wav)
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 44_100)]
        sample_rate: u32,
    },
    /// Reproduce la pieza en tiempo real
    Play { file: PathBuf },
    /// Imprime la CICADA IR en JSON
    Ir {
        file: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    /// Exporta un vídeo MP4 con visuales sincronizadas (necesita FFmpeg)
    Video {
        file: PathBuf,
        /// Archivo de salida (por defecto, el mismo nombre con .mp4)
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        look: Look,
        /// Calidad H.264: menos es mejor (18 es visualmente sin pérdida)
        #[arg(long, default_value_t = 18)]
        crf: u8,
    },
    /// Exporta un frame del vídeo como PNG, para revisar el aspecto sin codificar
    Frame {
        file: PathBuf,
        /// Instante en segundos
        #[arg(long)]
        at: f64,
        /// Archivo de salida (por defecto, nombre-<segundos>.png)
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        look: Look,
    },
}

#[derive(clap::Args)]
struct Look {
    /// Resolución, por ejemplo 1920x1080 o 1280x720
    #[arg(long, default_value = "1920x1080", value_parser = parse_size)]
    size: (u32, u32),
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// Ocultar el código en pantalla
    #[arg(long)]
    no_code: bool,
}

impl Look {
    fn options(&self) -> cicada_visual::Options {
        cicada_visual::Options {
            width: self.size.0,
            height: self.size.1,
            fps: self.fps,
            code: !self.no_code,
        }
    }
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once('x')
        .ok_or("formato: ANCHOxALTO, por ejemplo 1920x1080")?;
    let parse = |v: &str| {
        v.trim()
            .parse::<u32>()
            .map_err(|_| format!("`{v}` no es un número"))
    };
    Ok((parse(w)?, parse(h)?))
}

const SAMPLE_RATE: u32 = 44_100;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let (file, source, program) = match &cli.command {
        Command::Check { file }
        | Command::Render { file, .. }
        | Command::Play { file }
        | Command::Ir { file, .. }
        | Command::Video { file, .. }
        | Command::Frame { file, .. } => match load(file)? {
            Some((source, program)) => (file.clone(), source, program),
            None => return Ok(ExitCode::FAILURE),
        },
    };
    let file = file.as_path();
    let base_dir = file.parent().unwrap_or(Path::new("."));

    match cli.command {
        Command::Check { .. } => print_summary(file, &program),
        Command::Render {
            out, sample_rate, ..
        } => {
            let out = out.unwrap_or_else(|| file.with_extension("wav"));
            let samples = cicada_audio::render(&program, sample_rate, base_dir)?;
            cicada_audio::write_wav(&out, &samples, sample_rate)?;
            println!(
                "✓ {} ({})",
                out.display(),
                clock(duration(samples.len(), sample_rate))
            );
        }
        Command::Play { .. } => {
            let player = cicada_audio::player::Player::new()?;
            let audio = cicada_audio::render(&program, player.sample_rate(), base_dir)?;
            player.set_buffer(audio.into());
            println!(
                "▶ {} · {} · Ctrl+C para parar",
                file.display(),
                clock(player.duration())
            );
            player.play();
            while player.is_playing() {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            // Deja que el último bloque llegue a los altavoces.
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Command::Ir { pretty, .. } => {
            let ir = program.to_ir();
            let json = if pretty {
                serde_json::to_string_pretty(&ir)
            } else {
                serde_json::to_string(&ir)
            }?;
            // Cerrar la tubería antes de tiempo (`cicada ir x | head`) no es un error.
            if let Err(e) = writeln!(std::io::stdout().lock(), "{json}")
                && e.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(e.into());
            }
        }
        Command::Video { out, look, crf, .. } => {
            let out = out.unwrap_or_else(|| file.with_extension("mp4"));
            let audio = cicada_audio::render(&program, SAMPLE_RATE, base_dir)?;
            let wav = std::env::temp_dir().join(format!("cicada-{}.wav", std::process::id()));
            cicada_audio::write_wav(&wav, &audio, SAMPLE_RATE)?;

            let name = file_name(file);
            let mut vis = cicada_visual::Visualizer::new(
                &program,
                &source,
                &name,
                &audio,
                SAMPLE_RATE,
                look.options(),
            )?;
            let ffmpeg = std::env::var_os("CICADA_FFMPEG")
                .map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
            eprintln!(
                "▶ {} · {} frames · GPU: {}",
                out.display(),
                vis.frame_count(),
                vis.adapter_name()
            );

            let started = std::time::Instant::now();
            let result =
                cicada_visual::encode(&mut vis, &wav, &out, &ffmpeg, crf, |done, total| {
                    if done % 30 == 0 || done == total {
                        let fps = done as f64 / started.elapsed().as_secs_f64();
                        eprint!(
                            "\r  {done}/{total} frames · {:.0}% · {fps:.0} fps   ",
                            done as f64 * 100.0 / total as f64
                        );
                    }
                });
            eprintln!();
            let _ = std::fs::remove_file(&wav);
            result?;
            println!(
                "✓ {} ({}, en {:.0} s)",
                out.display(),
                clock(duration(audio.len(), SAMPLE_RATE)),
                started.elapsed().as_secs_f64()
            );
        }
        Command::Frame { at, out, look, .. } => {
            let audio = cicada_audio::render(&program, SAMPLE_RATE, base_dir)?;
            let options = look.options();
            let name = file_name(file);
            let mut vis = cicada_visual::Visualizer::new(
                &program,
                &source,
                &name,
                &audio,
                SAMPLE_RATE,
                options,
            )?;
            let rgba = vis.render_at(at)?;
            let out = out.unwrap_or_else(|| {
                file.with_file_name(format!("{}-{at}s.png", name.trim_end_matches(".cicada")))
            });
            write_png(&out, rgba, options.width, options.height)?;
            println!("✓ {} (t = {at} s)", out.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Compila el archivo. Los errores del lenguaje se imprimen aquí y devuelven `None`.
fn load(path: &Path) -> Result<Option<(String, Program)>> {
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("no se pudo leer {}", path.display()))?;
    match cicada_core::compile(&src) {
        Ok(program) => Ok(Some((src, program))),
        Err(diags) => {
            let name = path.display().to_string();
            for d in &diags {
                eprintln!("{}", d.render(&name, &src));
            }
            let plural = if diags.len() == 1 { "" } else { "es" };
            eprintln!("{} error{plural} en {name}", diags.len());
            Ok(None)
        }
    }
}

fn print_summary(file: &Path, p: &Program) {
    let names: Vec<_> = p.tracks.iter().map(|t| t.name.as_str()).collect();
    println!("✓ {}", file.display());
    println!(
        "  {} bpm · {} beats por ciclo · seed {}",
        p.bpm, p.beats, p.seed
    );
    println!("  {} pistas: {}", names.len(), names.join(", "));
    println!("  {} ciclos · {}", p.length(), clock(p.duration_seconds()));
    for s in &p.sections {
        println!(
            "    {:<10} {:>4} → {:<4} energía {:.2}",
            s.name,
            s.start,
            s.start + s.length,
            s.energy
        );
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "track".into(), |n| n.to_string_lossy().into_owned())
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("no se pudo crear {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// Duración de un render estéreo intercalado, en segundos.
fn duration(samples: usize, sample_rate: u32) -> f64 {
    samples as f64 / cicada_audio::CHANNELS as f64 / sample_rate as f64
}

fn clock(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
}
