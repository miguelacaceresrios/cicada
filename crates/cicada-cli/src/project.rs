//! Carga de una pieza desde el disco, resolviendo sus `import`.
//!
//! El núcleo no lee archivos a propósito, así que la parte de disco vive
//! aquí: resolver rutas, leerlas y saber mostrar un error que ocurrió dentro
//! de una paleta importada.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use cicada_core::Program;

/// Una pieza cargada, con el texto de todos los archivos que intervinieron.
pub struct Piece {
    pub source: String,
    pub program: Program,
}

/// Compila `path` resolviendo los `import` contra su carpeta.
///
/// Devuelve los errores ya formateados y listos para imprimir: cada uno sabe
/// en qué archivo está, aunque sea uno importado.
pub fn load(path: &Path) -> Result<Piece, Vec<String>> {
    let source = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            return Err(vec![format!(
                "error: no se pudo leer {}: {e}",
                path.display()
            )]);
        }
    };
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    // Se guarda lo leído para poder enseñar la línea del error después.
    let seen = RefCell::new(HashMap::new());

    let result = cicada_core::compile_with(&source, &mut |import| {
        let full = base.join(import);
        let text =
            std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        seen.borrow_mut().insert(import.to_string(), text.clone());
        Ok(text)
    });

    match result {
        Ok(program) => Ok(Piece { source, program }),
        Err(diags) => {
            let seen = seen.into_inner();
            let main = path.display().to_string();
            Err(diags
                .iter()
                .map(|d| match &d.file {
                    // El error está dentro de una paleta: se muestra con su
                    // texto, no con el del archivo que la importa.
                    Some(file) => {
                        let text = seen.get(file).map(String::as_str).unwrap_or("");
                        d.render(&base.join(file).display().to_string(), text)
                    }
                    None => d.render(&main, &source),
                })
                .collect())
        }
    }
}
