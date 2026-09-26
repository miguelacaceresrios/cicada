use std::path::Path;

use cicada_audio::render;
use cicada_core::compile;

const SR: u32 = 44_100;

fn render_src(src: &str) -> Vec<f32> {
    render(&compile(src).unwrap(), SR, Path::new(".")).unwrap()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn same_code_same_samples() {
    let src = include_str!("../../../examples/hakken.cicada");
    assert_eq!(render_src(src), render_src(src));
}

#[test]
fn length_covers_the_whole_piece() {
    // 8 ciclos × 4 beats a 120 bpm = 16 s, aunque el final sea silencio.
    let out = render_src("bpm 120\nkick x...\nlength 8\n");
    assert!(out.len() >= 16 * SR as usize);
    assert!(out.len() < 17 * SR as usize);
}

#[test]
fn events_land_on_the_exact_sample() {
    // A 120 bpm un ciclo dura 2 s: el segundo de cuatro pasos empieza en 0,5 s.
    let out = render_src("bpm 120\nlength 1\nhat .x..\n");
    let first = out.iter().position(|v| v.abs() > 1e-6).unwrap();
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
    let second_cycle = &out[2 * SR as usize + SR as usize / 2..4 * SR as usize];
    assert!(rms(second_cycle) < 1e-4);
}
