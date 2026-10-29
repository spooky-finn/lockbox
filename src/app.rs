use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eframe::egui;
use secrecy::SecretString;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    clipboard::{Guard, SystemBoard},
    config::Config,
    entry::{Entry, Totp, TotpAlgorithm},
    generator,
    icons::{self, Icon},
    totp,
    touchid::{self, Biometrics, TouchIdError},
    vault::{KdfParams, Vault},
};

const MIN_PASSWORD_CHARS: usize = 12;
/// Room for long passwords so typing never reallocates and leaves copies behind.
const SECRET_CAPACITY: usize = 256;

fn secret_buffer() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(SECRET_CAPACITY))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn save_config(config: &Config, path: &Option<PathBuf>) -> Result<(), String> {
    match path {
        Some(path) => config
            .save(path)
            .map_err(|error| format!("Не удалось сохранить настройки: {error}")),
        None => Ok(()),
    }
}

/// A vault unlocked on a worker thread.
struct Opened {
    vault: Vault,
    with_password: bool,
}

pub struct LockboxApp {
    config: Config,
    config_path: Option<PathBuf>,
    screen: Screen,
    clipboard: Guard,
    biometrics: Arc<dyn Biometrics>,
    icons: IconCache,
}

/// Rasterised brand glyphs, one texture per icon, size and glyph colour.
#[derive(Default)]
struct IconCache(HashMap<(String, u32, bool), egui::TextureHandle>);

impl IconCache {
    fn texture(
        &mut self,
        ctx: &egui::Context,
        icon: &Icon,
        px: u32,
        dark: bool,
    ) -> Option<egui::TextureId> {
        let key = (icon.slug.clone(), px, dark);
        if let Some(texture) = self.0.get(&key) {
            return Some(texture.id());
        }
        let pixels = icons::render(icon, px, dark)?;
        let image = egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], &pixels);
        let texture = ctx.load_texture(
            format!("icon-{}-{px}-{dark}", icon.slug),
            image,
            egui::TextureOptions::LINEAR,
        );
        let id = texture.id();
        self.0.insert(key, texture);
        Some(id)
    }
}

/// A rounded square: the brand glyph on its colour, or the first letter of the title.
fn icon_tile(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    icon: Option<&Icon>,
    title: &str,
    size: f32,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), sense);
    let radius = egui::CornerRadius::same((size * 0.22) as u8);
    let painter = ui.painter();
    match icon {
        Some(icon) => {
            let [r, g, b] = icons::rgb(&icon.hex);
            let dark = icons::glyph_is_dark([r, g, b]);
            painter.rect_filled(rect, radius, egui::Color32::from_rgb(r, g, b));
            let glyph = size * 0.66;
            let px = (glyph * ui.ctx().pixels_per_point()).ceil() as u32;
            if let Some(texture) = cache.texture(ui.ctx(), icon, px.max(1), dark) {
                painter.image(
                    texture,
                    egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(glyph)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            response.on_hover_text(&icon.title)
        }
        None => {
            let [r, g, b] = icons::letter_colour(title);
            painter.rect_filled(rect, radius, egui::Color32::from_rgb(r, g, b));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icons::letter(title),
                egui::FontId::proportional(size * 0.55),
                egui::Color32::WHITE,
            );
            response
        }
    }
}

enum Screen {
    Locked(Locked),
    Unlocked(Box<Unlocked>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Open,
    Create,
}

struct Locked {
    path: Option<PathBuf>,
    mode: Mode,
    password: Zeroizing<String>,
    repeat: Zeroizing<String>,
    error: Option<String>,
    /// Argon2id and Touch ID run on a worker thread so the window keeps repainting.
    pending: Option<Receiver<Result<Opened, String>>>,
    /// Vault identifier when Touch ID can open this file.
    touch_id: Option<String>,
    /// Ask for the finger right away: only when the app starts.
    auto_touch_id: bool,
    waiting_touch_id: bool,
}

impl Locked {
    fn new(path: Option<PathBuf>) -> Self {
        let error = path
            .as_ref()
            .filter(|path| !path.exists())
            .map(|path| format!("Файл не найден: {}", path.display()));
        Self {
            path,
            mode: Mode::Open,
            password: secret_buffer(),
            repeat: secret_buffer(),
            error,
            pending: None,
            touch_id: None,
            auto_touch_id: false,
            waiting_touch_id: false,
        }
    }

    /// Offers Touch ID when it was turned on for this file and the sensor is available.
    fn offer_touch_id(mut self, config: &Config, biometrics: &dyn Biometrics, auto: bool) -> Self {
        self.touch_id = self
            .path
            .as_ref()
            .filter(|path| path.exists())
            .and_then(|path| Vault::peek_id(path).ok())
            .filter(|id| config.touch_id.contains(id))
            .filter(|_| touchid::available(biometrics));
        self.auto_touch_id = auto && self.touch_id.is_some();
        self
    }

    fn start_touch_id(&mut self, ctx: &egui::Context, biometrics: Arc<dyn Biometrics>) {
        let (Some(path), Some(id)) = (self.path.clone(), self.touch_id.clone()) else {
            return;
        };
        self.error = None;
        self.waiting_touch_id = true;
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = touchid::unlock(&*biometrics, &id, unix_now())
                .map_err(|error| error.to_string())
                .and_then(|key| Vault::open_with_key(&path, &key).map_err(|e| e.to_string()))
                .map(|vault| Opened {
                    vault,
                    with_password: false,
                });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.pending = Some(receiver);
    }

