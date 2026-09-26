use std::fmt::Write;

/// Posición en el código fuente. Línea y columna empiezan en 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

impl Pos {
    /// La posición justo después de esta, para señalar algo que falta.
    pub fn after(self) -> Pos {
        Pos {
            col: self.col + self.len,
            len: 1,
            ..self
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub pos: Pos,
    pub help: Option<String>,
    /// Archivo al que pertenece `pos`, si no es el que se está compilando.
    /// Lo rellena la resolución de `import`.
    pub file: Option<String>,
}

impl Diagnostic {
    pub fn new(pos: Pos, message: impl Into<String>) -> Self {
        Diagnostic {
            message: message.into(),
            pos,
            help: None,
            file: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Marca el error como perteneciente a un archivo importado. El primero
    /// gana: un error dentro de una paleta anidada conserva su archivo real.
    pub fn in_file(mut self, file: &str) -> Self {
        self.file.get_or_insert_with(|| file.to_string());
        self
    }

    /// Formato al estilo de rustc: mensaje, ubicación, la línea y un subrayado.
    pub fn render(&self, path: &str, src: &str) -> String {
        let line = self.pos.line;
        let pad = " ".repeat(line.to_string().len());
        let mut out = String::new();
        let _ = writeln!(out, "error: {}", self.message);
        let _ = writeln!(out, "{pad}--> {path}:{line}:{}", self.pos.col);
        if let Some(text) = src.lines().nth(line.saturating_sub(1)) {
            let marker =
                " ".repeat(self.pos.col.saturating_sub(1)) + &"^".repeat(self.pos.len.max(1));
            let _ = writeln!(out, "{pad} |");
            let _ = writeln!(out, "{line} | {text}");
            let _ = writeln!(out, "{pad} | {marker}");
        }
        if let Some(help) = &self.help {
            let _ = writeln!(out, "{pad} = ayuda: {help}");
        }
        out
    }
}
