use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use cicada_audio::player::Player;
use cicada_core::{Diagnostic, Program, SectionKind};
use cicada_visual::palette::role_color;
use cicada_visual::timeline::Role;
use cicada_visual::{FrameState, Options, Visualizer};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, TextureHandle,
    TextureOptions, vec2,
};

use crate::jobs::{self, Export, Loaded};
use crate::theme::{self, ACCENT, DIM, OK};
use crate::{editor, screenshot::Screenshot};

const PREVIEW: [u32; 2] = [960, 540];
/// Espera tras la última tecla antes de recompilar.
const COMPILE_DELAY: f64 = 0.25;
const NEW_FILE: &str = "bpm 180\nseed 1\n\nkick x...x...x...x...\nhat  ..x...x...x...x.\n";

enum Pending {
    Close,
    New,
    Open,
}

enum Dialog {
    Unsaved(Pending),
    NoFfmpeg,
}

pub struct Studio {
    path: Option<PathBuf>,
    source: String,
    saved: String,

    /// Texto de la última compilación y sus errores.
    compiled: String,
    edited_at: f64,
    diagnostics: Vec<Diagnostic>,
    generation: u64,
    rendering: Option<Receiver<Result<Loaded, String>>>,

    /// La pieza que suena ahora (puede ir por detrás del texto si hay errores).
    program: Option<Program>,
    /// El render que está sonando, para poder recargar las visuales sin
    /// volver a calcular el audio.
    audio: Option<Arc<[f32]>>,
    loaded_source: String,
    loaded_generation: u64,

    player: Option<Player>,
    sample_rate: u32,
    vis: Option<Visualizer>,
    preview: Option<TextureHandle>,
    logo: Option<TextureHandle>,
    state: Option<FrameState>,
    drawn: Option<(u64, f64)>,

    export: Option<Export>,
    message: Option<(String, bool)>,
    dialog: Option<Dialog>,
    allow_close: bool,
    title: String,
    screenshot: Option<Screenshot>,
}