    fn submit(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.password.is_empty() {
            self.error = Some("Введите мастер-пароль".into());
            return;
        }
        if self.mode == Mode::Create && *self.password != *self.repeat {
            self.error = Some("Пароли не совпадают".into());
            return;
        }
        // Moved, not copied: the worker owns the only copy and wipes it on drop.
        let password = SecretString::from(std::mem::replace(
            &mut *self.password,
            String::with_capacity(SECRET_CAPACITY),
        ));
        self.repeat.zeroize();
        self.error = None;
        self.waiting_touch_id = false;
        let mode = self.mode;
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = match mode {
                Mode::Open => Vault::open(&path, &password),
                Mode::Create => Vault::create(&path, &password, KdfParams::default()),
            }
            .map(|vault| Opened {
                vault,
                with_password: true,
            })
            .map_err(|error| error.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.pending = Some(receiver);
    }
}

struct Unlocked {
    vault: Vault,
    search: String,
    selected: Option<String>,
    draft: Option<Draft>,
    show_password: bool,
    confirm_delete: bool,
    error: Option<String>,
    touch_id_available: bool,
    touch_id_enabled: bool,
    touch_id_pending: Option<Receiver<Result<(), String>>>,
    /// Generator settings last until the window closes; they are never saved.
    generator: generator::Options,
}

struct Draft {
    entry: Entry,
    is_new: bool,
    show_password: bool,
    show_totp: bool,
    totp_error: Option<String>,
    generator_open: bool,
    generator_error: Option<String>,
    icon_picker_open: bool,
    icon_query: String,
}

impl Draft {
    /// A pasted otpauth:// link replaces the TOTP settings and fills an empty title and login.
    fn apply_link(&mut self) {
        let Some(totp) = &self.entry.totp else {
            return;
        };
        if !totp::is_link(&totp.secret) {
            self.totp_error = None;
            return;
        }
        let text = Zeroizing::new(totp.secret.clone());
        match totp::parse_link(&text) {
            Ok(link) => {
                if self.entry.title.trim().is_empty()
                    && let Some(issuer) = &link.issuer
                {
                    self.entry.title = issuer.clone();
                }
                if self.entry.username.trim().is_empty()
                    && let Some(account) = &link.account
                {
                    self.entry.username = account.clone();
                }
                self.entry.totp = Some(link.totp);
                self.totp_error = None;
            }
            Err(error) => self.totp_error = Some(error.to_string()),
        }
    }
}

impl Unlocked {
    fn new(vault: Vault) -> Self {
        Self {
            vault,
            search: String::new(),
            selected: None,
            draft: None,
            show_password: false,
            confirm_delete: false,
            error: None,
            touch_id_available: false,
            touch_id_enabled: false,
            touch_id_pending: None,
            generator: generator::Options::default(),
        }
    }

    /// Entries matching the search, sorted by title.
    fn visible(&self) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> = self
            .vault
            .entries
            .iter()
            .filter(|entry| matches(entry, &self.search))
            .collect();
        entries.sort_by_key(|entry| entry.title.to_lowercase());
        entries
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected.as_ref()?;
        self.vault.entries.iter().find(|entry| &entry.id == id)
    }

    fn select(&mut self, id: String) {
        self.selected = Some(id);
        self.show_password = false;
        self.confirm_delete = false;
        self.error = None;
    }

    fn start_new(&mut self) {
        self.start_draft(Entry::new(), true);
    }

    fn start_edit(&mut self) {
        if let Some(entry) = self.selected_entry().cloned() {
            self.start_draft(entry, false);
        }
    }

    fn start_draft(&mut self, entry: Entry, is_new: bool) {
        self.draft = Some(Draft {
            entry,
            is_new,
            show_password: false,
            show_totp: false,
            totp_error: None,
            generator_open: false,
            generator_error: None,
            icon_picker_open: false,
            icon_query: String::new(),
        });
        self.confirm_delete = false;
        self.error = None;
    }

    /// Writes the draft to disk. On failure the vault is unchanged and the draft is kept.
    fn save_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        if draft.entry.title.trim().is_empty() {
            return Err("Укажите название записи".into());
        }
        if let Some(totp) = &draft.entry.totp {
            totp::validate(totp).map_err(|error| format!("TOTP: {error}"))?;
        }
        let mut entry = draft.entry.clone();
        if let Some(totp) = &mut entry.totp {
            totp.secret = totp::normalize_secret(&totp.secret);
        }
        let id = entry.id.clone();
        let previous = self.vault.entries.clone();
        match self.vault.entries.iter_mut().find(|e| e.id == id) {
            Some(slot) => *slot = entry,
            None => self.vault.entries.push(entry),
        }
        if let Err(error) = self.vault.save() {
            self.vault.entries = previous;
            return Err(format!("Не удалось сохранить: {error}"));
        }
        self.draft = None;
        self.select(id);
        Ok(())
    }

