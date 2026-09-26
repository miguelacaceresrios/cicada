//! Núcleo de CICADA: texto → AST → programa validado → eventos (CICADA IR).
//!
//! Este crate no sabe nada de audio, archivos ni interfaz.

pub mod ast;
pub mod diag;
pub mod import;
pub mod ir;
pub mod lexer;
pub mod note;
pub mod parser;
pub mod pattern;
pub mod program;
pub mod rng;
pub mod sema;
pub mod time;

pub use ast::Wave;
pub use diag::Diagnostic;
pub use ir::Ir;
pub use program::{Event, EventKind, Program, Section, SectionKind, Sound, Track};
pub use time::Rational;

/// Compila código CICADA hasta un programa listo para consultar.
///
/// Sin acceso a archivos: un `import` da un error que lo explica. Para
/// resolverlos, [`compile_with`].
pub fn compile(src: &str) -> Result<Program, Vec<Diagnostic>> {
    compile_with(src, &mut |path| {
        Err(format!(
            "aquí no se pueden leer archivos, y `{path}` es uno"
        ))
    })
}

/// Como [`compile`], pero resolviendo los `import` con `load`.
///
/// `load` recibe la ruta tal cual se escribió y decide contra qué la
/// resuelve; el núcleo no toca el disco a propósito.
pub fn compile_with(src: &str, load: import::Loader<'_>) -> Result<Program, Vec<Diagnostic>> {
    // El Bloc de notas de Windows puede guardar UTF-8 con BOM.
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let file = import::resolve(parser::parse(src)?, load)?;
    sema::analyze(&file)
}
