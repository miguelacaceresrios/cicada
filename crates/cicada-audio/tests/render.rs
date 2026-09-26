use std::path::Path;

use cicada_audio::{CHANNELS, render};
use cicada_core::compile;

const SR: u32 = 44_100;

fn render_src(src: &str) -> Vec<f32> {
    render(&compile(src).unwrap(), SR, Path::new(".")).unwrap()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Rango de muestras de un tramo en segundos, sobre audio estéreo intercalado.
fn window(from: f64, to: f64) -> std::ops::Range<usize> {
    let at = |s: f64| (s * SR as f64) as usize * CHANNELS;
    at(from)..at(to)
}

#[test]
fn same_code_same_samples() {
    let src = include_str!("../../../examples/hakken.cicada");
    assert_eq!(render_src(src), render_src(src));
}

#[test]
fn output_is_stereo() {
    // El bajo a un lado y el hat al otro deben dar canales distintos.
    let out = render_src("bass = synth saw pan -1\nhat x*8 pan 1\nbass C1 C1\nlength 2\n");
    assert_eq!(out.len() % CHANNELS, 0);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
    assert!(rms(&left) > 0.01 && rms(&right) > 0.01);
    assert_ne!(left, right, "los dos canales no pueden ser iguales");
}

#[test]
fn length_covers_the_whole_piece() {
    // 8 ciclos x 4 beats a 120 bpm = 16 s, aunque el final sea silencio.
    let out = render_src("bpm 120\nkick x...\nlength 8\n");
    assert!(out.len() >= 16 * SR as usize * CHANNELS);
    assert!(out.len() < 17 * SR as usize * CHANNELS);
}

#[test]
fn events_land_on_the_exact_sample() {
    // A 120 bpm un ciclo dura 2 s: el segundo de cuatro pasos empieza en 0,5 s.
    let out = render_src("bpm 120\nlength 1\nhat .x..\n");
    let first = out.iter().position(|v| v.abs() > 1e-6).unwrap() / CHANNELS;
    assert_eq!(first, SR as usize / 2);
}

#[test]
fn dist_makes_the_kick_louder_and_denser() {
    let clean = render_src("kick x...\nlength 1\n");
    let driven = render_src("kick x... dist 1\nlength 1\n");
    assert!(rms(&driven) > rms(&clean) * 1.5);
    assert!(driven.iter().all(|v| v.abs() <= 1.0));
}

#[test]
fn silent_sections_stay_silent() {
    let out = render_src("intro 1:\n  kick x...\nbreak 1:\n  energy 0\n");
    assert!(rms(&out[window(2.5, 4.0)]) < 1e-4);
}

#[test]
fn master_never_clips() {
    // Muchas pistas a tope a la vez: el limitador tiene que sujetar la suma.
    let out = render_src(
        "a = synth saw gain 4\nb = synth square gain 4\n\
         kick x*4 gain 4 dist 1\nsnare x*4 gain 4\na C1*4\nb C2*4\nlength 2\n",
    );
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 1.0, "pico {peak}");
    assert!(peak > 0.7, "pero sigue sonando fuerte: {peak}");
}

#[test]
fn filters_and_sends_change_the_sound() {
    let plain = render_src("lead = synth saw\nlead C4*8\nlength 2\n");
    let filtered = render_src("lead = synth saw lpf 200\nlead C4*8\nlength 2\n");
    let washed = render_src("lead = synth saw reverb 1\nlead C4*8\nlength 2\n");
    assert!(rms(&filtered) < rms(&plain), "el paso bajo quita energía");
    assert!(washed.len() > plain.len(), "la reverb alarga la cola");
}