    fn delete_selected(&mut self) -> Result<(), String> {
        let Some(id) = self.selected.clone() else {
            return Ok(());
        };
        let previous = self.vault.entries.clone();
        self.vault.entries.retain(|entry| entry.id != id);
        if let Err(error) = self.vault.save() {
            self.vault.entries = previous;
            return Err(format!("Не удалось сохранить: {error}"));
        }
        self.selected = None;
        self.confirm_delete = false;
        Ok(())
    }
}

fn matches(entry: &Entry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [&entry.title, &entry.username, &entry.url]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
}

/// Picked files get the `.lbx` extension, so `.gitignore` always covers them.
fn with_vault_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("lbx");
    }
    path
}

impl LockboxApp {
    pub fn new() -> Self {
        let config_path = Config::default_path();
        let config = config_path.as_deref().map(Config::load).unwrap_or_default();
        let biometrics = touchid::system();
        let screen = Screen::Locked(Locked::new(config.last_vault.clone()).offer_touch_id(
            &config,
            &*biometrics,
            true,
        ));
        Self {
            config,
            config_path,
            screen,
            clipboard: Guard::new(Box::new(SystemBoard::new())),
            biometrics,
            icons: IconCache::default(),
        }
    }

    /// Drops the unlocked vault, wiping its key and entries from memory.
    fn lock(&mut self) {
        self.clipboard.clear_now();
        if let Screen::Unlocked(unlocked) = &self.screen {
            let path = unlocked.vault.path().to_path_buf();
            self.screen = Screen::Locked(Locked::new(Some(path)).offer_touch_id(
                &self.config,
                &*self.biometrics,
                false,
            ));
        }
    }

    fn remember(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        self.config.last_vault = path;
        save_config(&self.config, &self.config_path)
    }

    fn open(&mut self, opened: Opened) {
        let path = opened.vault.path().to_path_buf();
        let id = opened.vault.id();
        let enabled = self.config.touch_id.contains(&id);
        let mut errors = Vec::new();
        // Typing the password restarts the Touch ID period (and restores a dropped key).
        if opened.with_password && enabled {
            match touchid::refresh(
                &*self.biometrics,
                &id,
                &opened.vault.export_key(),
                unix_now(),
            ) {
                Ok(()) | Err(TouchIdError::Unavailable) => {}
                Err(error) => errors.push(error.to_string()),
            }
        }
        let mut unlocked = Unlocked::new(opened.vault);
        unlocked.touch_id_available = touchid::available(&*self.biometrics);
        unlocked.touch_id_enabled = enabled;
        errors.extend(self.remember(Some(path)).err());
        unlocked.error = (!errors.is_empty()).then(|| errors.join("; "));
        self.screen = Screen::Unlocked(Box::new(unlocked));
    }

