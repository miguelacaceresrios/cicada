//! `import`: una paleta de sonidos compartida entre las piezas de un disco.

use std::collections::HashMap;

use cicada_core::{Diagnostic, Program, compile_with};

/// Compila `main` con una colección de archivos en memoria.
fn compile(main: &str, files: &[(&str, &str)]) -> Result<Program, Vec<Diagnostic>> {
    let files: HashMap<&str, &str> = files.iter().copied().collect();
    compile_with(main, &mut |path| {
        files
            .get(path)
            .map(|s| s.to_string())
            .ok_or_else(|| format!("no está en esta prueba: {path}"))
    })
}

fn errors(main: &str, files: &[(&str, &str)]) -> Vec<Diagnostic> {
    compile(main, files).unwrap_err()
}

const PALETA: &str = "golpe = kick dist 0.9\nsub = synth sine gain 0.5 lpf 200\n";

#[test]
fn a_palette_brings_its_sounds() {
    let p = compile(
        "import \"paleta.cicada\"\nbpm 200\ngolpe x...\nsub C1 ~\n",
        &[("paleta.cicada", PALETA)],
    )
    .unwrap();
    let golpe = p.tracks.iter().find(|t| t.name == "golpe").unwrap();
    assert_eq!(golpe.params.dist, 0.9);
    let sub = p.tracks.iter().find(|t| t.name == "sub").unwrap();
    assert_eq!(sub.fx.lpf, Some(200.0));
}

#[test]
fn the_piece_can_override_nothing_but_can_add() {
    // Declarar de nuevo un sonido de la paleta es un choque, y se avisa.
    let err = errors(
        "import \"paleta.cicada\"\ngolpe = snare\ngolpe x...\n",
        &[("paleta.cicada", PALETA)],
    );
    assert_eq!(err[0].message, "`golpe` ya tiene un sonido declarado");

    // Añadir sonidos propios sí vale.
    let p = compile(
        "import \"paleta.cicada\"\nmio = synth saw\ngolpe x...\nmio C3 ~\n",
        &[("paleta.cicada", PALETA)],
    )
    .unwrap();
    assert_eq!(p.tracks.len(), 3);
}

#[test]
fn a_palette_holds_only_sounds() {
    let err = errors(
        "import \"p.cicada\"\nkick x...\n",
        &[("p.cicada", "a = kick\nbpm 200\ndrop 4:\n  a x...\n")],
    );
    assert_eq!(err.len(), 2, "el ajuste y la sección");
    assert!(err.iter().all(|d| d.file.as_deref() == Some("p.cicada")));
    assert_eq!(
        err[0].message,
        "un archivo importado solo puede declarar sonidos"
    );
}

#[test]
fn errors_point_at_the_file_they_are_in() {
    let err = errors(
        "import \"p.cicada\"\nkick x...\n",
        &[("p.cicada", "a = kick\nb = synt saw\n")],
    );
    assert_eq!(err[0].message, "sonido desconocido `synt`");
    assert_eq!(err[0].file.as_deref(), Some("p.cicada"));
    assert_eq!(err[0].pos.line, 2, "la línea es la del archivo importado");
}

#[test]
fn a_palette_can_import_another() {
    let p = compile(
        "import \"b.cicada\"\ngolpe x...\nextra C2 ~\n",
        &[
            ("b.cicada", "import \"paleta.cicada\"\nextra = synth saw\n"),
            ("paleta.cicada", PALETA),
        ],
    )
    .unwrap();
    assert_eq!(p.tracks.len(), 3);
}

#[test]
fn circles_are_caught() {
    let err = errors(
        "import \"a.cicada\"\nkick x...\n",
        &[
            ("a.cicada", "import \"b.cicada\"\nx = kick\n"),
            ("b.cicada", "import \"a.cicada\"\ny = snare\n"),
        ],
    );
    assert!(
        err[0].message.contains("se importa a sí mismo"),
        "{}",
        err[0].message
    );
}

#[test]
fn a_missing_file_says_so() {
    let err = errors("import \"fantasma.cicada\"\nkick x...\n", &[]);
    assert_eq!(err[0].message, "no se pudo importar `fantasma.cicada`");
    assert_eq!(err[0].pos.line, 1);
}

#[test]
fn without_a_loader_an_import_explains_itself() {
    // `compile` no lee archivos a propósito; el error lo dice en vez de
    // fallar con algo sobre sintaxis.
    let err = cicada_core::compile("import \"p.cicada\"\nkick x...\n").unwrap_err();
    assert_eq!(err[0].message, "no se pudo importar `p.cicada`");
}
