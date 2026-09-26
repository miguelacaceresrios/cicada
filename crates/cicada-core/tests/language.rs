use cicada_core::{EventKind, Rational, SectionKind, compile};

fn errors(src: &str) -> Vec<String> {
    compile(src)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn minimal_track_without_sections() {
    let p = compile("bpm 195\nkick x.x.x.x.\nhat xxxxxxxx\n").unwrap();
    assert_eq!(p.bpm, 195.0);
    assert_eq!(p.length(), 8);
    assert_eq!(p.sections[0].kind, SectionKind::Main);
    let first_cycle = p.query(Rational::ZERO, Rational::ONE);
    assert_eq!(
        first_cycle
            .iter()
            .filter(|e| p.tracks[e.track].name == "kick")
            .count(),
        4
    );
    assert_eq!(
        first_cycle
            .iter()
            .filter(|e| p.tracks[e.track].name == "hat")
            .count(),
        8
    );
}

#[test]
fn sections_are_consecutive_and_globals_play_everywhere() {
    let src = "\
hat x.x.

intro 4:
  energy 0.3
  kick x...

drop 8:
  energy 1
  kick xxxx dist 0.8
";
    let p = compile(src).unwrap();
    assert_eq!(p.length(), 12);
    assert_eq!(
        (p.sections[1].start, p.sections[1].kind),
        (4, SectionKind::Drop)
    );
    assert_eq!(p.sections[1].energy, 1.0);

    let drop_cycle = p.query(Rational::int(4), Rational::int(5));
    let kicks: Vec<_> = drop_cycle
        .iter()
        .filter(|e| p.tracks[e.track].name == "kick")
        .collect();
    assert_eq!(kicks.len(), 4);
    assert!(kicks.iter().all(|e| e.dist == 0.8 && e.section == 1));
    assert_eq!(
        drop_cycle
            .iter()
            .filter(|e| p.tracks[e.track].name == "hat")
            .count(),
        2
    );
}

#[test]
fn notes_on_declared_synth() {
    let p = compile("bass = synth square gain 0.7\nbass C1 ~ [G0 C1]\n").unwrap();
    let events = p.query(Rational::ZERO, Rational::ONE);
    let notes: Vec<_> = events.iter().map(|e| (e.start, e.kind)).collect();
    assert_eq!(
        notes,
        vec![
            (Rational::ZERO, EventKind::Note { midi: 24 }),
            (Rational::new(2, 3), EventKind::Note { midi: 19 }),
            (Rational::new(5, 6), EventKind::Note { midi: 24 }),
        ]
    );
    assert!(events.iter().all(|e| e.gain == 0.7));
}

#[test]
fn query_is_the_union_of_its_parts() {
    let p = compile("seed 3\nkick x?x?[x?]*2\nsnare ..x.\n").unwrap();
    let whole = p.query(Rational::ZERO, Rational::int(8));
    let mut parts = Vec::new();
    let step = Rational::new(1, 7);
    let mut t = Rational::ZERO;
    while t < Rational::int(8) {
        parts.extend(p.query(t, t + step));
        t = t + step;
    }
    assert_eq!(whole, parts);
}

#[test]
fn randomness_depends_on_seed_only() {
    let hits = |seed: u64| {
        let p = compile(&format!("seed {seed}\nhat ????????????????\nlength 16\n")).unwrap();
        p.query(Rational::ZERO, Rational::int(16))
            .into_iter()
            .map(|e| e.start)
            .collect::<Vec<_>>()
    };
    assert_eq!(hits(1), hits(1));
    assert_ne!(hits(1), hits(2));
    let n = hits(1).len();
    assert!((80..=176).contains(&n), "{n} golpes de 256 con p = 0.5");
}

#[test]
fn diagnostics_point_at_the_problem() {
    let err = compile("bpm 195\nbass C1 G0\n").unwrap_err();
    assert_eq!(err[0].message, "la pista `bass` no tiene sonido");
    assert_eq!((err[0].pos.line, err[0].pos.col), (2, 1));

    let err = compile("kick x.z.\n").unwrap_err();
    assert_eq!((err[0].pos.col, err[0].pos.len), (8, 1));
}

#[test]
fn semantic_errors() {
    assert_eq!(
        errors("kick x C1"),
        ["el patrón de `kick` mezcla golpes y notas"]
    );
    assert_eq!(errors("kick C1"), ["`kick` suena con golpes, no con notas"]);
    assert_eq!(
        errors("lead = synth\nlead x.x."),
        ["`lead` es un synth: su patrón necesita notas"]
    );
    assert_eq!(errors("bpm 0"), ["`bpm` debe ser un tempo entre 20 y 1000"]);
    assert_eq!(errors("kick x dist 3"), ["`dist` debe estar entre 0 y 1"]);
    assert_eq!(
        errors("drop 0:\n  kick x"),
        ["los ciclos de una sección deben ser un entero mayor que 0"]
    );
}

#[test]
fn syntax_errors() {
    assert_eq!(errors("kick [x x"), ["falta cerrar este `[`"]);
    assert_eq!(errors("kick x.x.*2"), ["`*` repite un solo paso"]);
    assert_eq!(errors("  kick x"), ["indentación inesperada"]);
    assert_eq!(errors("bass = organ"), ["sonido desconocido `organ`"]);
    assert_eq!(
        errors("drop:\n  kick x\n"),
        ["cabecera de sección inválida"]
    );
}

#[test]
fn windows_files_compile() {
    // BOM de UTF-8 y saltos de línea CRLF, como guarda el Bloc de notas.
    let p = compile("\u{feff}bpm 150\r\nintro 2:\r\n  kick x.x.\r\n").unwrap();
    assert_eq!(p.bpm, 150.0);
    assert_eq!(p.length(), 2);
}

#[test]
fn piece_metadata() {
    let p = compile(
        "title \"Rotterdam\"
artist \"Isometrical\"
kick x...
",
    )
    .unwrap();
    assert_eq!(p.title.as_deref(), Some("Rotterdam"));
    assert_eq!(p.artist.as_deref(), Some("Isometrical"));

    // Sin metadatos la pieza sigue siendo válida.
    assert_eq!(compile("kick x...").unwrap().title, None);

    assert_eq!(
        errors("title Rotterdam"),
        ["`title` necesita un texto entre comillas"]
    );
    assert_eq!(errors("title \"\""), ["`title` no puede estar vacío"]);
    assert_eq!(
        errors(
            "title \"a\"
title \"b\""
        ),
        ["`title` está definido dos veces"]
    );
}

#[test]
fn stereo_and_fx_modifiers() {
    let p = compile(
        "lead = synth saw lpf 800 hpf 120 delay 0.4 reverb 0.5 gain 0.7 pan -0.3
lead C4 C4
",
    )
    .unwrap();
    let track = &p.tracks[0];
    assert_eq!(track.fx.lpf, Some(800.0));
    assert_eq!(track.fx.hpf, Some(120.0));
    assert_eq!((track.fx.delay, track.fx.reverb), (0.4, 0.5));
    assert_eq!((track.params.gain, track.params.pan), (0.7, -0.3));
    // Los eventos llevan el pan, porque se aplica voz a voz.
    assert!(
        p.query(Rational::ZERO, Rational::ONE)
            .iter()
            .all(|e| e.pan == -0.3)
    );
}

#[test]
fn a_pattern_line_overrides_only_the_voice() {
    let p = compile(
        "hat x.x. gain 0.5 pan 1
kick x...
",
    )
    .unwrap();
    let hat = p.query(Rational::ZERO, Rational::ONE);
    let hat = hat
        .iter()
        .find(|e| p.tracks[e.track].name == "hat")
        .unwrap();
    assert_eq!((hat.gain, hat.pan), (0.5, 1.0));

    // Un efecto en la línea de patrón se rechaza, y la ayuda dice dónde va.
    let err = compile(
        "lead = synth saw
lead C4 reverb 0.5
",
    )
    .unwrap_err();
    assert_eq!(
        err[0].message,
        "`reverb` se aplica a toda la pista, no a una línea"
    );
    assert!(
        err[0]
            .help
            .as_ref()
            .unwrap()
            .contains("synth saw reverb 0.5")
    );
}

#[test]
fn fx_ranges_are_checked() {
    assert_eq!(
        errors("l = synth saw lpf 10"),
        ["`lpf` debe estar entre 20 y 20000"]
    );
    assert_eq!(
        errors("l = synth saw reverb 2"),
        ["`reverb` debe estar entre 0 y 1"]
    );
    assert_eq!(errors("hat x pan -2"), ["`pan` debe estar entre -1 y 1"]);
}

#[test]
fn absurd_pieces_are_rejected_not_rendered() {
    // Un tempo diminuto pedía gigabytes al renderizar y tumbaba el proceso.
    assert_eq!(
        errors(
            "bpm 0.01
kick x...
"
        ),
        ["`bpm` debe ser un tempo entre 20 y 1000"]
    );
    let err = compile(
        "bpm 20
beats 32
length 10000
kick x...
",
    )
    .unwrap_err();
    assert!(
        err[0].message.starts_with("la pieza duraría"),
        "{}",
        err[0].message
    );

    // Una pieza larga pero razonable sigue valiendo: 250 ciclos de 12 s
    // son 50 minutos.
    assert!(
        compile(
            "bpm 20
beats 4
length 250
kick x...
"
        )
        .is_ok()
    );
}

#[test]
fn randomness_ignores_the_other_tracks() {
    // Añadir una pista no puede cambiar el azar de las demás: en directo eso
    // haría que tocar una línea alterase el resto de la sección.
    let hats = |src: &str| {
        let p = compile(src).unwrap();
        let name_of = |e: &cicada_core::Event| p.tracks[e.track].name.clone();
        p.query(Rational::ZERO, Rational::int(8))
            .into_iter()
            .filter(|e| name_of(e) == "hat")
            .map(|e| e.start)
            .collect::<Vec<_>>()
    };
    let solo = hats(
        "seed 5
length 8
intro 8:
  hat x?x?x?x?
",
    );
    let con_mas = hats(
        "seed 5
length 8
intro 8:
  kick x...
  hat x?x?x?x?
  snare ..x.
",
    );
    assert_eq!(solo, con_mas);
    assert!(!solo.is_empty());
}

#[test]
fn comments_and_blank_lines_do_not_change_the_sound() {
    let base = compile(
        "bpm 190
kick x.x.
hat  ..x.
",
    )
    .unwrap();

    // Un comentario, una línea en blanco o cambiar el título mueven las
    // posiciones del código pero no el audio.
    let retocado = compile(
        "# un comentario
bpm 190

kick x.x.
hat  ..x.
",
    )
    .unwrap();
    assert!(base.sounds_like(&retocado));
    let con_titulo = compile(
        "title \"Algo\"
bpm 190
kick x.x.
hat  ..x.
",
    )
    .unwrap();
    assert!(base.sounds_like(&con_titulo));

    // Cualquier cambio real sí cuenta.
    for distinto in [
        "bpm 191
kick x.x.
hat  ..x.
",
        "bpm 190
kick x.xx
hat  ..x.
",
        "bpm 190
kick x.x. gain 0.5
hat  ..x.
",
        "seed 1
bpm 190
kick x.x.
hat  ..x.
",
    ] {
        assert!(!base.sounds_like(&compile(distinto).unwrap()), "{distinto}");
    }
}