    fn locked_ui(&mut self, ui: &mut egui::Ui) {
        let Screen::Locked(locked) = &mut self.screen else {
            return;
        };

        if let Some(receiver) = &locked.pending {
            match receiver.try_recv() {
                Ok(Ok(opened)) => {
                    self.open(opened);
                    return;
                }
                Ok(Err(error)) => {
                    locked.error = Some(error);
                    locked.pending = None;
                    locked.waiting_touch_id = false;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    locked.error = Some("Разблокировка прервалась".into());
                    locked.pending = None;
                }
            }
        }

        let biometrics = self.biometrics.clone();
        if locked.auto_touch_id && locked.pending.is_none() {
            locked.auto_touch_id = false;
            locked.start_touch_id(ui.ctx(), biometrics.clone());
        }
        let busy = locked.pending.is_some();
        let mut forget = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.set_max_width(380.0);
                ui.add_space(48.0);
                ui.heading("lockbox");
                ui.add_space(16.0);

                match &locked.path {
                    Some(path) => ui.label(format!("Хранилище: {}", path.display())),
                    None => ui.label("Хранилище не выбрано"),
                };
                ui.add_space(8.0);

                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Открыть хранилище…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("lockbox", &["lbx"])
                                .pick_file()
                        {
                            *locked = Locked::new(Some(path)).offer_touch_id(
                                &self.config,
                                &*biometrics,
                                false,
                            );
                        }
                        if ui.button("Создать хранилище…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("lockbox", &["lbx"])
                                .set_file_name("vault.lbx")
                                .save_file()
                        {
                            *locked = Locked::new(None);
                            locked.path = Some(with_vault_extension(path));
                            locked.mode = Mode::Create;
                        }
                        if self.config.last_vault.is_some() && ui.button("Забыть путь").clicked() {
                            forget = true;
                        }
                    });
                });
                ui.add_space(16.0);

                let ready = locked.path.as_ref().is_some_and(|path| {
                    locked.mode == Mode::Create || path.exists()
                });
                if ready && locked.mode == Mode::Open && locked.touch_id.is_some() {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Открыть по Touch ID"))
                        .clicked()
                    {
                        locked.start_touch_id(ui.ctx(), biometrics.clone());
                    }
                    ui.add_space(8.0);
                }
                if ready {
                    let mut submit = false;
                    ui.add_enabled_ui(!busy, |ui| {
                        let password = ui.add(
                            egui::TextEdit::singleline(&mut *locked.password)
                                .password(true)
                                .hint_text("Мастер-пароль")
                                .desired_width(f32::INFINITY),
                        );
                        if !busy && !password.has_focus() && locked.password.is_empty() {
                            password.request_focus();
                        }
                        submit |= password.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if locked.mode == Mode::Create {
                            let repeat = ui.add(
                                egui::TextEdit::singleline(&mut *locked.repeat)
                                    .password(true)
                                    .hint_text("Повторите мастер-пароль")
                                    .desired_width(f32::INFINITY),
                            );
                            submit |= repeat.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter));
                            let length = locked.password.chars().count();
                            if length > 0 && length < MIN_PASSWORD_CHARS {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    format!(
                                        "Пароль короче {MIN_PASSWORD_CHARS} символов — его проще подобрать"
                                    ),
                                );
                            }
                        }
                        let label = match locked.mode {
                            Mode::Open => "Открыть",
                            Mode::Create => "Создать",
                        };
                        submit |= ui.button(label).clicked();
                    });
                    if submit && !busy {
                        locked.submit(ui.ctx());
                    }
                }

                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(match locked.mode {
                            Mode::Open if locked.waiting_touch_id => "Жду Touch ID…",
                            Mode::Open => "Проверяю пароль…",
                            Mode::Create => "Создаю хранилище…",
                        });
                    });
                }
                if let Some(error) = &locked.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        });

        if forget {
            let error = self.remember(None).err();
            self.screen = Screen::Locked(Locked::new(None));
            if let Screen::Locked(locked) = &mut self.screen {
                locked.error = error;
            }
        }
    }

    fn unlocked_ui(&mut self, ui: &mut egui::Ui) {
        let Screen::Unlocked(unlocked) = &mut self.screen else {
            return;
        };
        let mut lock = false;
        let mut touch_id_wanted = None;
        let editing = unlocked.draft.is_some();

        if let Some(receiver) = &unlocked.touch_id_pending {
            match receiver.try_recv() {
                Ok(result) => {
                    unlocked.touch_id_pending = None;
                    match result {
                        Ok(()) => {
                            let id = unlocked.vault.id();
                            if !self.config.touch_id.contains(&id) {
                                self.config.touch_id.push(id);
                            }
                            unlocked.touch_id_enabled = true;
                            unlocked.error = save_config(&self.config, &self.config_path).err();
                        }
                        Err(error) => unlocked.error = Some(error),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => unlocked.touch_id_pending = None,
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("lockbox");
                let name = unlocked
                    .vault
                    .path()
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.label(name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    lock = ui.button("Заблокировать").clicked();
                    if unlocked.touch_id_available {
                        let mut wanted = unlocked.touch_id_enabled;
                        let checkbox = egui::Checkbox::new(&mut wanted, "Touch ID");
                        if ui
                            .add_enabled(unlocked.touch_id_pending.is_none(), checkbox)
                            .on_hover_text("Открывать это хранилище отпечатком пальца")
                            .changed()
                        {
                            touch_id_wanted = Some(wanted);
                        }
                    }
                    if ui
                        .add_enabled(!editing, egui::Button::new("Новая запись"))
                        .clicked()
                    {
                        unlocked.start_new();
                    }
                });
            });
        });

        egui::Panel::left("entries")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut unlocked.search)
                        .hint_text("Поиск")
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                let mut clicked = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!editing, |ui| {
                        for entry in unlocked.visible() {
                            let selected = unlocked.selected.as_ref() == Some(&entry.id);
                            let title = if entry.title.is_empty() {
                                "(без названия)"
                            } else {
                                &entry.title
                            };
                            ui.horizontal(|ui| {
                                let icon = icons::catalog().for_entry(entry);
                                icon_tile(
                                    ui,
                                    &mut self.icons,
                                    icon,
                                    title,
                                    18.0,
                                    egui::Sense::hover(),
                                );
                                if ui.selectable_label(selected, title).clicked() {
                                    clicked = Some(entry.id.clone());
                                }
                            });
                        }
                    });
                });
                if let Some(id) = clicked {
                    unlocked.select(id);
                }
            });

        if let Some(left) = self.clipboard.remaining(Instant::now()) {
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.label(format!(
                    "Скопировано · буфер очистится через {} с",
                    left.as_secs() + 1
                ));
            });
        }

        let copy = egui::CentralPanel::default()
            .show(ui, |ui| {
                if let Some(error) = &unlocked.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    ui.add_space(8.0);
                }
                if unlocked.draft.is_some() {
                    draft_ui(ui, unlocked, &mut self.icons);
                    None
                } else if unlocked.selected_entry().is_some() {
                    entry_ui(ui, unlocked, &mut self.icons)
                } else {
                    ui.label("Выберите запись слева или создайте новую.");
                    None
                }
            })
            .inner;
        match touch_id_wanted {
            Some(true) => {
                let (sender, receiver) = mpsc::channel();
                let biometrics = self.biometrics.clone();
                let id = unlocked.vault.id();
                let key = unlocked.vault.export_key();
                let ctx = ui.ctx().clone();
                thread::spawn(move || {
                    let result = touchid::enable(&*biometrics, &id, &key, unix_now())
                        .map_err(|error| format!("Не удалось включить Touch ID: {error}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                unlocked.touch_id_pending = Some(receiver);
            }
            Some(false) => {
                let id = unlocked.vault.id();
                touchid::disable(&*self.biometrics, &id);
                self.config.touch_id.retain(|known| known != &id);
                unlocked.touch_id_enabled = false;
                unlocked.error = save_config(&self.config, &self.config_path).err();
            }
            None => {}
        }

        if let Some(text) = copy {
            unlocked.error = self
                .clipboard
                .copy(&text, Instant::now())
                .err()
                .map(|error| format!("Не удалось скопировать: {error}"));
        }

        if lock {
            self.lock();
        }
    }
}

impl Default for LockboxApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for LockboxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        self.clipboard.tick(now);
        if let Some(left) = self.clipboard.remaining(now) {
            // Wake up for the countdown and for the wipe itself, even when idle.
            ui.ctx()
                .request_repaint_after(left.min(Duration::from_secs(1)));
        }
        match self.screen {
            Screen::Locked(_) => self.locked_ui(ui),
            Screen::Unlocked(_) => self.unlocked_ui(ui),
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.clipboard.clear_now();
    }
}

/// Shows the selected entry; returns text the user asked to copy.
fn entry_ui(
    ui: &mut egui::Ui,
    unlocked: &mut Unlocked,
    cache: &mut IconCache,
) -> Option<Zeroizing<String>> {
    let entry = unlocked.selected_entry()?;
    let mut copy = None;
    let title = entry.title.clone();
    let mut toggle_password = false;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    ui.horizontal(|ui| {
        let icon = icons::catalog().for_entry(entry);
        icon_tile(ui, cache, icon, &entry.title, 32.0, egui::Sense::hover());
        ui.heading(&entry.title);
    });
    ui.add_space(8.0);
    egui::Grid::new("entry")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Логин");
            ui.horizontal(|ui| {
                ui.label(&entry.username);
                if !entry.username.is_empty() && ui.small_button("Скопировать").clicked()
                {
                    copy = Some(Zeroizing::new(entry.username.clone()));
                }
            });
            ui.end_row();

            ui.label("Пароль");
            ui.horizontal(|ui| {
                if unlocked.show_password {
                    ui.monospace(&entry.password);
                } else {
                    ui.monospace("••••••••");
                }
                let label = if unlocked.show_password {
                    "Скрыть"
                } else {
                    "Показать"
                };
                toggle_password = ui.small_button(label).clicked();
                if !entry.password.is_empty() && ui.small_button("Скопировать").clicked()
                {
                    copy = Some(Zeroizing::new(entry.password.clone()));
                }
            });
            ui.end_row();

            ui.label("URL");
            ui.label(&entry.url);
            ui.end_row();

            if let Some(totp) = &entry.totp {
                ui.label("Код");
                ui.horizontal(|ui| match totp::code(totp, now.as_secs()) {
                    Ok(code) => {
                        ui.label(
                            egui::RichText::new(totp::group(&code))
                                .monospace()
                                .size(22.0),
                        );
                        let left = totp::seconds_left(totp, now.as_secs());
                        ui.add(
                            egui::ProgressBar::new(left as f32 / totp.period as f32)
                                .desired_width(120.0)
                                .text(format!("{left} с")),
                        );
                        if ui.small_button("Скопировать").clicked() {
                            copy = Some(Zeroizing::new(code));
                        }
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                });
                ui.end_row();
                // Repaint exactly when the next second starts, only while a code is shown.
                ui.ctx()
                    .request_repaint_after(Duration::from_millis(u64::from(
                        1000 - now.subsec_millis(),
                    )));
            }

            ui.label("Заметка");
            ui.label(&entry.notes);
            ui.end_row();
        });
    if toggle_password {
        unlocked.show_password = !unlocked.show_password;
    }

    ui.add_space(16.0);
    if unlocked.confirm_delete {
        ui.label(format!("Удалить запись «{title}»? Это нельзя отменить."));
        ui.horizontal(|ui| {
            if ui.button("Удалить").clicked() {
                unlocked.error = unlocked.delete_selected().err();
            }
            if ui.button("Отмена").clicked() {
                unlocked.confirm_delete = false;
            }
        });
    } else {
        ui.horizontal(|ui| {
            if ui.button("Изменить").clicked() {
                unlocked.start_edit();
            }
            if ui.button("Удалить").clicked() {
                unlocked.confirm_delete = true;
            }
        });
    }
    copy
}

