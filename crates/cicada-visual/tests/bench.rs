//! Medidas de rendimiento del motor visual. No son tests: se ejecutan a mano
//! con `cargo test -p cicada-visual --release -- --ignored --nocapture`.

use std::time::Instant;

use cicada_visual::{Options, Visualizer};

const SRC: &str = include_str!("../../../examples/hakken.cicada");
const FRAMES: usize = 120;

fn bench(label: &str, options: Options) {
    let program = cicada_core::compile(SRC).unwrap();
    let sr = 44_100;
    let audio = cicada_audio::render(&program, sr, std::path::Path::new(".")).unwrap();
    let mut vis = Visualizer::new(&program, SRC, "bench", &audio, sr, options).unwrap();

    // Un frame de calentamiento: el primero incluye compilar el shader.
    vis.render_at(20.0).unwrap();
    let started = Instant::now();
    for i in 0..FRAMES {
        vis.render_at(20.0 + i as f64 / options.fps as f64).unwrap();
    }
    let per = started.elapsed().as_secs_f64() / FRAMES as f64;
    println!(
        "{label:28} {:6.1} fps  ({:5.1} ms/frame)",
        1.0 / per,
        per * 1000.0
    );
}

#[test]
#[ignore = "medida, no comprobación"]
fn frames_per_second() {
    for (label, width, height, code) in [
        ("1080p con código", 1920, 1080, true),
        ("1080p sin código", 1920, 1080, false),
        ("720p con código", 1280, 720, true),
        ("540p con código", 960, 540, true),
    ] {
        bench(
            label,
            Options {
                width,
                height,
                fps: 60,
                code,
            },
        );
    }
}
