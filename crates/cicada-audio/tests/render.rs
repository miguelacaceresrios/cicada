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
    let src = include_str!("../../../examples/04-efectos.cicada");
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

#[test]
fn noise_does_not_depend_on_the_sample_rate() {
    // La vista previa de Studio usa la frecuencia de la tarjeta y la
    // exportación 44,1 kHz: el ruido del hat tiene que sonar igual en las dos.
    let src = "bpm 120
length 1
seed 9
hat x*8
";
    let program = compile(src).unwrap();
    let at = |sr: u32| {
        let out = render(&program, sr, Path::new(".")).unwrap();
        // Envolvente en 200 puntos, para comparar entre frecuencias distintas.
        let frames = out.len() / CHANNELS;
        (0..200)
            .map(|i| {
                let a = i * frames / 200;
                let b = (i + 1) * frames / 200;
                rms(&out[a * CHANNELS..b * CHANNELS])
            })
            .collect::<Vec<f32>>()
    };
    let (a, b) = (at(44_100), at(48_000));
    let diff = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(diff < 0.02, "la envolvente cambia demasiado: {diff}");
}

#[test]
fn a_slow_tempo_does_not_blow_up_the_delay() {
    // Con el delay atado al tempo sin tope, 20 bpm pedía un búfer enorme.
    let out = render_src(
        "bpm 20
beats 4
length 1
l = synth saw delay 1
l C2 ~
",
    );
    assert!(out.len() < 60 * SR as usize * CHANNELS);
}

#[test]
fn master_never_exceeds_the_true_peak_ceiling() {
    // El pico real es el que alcanza la onda entre muestras: es el que hace
    // recortar a un conversor o a un codificador AAC, y un limitador que
    // solo mire las muestras lo deja pasar.
    let out = render_src(
        "a = synth saw gain 4
kick x*8 gain 4 dist 1
snare x*8 gain 4
a C1*8
length 4
",
    );
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let peak = left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 0.95, "pico de muestra {peak}");

    // Interpolación a 4x entre muestras, como hace la norma BS.1770.
    let mut worst = peak;
    for w in left.windows(4) {
        for k in 1..4 {
            let t = k as f32 / 4.0;
            // Interpolación cúbica de Catmull-Rom: se acerca a lo que
            // reconstruye un conversor mucho más que unir puntos con rectas.
            let (p0, p1, p2, p3) = (w[0], w[1], w[2], w[3]);
            let v = p1
                + 0.5
                    * t
                    * ((p2 - p0)
                        + t * ((2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3)
                            + t * (3.0 * (p1 - p2) + p3 - p0)));
            worst = worst.max(v.abs());
        }
    }
    assert!(worst <= 1.0, "pico real {worst}");
}

#[test]
fn master_scales_the_output_predictably() {
    // `master` va después del limitador a propósito: puesto antes, el
    // limitador recupera lo que se le quite y el archivo sale igual de alto,
    // solo que menos apretado. Bajarlo a la mitad da -6 dB justos.
    let entero = render_src(
        "kick x*4
a = synth saw
a C2*4
length 2
",
    );
    let mitad = render_src(
        "master 0.5
kick x*4
a = synth saw
a C2*4
length 2
",
    );
    let ratio = rms(&mitad) / rms(&entero);
    assert!((ratio - 0.5).abs() < 0.01, "salió {ratio}, se esperaba 0.5");
}