fn icon_picker_ui(ui: &mut egui::Ui, draft: &mut Draft, cache: &mut IconCache) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft.icon_query)
                    .hint_text("Название сервиса")
                    .desired_width(240.0),
            );
            if ui.button("Закрыть").clicked() {
                draft.icon_picker_open = false;
            }
        });
        let found = icons::catalog().search(&draft.icon_query, 60);
        if draft.icon_query.trim().is_empty() {
            ui.label(format!(
                "Начните вводить название — в наборе {} иконок.",
                icons::catalog().len()
            ));
        } else if found.is_empty() {
            ui.label("Ничего не найдено — останется буква.");
        }
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for icon in found {
                if icon_tile(
                    ui,
                    cache,
                    Some(icon),
                    &icon.title,
                    32.0,
                    egui::Sense::click(),
                )
                .clicked()
                {
                    chosen = Some(icon.slug.clone());
                }
            }
        });
        if let Some(slug) = chosen {
            draft.entry.icon = Some(slug);
            draft.icon_picker_open = false;
        }
    });
}

fn generator_ui(ui: &mut egui::Ui, draft: &mut Draft, options: &mut generator::Options) {
    ui.group(|ui| {
        ui.add(egui::Slider::new(&mut options.length, generator::LENGTH).text("символов"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut options.lower, "a–z");
            ui.checkbox(&mut options.upper, "A–Z");
            ui.checkbox(&mut options.digits, "0–9");
            ui.checkbox(&mut options.symbols, "!#$%…");
        });
        ui.checkbox(
            &mut options.avoid_ambiguous,
            format!("без похожих символов ({})", generator::AMBIGUOUS),
        );
        let bits = options.entropy_bits();
        let verdict = match bits {
            b if b < 64.0 => "слабый",
            b if b < 100.0 => "нормальный",
            _ => "сильный",
        };
        ui.label(format!("Стойкость: ≈ {bits:.0} бит — {verdict}"));
        ui.horizontal(|ui| {
            if ui.button("Подставить").clicked() {
                match generator::generate(options) {
                    Ok(password) => {
                        // Overwrite in place so the old password does not linger.
                        draft.entry.password.zeroize();
                        draft.entry.password.push_str(&password);
                        draft.generator_error = None;
                        draft.generator_open = false;
                    }
                    Err(error) => draft.generator_error = Some(error.to_string()),
                }
            }
            if ui.button("Закрыть").clicked() {
                draft.generator_open = false;
            }
        });
        if let Some(error) = &draft.generator_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    });
}