impl Studio {
    pub fn new(file: Option<PathBuf>, screenshot: Option<Screenshot>) -> Studio {
        let (player, message) = match Player::new() {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some((format!("Sin salida de audio: {e}"), true))),
        };
        let mut studio = Studio {
            path: None,
            source: String::new(),
            saved: String::new(),
            compiled: String::new(),
            edited_at: f64::NEG_INFINITY,
            diagnostics: Vec::new(),
            generation: 0,
            rendering: None,
            program: None,
            audio: None,
            loaded_source: String::new(),
            loaded_generation: 0,
            sample_rate: player
                .as_ref()
                .map_or(jobs::EXPORT_SAMPLE_RATE, Player::sample_rate),
            player,
            vis: None,
            preview: None,
            logo: None,
            state: None,
            drawn: None,
            export: None,
            message,
            dialog: None,
            allow_close: false,
            title: String::new(),
            screenshot,
        };
        match file.or_else(crate::default_file) {
            Some(path) => studio.open_path(&path),
            None => studio.new_file(),
        }
        studio
    }

    fn dirty(&self) -> bool {
        self.source != self.saved
    }

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || "sin título.cicada".into(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    fn base_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    fn time(&self) -> f64 {
        self.player.as_ref().map_or(0.0, Player::position)
    }

    fn playing(&self) -> bool {
        self.player.as_ref().is_some_and(Player::is_playing)
    }

    // ── Archivos ──────────────────────────────────────────────────────────

    fn reset_document(&mut self, path: Option<PathBuf>, text: String) {
        self.path = path;
        self.source = text.clone();
        self.saved = text;
        self.compiled.clear();
        self.edited_at = f64::NEG_INFINITY;
        if let Some(p) = &self.player {
            p.pause();
            p.seek(0.0);
        }
    }

    fn open_path(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.reset_document(Some(path.to_path_buf()), text);
                self.info(format!("Abierto {}", path.display()));
            }
            Err(e) => self.error(format!("No se pudo abrir {}: {e}", path.display())),
        }
    }

    fn new_file(&mut self) {
        self.reset_document(None, NEW_FILE.into());
        self.saved.clear();
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("CICADA", &["cicada"]);
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_path(&path);
        }
    }

    /// Guarda; si el archivo es nuevo, pregunta dónde. `false` si se canceló.
    fn save(&mut self) -> bool {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("CICADA", &["cicada"])
                    .set_file_name("pieza.cicada");
                if let Some(dir) = crate::documents_dir() {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        match std::fs::write(&path, &self.source) {
            Ok(()) => {
                self.saved = self.source.clone();
                self.info(format!("Guardado {}", path.display()));
                self.path = Some(path);
                true
            }
            Err(e) => {
                self.error(format!("No se pudo guardar: {e}"));
                false
            }
        }
    }

    fn request(&mut self, action: Pending) {
        if self.dirty() {
            self.dialog = Some(Dialog::Unsaved(action));
        } else {
            self.run(action);
        }
    }

    fn run(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_file(),
            Pending::Open => self.open_dialog(),
            Pending::Close => self.allow_close = true,
        }
    }

    // ── Compilación y recarga en caliente ────────────────────────────────

    fn compile(&mut self, ctx: &egui::Context) {
        self.compiled = self.source.clone();
        match cicada_core::compile(&self.source) {
            Ok(program) => {
                self.diagnostics.clear();
                self.generation += 1;
                // Si la pieza suena igual que la que ya está cargada, basta
                // con refrescar las visuales: renderizar de nuevo un minuto
                // de música por retocar un comentario se nota al escribir.
                if let (Some(current), Some(audio)) = (&self.program, self.audio.clone())
                    && current.sounds_like(&program)
                {
                    let source = self.source.clone();
                    self.apply(
                        ctx,
                        Loaded {
                            generation: self.generation,
                            program,
                            source,
                            audio,
                        },
                    );
                    return;
                }
                self.rendering = Some(jobs::render_preview(
                    ctx,
                    self.generation,
                    program,
                    self.source.clone(),
                    self.sample_rate,
                    self.base_dir(),
                ));
            }
            Err(diags) => self.diagnostics = diags,
        }
    }

    fn poll_render(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rendering else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.rendering = None;
        match result {
            Ok(loaded) if loaded.generation == self.generation => self.apply(ctx, loaded),
            Ok(_) => {}
            Err(e) => self.error(e),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, loaded: Loaded) {
        let reused = self
            .audio
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(a, &loaded.audio));
        if let Some(p) = &self.player
            && !reused
        {
            p.set_buffer(loaded.audio.clone());
        }
        self.audio = Some(loaded.audio.clone());
        let name = self.name();
        match &mut self.vis {
            Some(vis) => vis.load(
                &loaded.program,
                &loaded.source,
                &name,
                &loaded.audio,
                self.sample_rate,
            ),
            None => {
                let options = Options {
                    width: PREVIEW[0],
                    height: PREVIEW[1],
                    fps: 60,
                    code: false,
                };
                match Visualizer::new(
                    &loaded.program,
                    &loaded.source,
                    &name,
                    &loaded.audio,
                    self.sample_rate,
                    options,
                ) {
                    Ok(vis) => self.vis = Some(vis),
                    Err(e) => self.error(format!("Vista previa desactivada: {e}")),
                }
            }
        }
        self.program = Some(loaded.program);
        self.loaded_source = loaded.source;
        self.loaded_generation = loaded.generation;
        self.drawn = None;
        ctx.request_repaint();
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        let t = self.time();
        let Some(vis) = &mut self.vis else { return };
        if self.drawn == Some((self.loaded_generation, t)) {
            return;
        }
        let state = vis.state_at(t);
        match vis.render_state(&state) {
            Ok(rgba) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [PREVIEW[0] as usize, PREVIEW[1] as usize],
                    rgba,
                );
                match &mut self.preview {
                    Some(tex) => tex.set(image, TextureOptions::LINEAR),
                    None => {
                        self.preview =
                            Some(ctx.load_texture("preview", image, TextureOptions::LINEAR))
                    }
                }
            }
            Err(e) => {
                self.error(e.to_string());
                self.vis = None;
            }
        }
        self.state = Some(state);
        self.drawn = Some((self.loaded_generation, t));
    }

    // ── Transporte y exportación ─────────────────────────────────────────

    fn toggle_play(&mut self) {
        if let Some(p) = &self.player {
            if p.is_playing() { p.pause() } else { p.play() }
        }
    }

    fn stop(&mut self) {
        if let Some(p) = &self.player {
            p.pause();
            p.seek(0.0);
        }
    }

    fn export_wav(&mut self, ctx: &egui::Context) {
        let Some(program) = self.program.clone() else {
            return;
        };
        let stem = self.name().trim_end_matches(".cicada").to_string();
        let Some(out) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_directory(self.base_dir())
            .set_file_name(format!("{stem}.wav"))
            .save_file()
        else {
            return;
        };
        self.export = Some(jobs::export_wav(ctx, program, self.base_dir(), out));
    }

    fn export_video(&mut self, ctx: &egui::Context) {
        let Some(program) = self.program.clone() else {
            return;
        };
        let Some(ffmpeg) = jobs::find_ffmpeg() else {
            self.dialog = Some(Dialog::NoFfmpeg);
            return;
        };
        let name = self.name();
        let stem = name.trim_end_matches(".cicada").to_string();
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Vídeo MP4", &["mp4"])
            .set_directory(self.base_dir())
            .set_file_name(format!("{stem}.mp4"))
            .save_file()
        else {
            return;
        };
        let job = jobs::VideoJob {
            program,
            source: self.loaded_source.clone(),
            name,
            base_dir: self.base_dir(),
            out,
            ffmpeg,
        };
        self.export = Some(jobs::export_video(ctx, job));
    }

    fn poll_export(&mut self) {
        let Some(export) = &self.export else { return };
        let Ok(result) = export.result.try_recv() else {
            return;
        };
        self.export = None;
        match result {
            Ok(path) => self.info(format!("Exportado {}", path.display())),
            Err(e) => self.error(format!("La exportación falló: {e}")),
        }
    }
}

impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);

        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Unsaved(Pending::Close));
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.request(Pending::Open);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.toggle_play();
        }

        if self.source != self.compiled && now - self.edited_at >= COMPILE_DELAY {
            self.compile(&ctx);
        }
        self.poll_render(&ctx);
        self.poll_export();
        self.update_preview(&ctx);
        if let Some(shot) = &mut self.screenshot
            && shot.step(
                &ctx,
                self.player.as_ref(),
                self.program.is_some() || !self.diagnostics.is_empty(),
            )
        {
            self.screenshot = None;
        }

        let title = format!(
            "{}{} — CICADA Studio",
            self.name(),
            if self.dirty() { " ●" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        self.toolbar(ui, &ctx);
        self.status_bar(ui);
        self.editor_panel(ui, now);
        egui::CentralPanel::default_margins().show(ui, |ui| self.stage(ui));
        self.dialogs(&ctx);

        if self.playing() || self.export.is_some() || self.screenshot.is_some() {
            // Con `request_repaint()` a secas la ventana repinta tan rápido
            // como pueda, y cada repintado dibuja un frame en la GPU: en un
            // portátil eso es ventilador para nada.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if self.rendering.is_some() || self.source != self.compiled {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

// ── Paneles ──────────────────────────────────────────────────────────────

impl Studio {
    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let logo = self.logo.get_or_insert_with(|| {
                    let image = crate::icon_image();
                    ctx.load_texture("logo", image, TextureOptions::LINEAR)
                });
                ui.image((logo.id(), vec2(24.0, 24.0)));
                ui.label(
                    RichText::new("CICADA")
                        .family(theme::bold())
                        .size(18.0)
                        .color(ACCENT),
                );
                ui.label(RichText::new("Studio").size(15.0).color(DIM));
                ui.separator();

                if ui.button("Nuevo").clicked() {
                    self.request(Pending::New);
                }
                if ui.button("Abrir…").on_hover_text("Ctrl+O").clicked() {
                    self.request(Pending::Open);
                }
                if ui.button("Guardar").on_hover_text("Ctrl+S").clicked() {
                    self.save();
                }
                ui.separator();

                let ready = self.player.is_some() && self.program.is_some();
                let label = if self.playing() {
                    "⏸  Pausa"
                } else {
                    "▶  Reproducir"
                };
                let play = egui::Button::new(RichText::new(label).strong())
                    .fill(Color32::from_rgb(110, 24, 20));
                if ui.add_enabled(ready, play).on_hover_text("F5").clicked() {
                    self.toggle_play();
                }
                if ui
                    .add_enabled(ready, egui::Button::new("⏹"))
                    .on_hover_text("Parar y volver al inicio")
                    .clicked()
                {
                    self.stop();
                }
                let duration = self.player.as_ref().map_or(0.0, Player::duration);
                ui.label(
                    RichText::new(format!("{} / {}", clock(self.time()), clock(duration)))
                        .monospace(),
                );
                if self.rendering.is_some() {
                    ui.spinner();
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let can_export = self.program.is_some() && self.export.is_none();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Exportar vídeo…"))
                        .clicked()
                    {
                        self.export_video(ctx);
                    }
                    if ui
                        .add_enabled(can_export, egui::Button::new("Exportar WAV…"))
                        .clicked()
                    {
                        self.export_wav(ctx);
                    }
                });
            });
            ui.add_space(4.0);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(export) = &self.export {
                    let p = export.progress();
                    ui.add(egui::ProgressBar::new(p).desired_width(240.0).text(format!(
                        "{} · {:.0}%",
                        export.label,
                        p * 100.0
                    )));
                }
                match &self.message {
                    Some((text, true)) => ui.label(RichText::new(text).color(ACCENT)),
                    Some((text, false)) => ui.label(RichText::new(text).color(DIM)),
                    None => ui.label(""),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("F5 reproducir · Ctrl+S guardar")
                            .color(DIM)
                            .small(),
                    );
                });
            });
        });
    }

    fn editor_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        egui::Panel::left("editor")
            .resizable(true)
            .default_size(640.0)
            .size_range(380.0..=1100.0)
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.name()).family(theme::bold()).size(15.0));
                    if self.dirty() {
                        ui.label(RichText::new("sin guardar").color(DIM).small());
                    }
                });
                ui.add_space(4.0);

                let diag_height = if self.diagnostics.is_empty() {
                    30.0
                } else {
                    34.0 + 42.0 * self.diagnostics.len().min(4) as f32
                };
                let editor_height = (ui.available_height() - diag_height).max(120.0);

                // Los pasos se iluminan solo si el texto es el que está sonando.
                let active = match &self.state {
                    Some(s) if self.source == self.loaded_source => s.active.clone(),
                    _ => Vec::new(),
                };
                let errors = &self.diagnostics;
                let font = FontId::monospace(15.0);
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                    let job = editor::layout_job(text.as_str(), font.clone(), &active, errors);
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                let frame = egui::Frame::new()
                    .fill(theme::EDITOR_BG)
                    .inner_margin(10.0)
                    .corner_radius(6.0);
                let id = egui::Id::new("code-edit");
                // Línea del cursor, para resaltar su número en el margen.
                let current = egui::TextEdit::load_state(ui.ctx(), id)
                    .and_then(|state| state.cursor.char_range())
                    .map(|range| {
                        let at = range.primary.index.0.min(self.source.chars().count());
                        self.source.chars().take(at).filter(|c| *c == '\n').count() + 1
                    });
                frame.show(ui, |ui| {
                    egui::ScrollArea::both()
                        .id_salt("code")
                        .max_height(editor_height - 20.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                let job = editor::gutter_job(
                                    &self.source,
                                    font.clone(),
                                    &self.diagnostics,
                                    current,
                                );
                                ui.add(egui::Label::new(job).selectable(false));
                                let edit = egui::TextEdit::multiline(&mut self.source)
                                    .id(id)
                                    .code_editor()
                                    .frame(egui::Frame::new())
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(30)
                                    .layouter(&mut layouter);
                                if ui.add(edit).changed() {
                                    self.edited_at = now;
                                }
                            });
                        });
                });

                ui.add_space(6.0);
                if self.diagnostics.is_empty() {
                    let (text, color) = if self.source == self.compiled {
                        ("Sin errores", OK)
                    } else {
                        ("Compilando…", DIM)
                    };
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, color);
                        ui.label(RichText::new(text).color(color));
                    });
                } else {
                    let n = self.diagnostics.len();
                    ui.label(
                        RichText::new(format!("{n} error{}", if n == 1 { "" } else { "es" }))
                            .color(ACCENT)
                            .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("diags")
                        .max_height(42.0 * 4.0)
                        .show(ui, |ui| {
                            for d in &self.diagnostics {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(format!("{}:{}", d.pos.line, d.pos.col))
                                            .monospace()
                                            .color(DIM),
                                    );
                                    ui.label(&d.message);
                                });
                                if let Some(help) = &d.help {
                                    ui.label(RichText::new(format!("ayuda: {help}")).color(DIM));
                                }
                            }
                        });
                }
            });
    }

    /// Vista previa, línea de tiempo de secciones y medidores.
    fn stage(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let height = (width * 9.0 / 16.0)
            .min(ui.available_height() - 260.0)
            .max(120.0);
        let size = vec2(height * 16.0 / 9.0, height);

        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let image_rect = egui::Rect::from_center_size(rect.center(), size);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::BLACK);
        match &self.preview {
            Some(tex) => {
                painter.image(
                    tex.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                let text = if self.program.is_none() {
                    "Corrige los errores para ver la pieza"
                } else {
                    "Preparando la vista previa…"
                };
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(16.0),
                    DIM,
                );
            }
        }
        if let (Some(state), Some(program)) = (&self.state, &self.program)
            && let Some(section) = program.sections.get(state.section)
        {
            let pos = image_rect.left_bottom() + vec2(16.0, -16.0);
            painter.text(
                pos,
                Align2::LEFT_BOTTOM,
                section.name.to_uppercase(),
                FontId::new(22.0, theme::bold()),
                Color32::WHITE,
            );
        }

        ui.add_space(10.0);
        self.timeline(ui);
        ui.add_space(10.0);
        self.meters(ui);
        ui.add_space(12.0);
        self.tracks(ui);
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 21, 22));
        let Some(program) = &self.program else { return };
        let duration = self
            .player
            .as_ref()
            .map_or(program.duration_seconds(), Player::duration)
            .max(0.001);
        let spc = program.seconds_per_cycle();
        let x = |t: f64| rect.left() + (t / duration) as f32 * rect.width();

        for (i, s) in program.sections.iter().enumerate() {
            let r = egui::Rect::from_x_y_ranges(
                x(s.start as f64 * spc) + 1.0..=x((s.start + s.length) as f64 * spc) - 1.0,
                rect.y_range(),
            );
            let active = self.state.as_ref().is_some_and(|st| st.section == i);
            let base = section_color(s.kind);
            let alpha = (0.35 + 0.55 * s.energy) * if active { 1.0 } else { 0.6 };
            painter.rect_filled(r, 3.0, base.gamma_multiply(alpha));
            let bar = egui::Rect::from_min_max(
                egui::pos2(r.left(), r.bottom() - 4.0 - 14.0 * s.energy),
                egui::pos2(r.left() + 3.0, r.bottom() - 4.0),
            );
            painter.rect_filled(bar, 1.0, Color32::WHITE.gamma_multiply(0.7));
            if r.width() > 40.0 {
                painter.text(
                    r.left_top() + vec2(8.0, 6.0),
                    Align2::LEFT_TOP,
                    &s.name,
                    FontId::new(13.0, theme::bold()),
                    Color32::WHITE,
                );
                painter.text(
                    r.left_bottom() + vec2(8.0, -6.0),
                    Align2::LEFT_BOTTOM,
                    format!("{} ciclos", s.length),
                    FontId::proportional(11.0),
                    Color32::WHITE.gamma_multiply(0.7),
                );
            }
        }
        let head = x(self.time());
        painter.line_segment(
            [
                egui::pos2(head, rect.top()),
                egui::pos2(head, rect.bottom()),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );

        if let (Some(pos), Some(player)) = (response.interact_pointer_pos(), &self.player) {
            player.seek(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * duration);
        }
        response.on_hover_text("Haz clic para saltar a ese momento");
    }

    fn meters(&self, ui: &mut egui::Ui) {
        let (Some(state), Some(program)) = (&self.state, &self.program) else {
            return;
        };
        let cycle = (state.cycle.floor() as i64 + 1).min(program.length());
        let beat = (state.cycle.fract() * program.beats as f64) as u32 + 1;
        ui.label(
            RichText::new(format!(
                "ciclo {cycle}/{}   beat {beat}/{}   energía {:.2}   {} bpm",
                program.length(),
                program.beats,
                state.energy,
                program.bpm
            ))
            .monospace()
            .color(DIM),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (label, role, value) in [
                ("KICK", Role::Kick, state.kick),
                ("SNARE", Role::Snare, state.snare),
                ("HAT", Role::Hat, state.hat),
                ("NOTAS", Role::Melodic, state.melodic),
            ] {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).small().color(DIM));
                    let (rect, _) = ui.allocate_exact_size(vec2(110.0, 8.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 2.0, Color32::from_rgb(30, 27, 28));
                    let mut fill = rect;
                    fill.set_width(rect.width() * value.clamp(0.0, 1.0));
                    ui.painter()
                        .rect_filled(fill, 2.0, theme::color(role_color(role)));
                });
            }
        });
    }

    /// Qué pistas tiene la pieza, cómo suenan y en qué secciones entran.
    fn tracks(&self, ui: &mut egui::Ui) {
        let Some(program) = &self.program else { return };
        ui.label(RichText::new("PISTAS").small().color(DIM));
        egui::ScrollArea::vertical()
            .id_salt("tracks")
            .show(ui, |ui| {
                egui::Grid::new("tracks")
                    .num_columns(3)
                    .spacing(vec2(18.0, 6.0))
                    .show(ui, |ui| {
                        for (i, track) in program.tracks.iter().enumerate() {
                            let role = cicada_visual::timeline::Role::of(track);
                            ui.horizontal(|ui| {
                                let (chip, _) =
                                    ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                                ui.painter()
                                    .rect_filled(chip, 2.0, theme::color(role_color(role)));
                                ui.label(RichText::new(&track.name).family(theme::bold()));
                            });
                            ui.label(RichText::new(track.sound.describe()).monospace().color(DIM));
                            ui.label(
                                RichText::new(track.fx.describe())
                                    .monospace()
                                    .color(theme::GUTTER),
                            );
                            let sections: Vec<&str> = program
                                .sections
                                .iter()
                                .filter(|s| s.layers.iter().any(|l| l.track == i))
                                .map(|s| s.name.as_str())
                                .collect();
                            ui.label(RichText::new(sections.join(" · ")).color(DIM));
                            ui.end_row();
                        }
                    });
            });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            match dialog {
                Dialog::Unsaved(_) => {
                    ui.heading("Hay cambios sin guardar");
                    ui.label(format!("¿Guardar los cambios de {}?", self.name()));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Guardar").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Descartar").clicked() {
                            choice = Some(2);
                        }
                        if ui.button("Cancelar").clicked() {
                            choice = Some(0);
                        }
                    });
                }
                Dialog::NoFfmpeg => {
                    ui.heading("Falta FFmpeg");
                    ui.label("CICADA usa FFmpeg para codificar el vídeo. Se puede instalar con winget; al terminar, vuelve a pulsar «Exportar vídeo…».");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Instalar FFmpeg").clicked() {
                            choice = Some(3);
                        }
                        if ui.button("Cerrar").clicked() {
                            choice = Some(0);
                        }
                    });
                }
            }
        });

        let Some(choice) = choice else { return };
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match (dialog, choice) {
            (Dialog::Unsaved(action), 1) => {
                if self.save() {
                    self.run(action);
                }
            }
            (Dialog::Unsaved(action), 2) => {
                self.saved = self.source.clone();
                self.run(action);
            }
            (Dialog::NoFfmpeg, 3) => match jobs::install_ffmpeg() {
                Ok(()) => self.info("Instalando FFmpeg en una consola aparte…"),
                Err(e) => self.error(format!("No se pudo lanzar winget: {e}")),
            },
            _ => {}
        }
    }
}

fn section_color(kind: SectionKind) -> Color32 {
    match kind {
        SectionKind::Drop => Color32::from_rgb(230, 40, 32),
        SectionKind::Build => Color32::from_rgb(220, 110, 50),
        SectionKind::Break => Color32::from_rgb(70, 110, 200),
        SectionKind::Intro | SectionKind::Outro => Color32::from_rgb(140, 30, 28),
        SectionKind::Main | SectionKind::Custom => Color32::from_rgb(110, 100, 104),
    }
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0);
    format!("{}:{:04.1}", (s / 60.0) as u64, s % 60.0)
}
