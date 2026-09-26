//! Resolución de `import`.
//!
//! Un archivo importado es una **paleta**: solo declara sonidos. La idea es
//! que los tracks de un disco compartan el mismo kick y el mismo bajo sin
//! copiarlos a mano en cada archivo.
//!
//! Se resuelve sobre el AST, antes del análisis semántico, así que el resto
//! del núcleo no se entera de que los imports existen.

use std::collections::HashSet;

use crate::ast::{File, Item};
use crate::diag::Diagnostic;
use crate::parser;

/// Hasta dónde puede encadenarse una paleta que importa a otra. Es un tope de
/// cordura: un disco entero no necesita más de dos o tres niveles.
const MAX_DEPTH: usize = 8;

/// Lee el contenido de un archivo importado, o explica por qué no puede.
///
/// La ruta llega tal cual se escribió, y quien implemente esto decide contra
/// qué la resuelve: el núcleo no toca el disco.
pub type Loader<'a> = &'a mut dyn FnMut(&str) -> Result<String, String>;

/// Sustituye cada `import` por los sonidos que declara el archivo.
pub fn resolve(file: File, load: Loader<'_>) -> Result<File, Vec<Diagnostic>> {
    let mut state = Resolver {
        load,
        visiting: HashSet::new(),
        diags: Vec::new(),
    };
    let items = state.expand(file.items, 0, None);
    if state.diags.is_empty() {
        Ok(File { items })
    } else {
        Err(state.diags)
    }
}

struct Resolver<'a> {
    load: Loader<'a>,
    /// Rutas en la cadena actual, para detectar que una paleta se importa a
    /// sí misma dando vueltas.
    visiting: HashSet<String>,
    diags: Vec<Diagnostic>,
}

impl Resolver<'_> {
    /// `from` es el archivo que contiene estos items, o `None` si es el principal.
    fn expand(&mut self, items: Vec<Item>, depth: usize, from: Option<&str>) -> Vec<Item> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Item::Import(path) = item else {
                if let Some(origin) = from
                    && !matches!(item, Item::Sound(_))
                {
                    self.diags.push(
                        Diagnostic::new(
                            item_pos(&item),
                            "un archivo importado solo puede declarar sonidos",
                        )
                        .in_file(origin)
                        .with_help(
                            "quita de aquí los ajustes, las secciones y los patrones: \
                             una paleta es solo la lista de sonidos",
                        ),
                    );
                }
                out.push(item);
                continue;
            };

            if depth >= MAX_DEPTH {
                self.report(
                    Diagnostic::new(path.pos, "demasiados imports encadenados")
                        .with_help("una paleta puede importar otra, pero no en cadenas tan largas"),
                    from,
                );
                continue;
            }
            if !self.visiting.insert(path.value.clone()) {
                self.report(
                    Diagnostic::new(
                        path.pos,
                        format!("`{}` se importa a sí mismo, dando la vuelta", path.value),
                    )
                    .with_help("revisa la cadena de imports: hay un círculo"),
                    from,
                );
                continue;
            }

            match (self.load)(&path.value) {
                Ok(text) => match parser::parse(&text) {
                    Ok(inner) => {
                        let expanded = self.expand(inner.items, depth + 1, Some(&path.value));
                        out.extend(expanded);
                    }
                    Err(inner) => {
                        // Los errores llevan el nombre del archivo donde están,
                        // no el del que lo importa.
                        self.diags
                            .extend(inner.into_iter().map(|d| d.in_file(&path.value)));
                    }
                },
                Err(why) => self.report(
                    Diagnostic::new(path.pos, format!("no se pudo importar `{}`", path.value))
                        .with_help(why),
                    from,
                ),
            }
            self.visiting.remove(&path.value);
        }
        out
    }

    fn report(&mut self, diag: Diagnostic, from: Option<&str>) {
        self.diags.push(match from {
            Some(file) => diag.in_file(file),
            None => diag,
        });
    }
}

fn item_pos(item: &Item) -> crate::diag::Pos {
    match item {
        Item::Setting { name, .. } | Item::Meta { name, .. } => name.pos,
        Item::Sound(decl) => decl.track.pos,
        Item::Section(decl) => decl.name.pos,
        Item::Pattern(line) => line.track.pos,
        Item::Import(path) => path.pos,
    }
}