fn algorithm_name(algorithm: TotpAlgorithm) -> &'static str {
    match algorithm {
        TotpAlgorithm::Sha1 => "SHA1",
        TotpAlgorithm::Sha256 => "SHA256",
        TotpAlgorithm::Sha512 => "SHA512",
    }
}

fn wide(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.desired_width(f32::INFINITY)
}

fn draft_ui(ui: &mut egui::Ui, unlocked: &mut Unlocked, cache: &mut IconCache) {
    let Some(draft) = &mut unlocked.draft else {
        return;
    };
    ui.heading(if draft.is_new {
        "Новая запись"
    } else {
        "Изменение записи"
    });
    ui.add_space(8.0);
    egui::Grid::new("draft")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Название");
            ui.add(wide(egui::TextEdit::singleline(&mut draft.entry.title)));
            ui.end_row();

            ui.label("Логин");
            ui.add(wide(egui::TextEdit::singleline(&mut draft.entry.username)));
            ui.end_row();

            ui.label("Пароль");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut draft.entry.password)
                        .password(!draft.show_password)
                        .desired_width(240.0),
                );
                ui.checkbox(&mut draft.show_password, "показать");
                if ui.button("Сгенерировать…").clicked() {
                    draft.generator_open = !draft.generator_open;
                }
            });
            ui.end_row();

            ui.label("URL");
            ui.add(wide(egui::TextEdit::singleline(&mut draft.entry.url)));
            ui.end_row();

            ui.label("Иконка");
            ui.horizontal(|ui| {
                let icon = icons::catalog().for_entry(&draft.entry);
                icon_tile(
                    ui,
                    cache,
                    icon,
                    &draft.entry.title,
                    20.0,
                    egui::Sense::hover(),
                );
                ui.label(match (&draft.entry.icon, icon) {
                    (Some(_), Some(icon)) => icon.title.clone(),
                    (None, Some(icon)) => format!("авто: {}", icon.title),
                    _ => "авто: буква".to_owned(),
                });
                if ui.button("Выбрать…").clicked() {
                    draft.icon_picker_open = !draft.icon_picker_open;
                }
                if draft.entry.icon.is_some() && ui.button("Авто").clicked() {
                    draft.entry.icon = None;
                }
            });
            ui.end_row();

            ui.label("Заметка");
            ui.add(wide(egui::TextEdit::multiline(&mut draft.entry.notes)));
            ui.end_row();
        });

    if draft.icon_picker_open {
        ui.add_space(8.0);
        icon_picker_ui(ui, draft, cache);
    }

    if draft.generator_open {
        ui.add_space(8.0);
        generator_ui(ui, draft, &mut unlocked.generator);
    }

    ui.add_space(12.0);
    ui.separator();
    ui.strong("TOTP");
    if draft.entry.totp.is_none() {
        if ui.button("Добавить TOTP").clicked() {
            draft.entry.totp = Some(Totp {
                secret: String::new(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        }
    } else {
        let mut remove = false;
        let mut changed = false;
        if let Some(totp) = &mut draft.entry.totp {
            ui.horizontal(|ui| {
                changed = ui
                    .add(
                        egui::TextEdit::singleline(&mut totp.secret)
                            .password(!draft.show_totp)
                            .hint_text("Секрет или ссылка otpauth://")
                            .desired_width(320.0),
                    )
                    .changed();
                ui.checkbox(&mut draft.show_totp, "показать");
            });
            egui::CollapsingHeader::new("Дополнительно").show(ui, |ui| {
                egui::ComboBox::from_label("Алгоритм")
                    .selected_text(algorithm_name(totp.algorithm))
                    .show_ui(ui, |ui| {
                        for algorithm in [
                            TotpAlgorithm::Sha1,
                            TotpAlgorithm::Sha256,
                            TotpAlgorithm::Sha512,
                        ] {
                            ui.selectable_value(
                                &mut totp.algorithm,
                                algorithm,
                                algorithm_name(algorithm),
                            );
                        }
                    });
                ui.add(
                    egui::DragValue::new(&mut totp.digits)
                        .range(totp::DIGITS)
                        .prefix("Цифр: "),
                );
                ui.add(
                    egui::DragValue::new(&mut totp.period)
                        .range(totp::PERIOD)
                        .prefix("Период: ")
                        .suffix(" с"),
                );
            });
            remove = ui.button("Убрать TOTP").clicked();
        }
        if changed {
            draft.apply_link();
        }
        if remove {
            draft.entry.totp = None;
            draft.totp_error = None;
        }
        if let Some(error) = &draft.totp_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        if ui.button("Сохранить").clicked() {
            unlocked.error = unlocked.save_draft().err();
        }
        if ui.button("Отмена").clicked() {
            unlocked.draft = None;
            unlocked.error = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touchid::tests::FakeBiometrics;

    const FAST: KdfParams = KdfParams {
        memory_kib: 8 * 1024,
        passes: 1,
        lanes: 1,
    };

    fn unlocked(dir: &tempfile::TempDir) -> Unlocked {
        let path = dir.path().join("test.lbx");
        let password = SecretString::from("master".to_owned());
        Unlocked::new(Vault::create(path, &password, FAST).unwrap())
    }

    fn reopen(unlocked: &Unlocked) -> Vault {
        Vault::open(
            unlocked.vault.path(),
            &SecretString::from("master".to_owned()),
        )
        .unwrap()
    }

    fn add(unlocked: &mut Unlocked, title: &str, username: &str, url: &str) -> String {
        unlocked.start_new();
        let draft = &mut unlocked.draft.as_mut().unwrap().entry;
        draft.title = title.into();
        draft.username = username.into();
        draft.url = url.into();
        let id = draft.id.clone();
        unlocked.save_draft().unwrap();
        id
    }

    #[test]
    fn search_matches_title_username_and_url_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        add(&mut unlocked, "GitLab", "a8ee", "https://gitlab.example");
        add(&mut unlocked, "Почта", "inbox", "https://mail.example");
        add(&mut unlocked, "банк", "client", "https://bank.example");
        let titles = |unlocked: &Unlocked| {
            unlocked
                .visible()
                .iter()
                .map(|entry| entry.title.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&unlocked), ["GitLab", "банк", "Почта"]);
        unlocked.search = "GITLAB".into();
        assert_eq!(titles(&unlocked), ["GitLab"]);
        unlocked.search = "INBOX".into();
        assert_eq!(titles(&unlocked), ["Почта"]);
        unlocked.search = "bank.example".into();
        assert_eq!(titles(&unlocked), ["банк"]);
        unlocked.search = "ПОЧ".into();
        assert_eq!(titles(&unlocked), ["Почта"]);
    }

    #[test]
    fn new_and_edited_entries_are_written_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        let id = add(&mut unlocked, "GitLab", "a8ee", "");
        assert_eq!(unlocked.selected.as_deref(), Some(id.as_str()));
        assert!(unlocked.draft.is_none());

        unlocked.start_edit();
        unlocked.draft.as_mut().unwrap().entry.password = "s3cret".into();
        unlocked.save_draft().unwrap();

        let saved = reopen(&unlocked);
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(saved.entries[0].password, "s3cret");
    }

    #[test]
    fn entry_without_title_is_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        unlocked.start_new();
        assert!(unlocked.save_draft().is_err());
        assert!(unlocked.draft.is_some());
        assert!(reopen(&unlocked).entries.is_empty());
    }

    #[test]
    fn failed_save_keeps_the_draft_and_the_vault_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        add(&mut unlocked, "GitLab", "a8ee", "");
        // The temporary file cannot be created inside a directory that is gone.
        std::fs::remove_dir_all(dir.path()).unwrap();
        unlocked.start_new();
        unlocked.draft.as_mut().unwrap().entry.title = "Почта".into();
        assert!(unlocked.save_draft().is_err());
        assert!(unlocked.draft.is_some());
        assert_eq!(unlocked.vault.entries.len(), 1);
    }

    #[test]
    fn deleted_entry_is_removed_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        add(&mut unlocked, "GitLab", "a8ee", "");
        add(&mut unlocked, "Почта", "inbox", "");
        unlocked.delete_selected().unwrap();
        assert!(unlocked.selected.is_none());
        let saved = reopen(&unlocked);
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(saved.entries[0].title, "GitLab");
    }

    #[test]
    fn locking_returns_to_the_password_prompt_for_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let unlocked = unlocked(&dir);
        let path = unlocked.vault.path().to_path_buf();
        let board = crate::clipboard::tests::FakeBoard::default();
        let mut app = LockboxApp {
            config: Config::default(),
            config_path: None,
            screen: Screen::Unlocked(Box::new(unlocked)),
            clipboard: Guard::new(Box::new(board.clone())),
            biometrics: Arc::new(FakeBiometrics::default()),
            icons: IconCache::default(),
        };
        app.clipboard.copy("s3cret", Instant::now()).unwrap();
        app.lock();
        assert_eq!(*board.0.borrow(), None, "locking wipes the clipboard");
        let Screen::Locked(locked) = &app.screen else {
            panic!("vault is still unlocked");
        };
        assert_eq!(locked.path.as_deref(), Some(path.as_path()));
        assert!(locked.error.is_none());
        assert!(locked.password.is_empty());
    }

    fn totp_with(secret: &str) -> Totp {
        Totp {
            secret: secret.into(),
            algorithm: TotpAlgorithm::Sha1,
            digits: 6,
            period: 30,
        }
    }

    #[test]
    fn pasted_link_fills_totp_and_empty_fields_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        unlocked.start_new();
        let draft = unlocked.draft.as_mut().unwrap();
        draft.entry.username = "kept".into();
        draft.entry.totp = Some(totp_with(
            "otpauth://totp/GitLab:a8ee?secret=jbsw%20y3dpehpk3pxp&digits=8",
        ));
        draft.apply_link();
        assert_eq!(draft.totp_error, None);
        assert_eq!(draft.entry.title, "GitLab");
        assert_eq!(draft.entry.username, "kept");
        let totp = draft.entry.totp.as_ref().unwrap();
        assert_eq!((totp.secret.as_str(), totp.digits), ("JBSWY3DPEHPK3PXP", 8));
    }

    #[test]
    fn broken_link_is_reported_and_left_for_correction() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        unlocked.start_new();
        let draft = unlocked.draft.as_mut().unwrap();
        draft.entry.totp = Some(totp_with("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP"));
        draft.apply_link();
        assert!(draft.totp_error.as_deref().unwrap().contains("hotp"));
        assert!(
            draft
                .entry
                .totp
                .as_ref()
                .unwrap()
                .secret
                .starts_with("otpauth://")
        );
    }

    #[test]
    fn invalid_totp_secret_is_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        unlocked.start_new();
        let draft = unlocked.draft.as_mut().unwrap();
        draft.entry.title = "GitLab".into();
        draft.entry.totp = Some(totp_with("not base32!"));
        assert!(unlocked.save_draft().unwrap_err().starts_with("TOTP:"));
        assert!(reopen(&unlocked).entries.is_empty());
    }

    #[test]
    fn totp_secret_is_saved_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let mut unlocked = unlocked(&dir);
        unlocked.start_new();
        let draft = unlocked.draft.as_mut().unwrap();
        draft.entry.title = "GitLab".into();
        draft.entry.totp = Some(totp_with("jbsw y3dp ehpk 3pxp"));
        unlocked.save_draft().unwrap();
        let saved = reopen(&unlocked);
        assert_eq!(
            saved.entries[0].totp.as_ref().unwrap().secret,
            "JBSWY3DPEHPK3PXP"
        );
    }

    #[test]
    fn touch_id_is_offered_only_where_it_was_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        let unlocked = unlocked(&dir);
        let path = unlocked.vault.path().to_path_buf();
        let mut config = Config::default();
        let fake = FakeBiometrics::default();

        let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
        assert!(locked.touch_id.is_none());
        assert!(!locked.auto_touch_id);

        config.touch_id.push(unlocked.vault.id());
        let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
        assert_eq!(locked.touch_id, Some(unlocked.vault.id()));
        assert!(locked.auto_touch_id);

        *fake.fingers.lock().unwrap() = None;
        let locked = Locked::new(Some(path)).offer_touch_id(&config, &fake, true);
        assert!(locked.touch_id.is_none(), "no sensor, no Touch ID button");
    }

    #[test]
    fn typing_the_password_restores_the_touch_id_key() {
        let dir = tempfile::tempdir().unwrap();
        let unlocked = unlocked(&dir);
        let id = unlocked.vault.id();
        let fake = Arc::new(FakeBiometrics::default());
        let mut app = LockboxApp {
            config: Config {
                last_vault: None,
                touch_id: vec![id.clone()],
            },
            config_path: None,
            screen: Screen::Locked(Locked::new(None)),
            clipboard: Guard::new(Box::new(crate::clipboard::tests::FakeBoard::default())),
            biometrics: fake.clone(),
            icons: IconCache::default(),
        };
        app.open(Opened {
            vault: unlocked.vault,
            with_password: true,
        });
        assert!(fake.items.lock().unwrap().contains_key(&id));
        let Screen::Unlocked(unlocked) = &app.screen else {
            panic!("vault did not open");
        };
        assert!(unlocked.touch_id_enabled);
    }

    #[test]
    fn remembered_file_that_disappeared_is_reported() {
        let locked = Locked::new(Some("/nonexistent/vault.lbx".into()));
        assert!(locked.error.unwrap().contains("Файл не найден"));
    }

    #[test]
    fn picked_files_get_the_vault_extension() {
        assert_eq!(
            with_vault_extension("/tmp/secrets".into()),
            PathBuf::from("/tmp/secrets.lbx")
        );
        assert_eq!(
            with_vault_extension("/tmp/vault.lbx".into()),
            PathBuf::from("/tmp/vault.lbx")
        );
    }
}
