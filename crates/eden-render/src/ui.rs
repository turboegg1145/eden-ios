//! Full game UI state machine and software compositor.
//!
//! All screens are composed from the retail `sys.paz` assets (title menus,
//! save/load bases, music room, CG gallery, message panel, icons). The same
//! model is used by the desktop CLI and the iOS host through `eden-ffi`.
//!
//! The UI owns no game state: it emits [`UiCommand`] values that the runtime
//! applies to the VM, archives and save slots.

use std::sync::Arc;

use eden_paz::image::RgbaImage;

use crate::text::FontRenderer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiScreen {
    Title,
    Game,
    Save,
    Load,
    Backlog,
    Gallery,
    GalleryView,
    Music,
    Config,
    Extra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiCommand {
    StartMain,
    StartPlusMosaic,
    Continue,
    OpenTitle,
    OpenSave,
    OpenLoad,
    OpenBacklog,
    OpenGallery,
    OpenGalleryView(usize),
    OpenMusic,
    OpenConfig,
    OpenExtra,
    SaveSlot(usize),
    LoadSlot(usize),
    Back,
    PlayMusic(String),
    PrevMusic,
    NextMusic,
    PrevPage,
    NextPage,
    PlayMovie,
    StopMusic,
    PlayVoice(String),
    SetAuto(bool),
    SetSkip(bool),
    ToggleHideUi,
    ConfigChanged,
    ConfigAdjust(usize, i32),
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct SaveSlot {
    pub index: usize,
    pub thumbnail: Option<Arc<RgbaImage>>,
    pub timestamp: String,
    pub summary: String,
    pub script: String,
    pub empty: bool,
}

#[derive(Clone, Debug)]
pub struct BacklogEntry {
    pub id: i64,
    pub speaker: Option<String>,
    pub text: String,
    pub voice: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GalleryEntry {
    pub thumb_name: String,
    pub full_name: String,
    pub thumb: Option<Arc<RgbaImage>>,
    pub full: Option<Arc<RgbaImage>>,
    pub unlocked: bool,
}

#[derive(Clone, Debug)]
pub struct MusicEntry {
    pub file: String,
    pub title: String,
    pub unlocked: bool,
}

#[derive(Clone, Debug)]
pub struct UiConfig {
    pub bgm_volume: u8,
    pub se_volume: u8,
    pub voice_volume: u8,
    pub text_speed: u8,
    pub auto_speed: u8,
    pub fullscreen: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            bgm_volume: 80,
            se_volume: 80,
            voice_volume: 90,
            text_speed: 60,
            auto_speed: 60,
            fullscreen: true,
        }
    }
}

/// Images loaded from `sys.paz` / `bg.paz` by the runtime.
#[derive(Default)]
pub struct UiAssets {
    pub title: Vec<Arc<RgbaImage>>,
    pub save_base: Vec<Arc<RgbaImage>>,
    pub load_base: Vec<Arc<RgbaImage>>,
    pub music_base: Option<Arc<RgbaImage>>,
    pub music_pages: Vec<Arc<RgbaImage>>,
    pub cg_base: Vec<Arc<RgbaImage>>,
    pub cg_box: Vec<Arc<RgbaImage>>,
    pub cg_menu: Vec<Arc<RgbaImage>>,
    pub cg_mode: usize,
    pub config_base: Option<Arc<RgbaImage>>,
    pub msg_panel: Option<Arc<RgbaImage>>,
    pub backlog_gauge: Option<Arc<RgbaImage>>,
    pub icon_auto: Option<Arc<RgbaImage>>,
    pub icon_skip: Option<Arc<RgbaImage>>,
    pub icon_save: Option<Arc<RgbaImage>>,
    pub icon_load: Option<Arc<RgbaImage>>,
    pub icon_system: Option<Arc<RgbaImage>>,
    pub icon_back: Option<Arc<RgbaImage>>,
    pub select: Option<Arc<RgbaImage>>,
    pub extra: Option<Arc<RgbaImage>>,
}

pub struct UiModel {
    pub screen: UiScreen,
    pub previous: UiScreen,
    pub hover: Option<usize>,
    pub page: usize,
    pub slots: Vec<SaveSlot>,
    pub backlog: Vec<BacklogEntry>,
    pub backlog_offset: usize,
    pub gallery: Vec<GalleryEntry>,
    pub music: Vec<MusicEntry>,
    pub config: UiConfig,
    pub auto: bool,
    pub skip: bool,
    pub hide_ui: bool,
    pub selected_gallery: Option<usize>,
    pub selected_music: Option<usize>,
    pub toast: Option<String>,
    pub toast_frames: u32,
    pub cursor: (i32, i32),
    pub cursor_visible: bool,
}

impl Default for UiModel {
    fn default() -> Self {
        Self {
            screen: UiScreen::Title,
            previous: UiScreen::Title,
            hover: None,
            page: 0,
            slots: (0..10)
                .map(|index| SaveSlot {
                    index,
                    empty: true,
                    ..Default::default()
                })
                .collect(),
            backlog: Vec::new(),
            backlog_offset: 0,
            gallery: Vec::new(),
            music: Vec::new(),
            config: UiConfig::default(),
            auto: false,
            skip: false,
            hide_ui: false,
            selected_gallery: None,
            selected_music: None,
            toast: None,
            toast_frames: 0,
            cursor: (0, 0),
            cursor_visible: false,
        }
    }
}

impl UiModel {
    pub fn open(&mut self, screen: UiScreen) {
        self.previous = self.screen;
        if screen != self.screen {
            self.page = 0;
        }
        self.screen = screen;
        self.hover = None;
        self.toast = None;
        self.toast_frames = 0;
        if screen == UiScreen::Backlog {
            self.backlog_offset = self.backlog.len().saturating_sub(BACKLOG_ROWS);
        }
    }

    pub fn back(&mut self) {
        self.screen = match self.screen {
            UiScreen::GalleryView => UiScreen::Gallery,
            UiScreen::Save | UiScreen::Load | UiScreen::Config => UiScreen::Title,
            UiScreen::Backlog => UiScreen::Game,
            UiScreen::Gallery | UiScreen::Music | UiScreen::Extra => UiScreen::Title,
            other => other,
        };
        self.page = 0;
        self.selected_gallery = None;
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(text.into());
        self.toast_frames = 150;
    }

    pub fn clear_toast(&mut self) {
        self.toast = None;
        self.toast_frames = 0;
    }

    pub fn tick(&mut self) {
        if self.toast_frames > 0 {
            self.toast_frames -= 1;
            if self.toast_frames == 0 {
                self.toast = None;
            }
        }
    }

    pub fn scroll(&mut self, delta: i32) {
        if self.screen == UiScreen::Backlog {
            let max = self.backlog.len().saturating_sub(BACKLOG_ROWS);
            let next = self.backlog_offset as i32 + delta;
            self.backlog_offset = next.clamp(0, max as i32) as usize;
        }
    }

    /// Mouse/touch hover, no command.
    pub fn move_cursor(&mut self, x: i32, y: i32) {
        self.cursor = (x, y);
        self.cursor_visible = true;
        self.hover = self.hit(x, y).map(|(i, _)| i);
    }

    /// Handle a primary tap. Returns the action to apply.
    pub fn tap(&mut self, x: i32, y: i32) -> Option<UiCommand> {
        self.cursor = (x, y);
        self.cursor_visible = true;
        let (index, command) = self.hit(x, y)?;
        self.hover = Some(index);
        Some(command)
    }

    fn hit(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        match self.screen {
            UiScreen::Title => self.hit_title(x, y),
            UiScreen::Game => self.hit_game(x, y),
            UiScreen::Save => self.hit_slot(x, y, true),
            UiScreen::Load => self.hit_slot(x, y, false),
            UiScreen::Backlog => self.hit_backlog(x, y),
            UiScreen::Gallery => self.hit_gallery(x, y),
            UiScreen::GalleryView => self.hit_gallery_view(x, y),
            UiScreen::Music => self.hit_music(x, y),
            UiScreen::Config => self.hit_config(x, y),
            UiScreen::Extra => self.hit_extra(x, y),
        }
    }

    fn hit_title(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        for (i, (rect, command)) in TITLE_ITEMS.iter().enumerate() {
            if contains(*rect, x, y) {
                return Some((i, command.clone()));
            }
        }
        None
    }

    fn hit_game(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        for (i, (rect, command)) in GAME_BUTTONS.iter().enumerate() {
            if contains(*rect, x, y) {
                return Some((i, command.clone()));
            }
        }
        None
    }

    fn hit_slot(&self, x: i32, y: i32, save: bool) -> Option<(usize, UiCommand)> {
        for i in 0..10 {
            if contains(slot_rect(i), x, y) {
                let command = if save {
                    UiCommand::SaveSlot(i)
                } else {
                    UiCommand::LoadSlot(i)
                };
                return Some((i, command));
            }
        }
        if contains((24, 590, 160, 630), x, y) {
            return Some((10, UiCommand::Back));
        }
        None
    }

    fn hit_backlog(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        if contains((24, 590, 160, 630), x, y) {
            return Some((0, UiCommand::Back));
        }
        let row = ((y - BACKLOG_TOP) / BACKLOG_ROW_H) as usize;
        if (0..BACKLOG_ROWS).contains(&row) {
            let index = self.backlog_offset + row;
            if let Some(entry) = self.backlog.get(index) {
                if let Some(voice) = &entry.voice {
                    return Some((1 + row, UiCommand::PlayVoice(voice.clone())));
                }
            }
        }
        None
    }

    fn hit_gallery(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        let per_page = GALLERY_COLS * GALLERY_ROWS;
        for cell in 0..per_page {
            if contains(gallery_rect(cell), x, y) {
                let index = self.page * per_page + cell;
                if let Some(entry) = self.gallery.get(index) {
                    if entry.unlocked {
                        return Some((1 + cell, UiCommand::OpenGalleryView(index)));
                    }
                }
                return None;
            }
        }
        // Navigation baked into cgmode0menu.png / cgmode1menu.png.
        if contains((604, 578, 690, 618), x, y) {
            return Some((30, UiCommand::Back));
        }
        if contains((730, 578, 828, 618), x, y) && (self.page + 1) * per_page < self.gallery.len()
        {
            return Some((31, UiCommand::NextPage));
        }
        if contains((860, 578, 948, 618), x, y) {
            return Some((32, UiCommand::Back));
        }
        None
    }

    fn hit_gallery_view(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        if x >= 0 && y >= 0 {
            return Some((0, UiCommand::Back));
        }
        None
    }

    fn hit_music(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        // Transport controls baked into musicBase.png.
        if contains((690, 160, 736, 206), x, y) {
            return Some((1, UiCommand::PrevMusic));
        }
        if contains((752, 160, 798, 206), x, y) {
            return Some((2, UiCommand::Continue));
        }
        if contains((816, 160, 862, 206), x, y) {
            return Some((3, UiCommand::StopMusic));
        }
        if contains((880, 160, 926, 206), x, y) {
            return Some((4, UiCommand::NextMusic));
        }
        for row in 0..MUSIC_ROWS {
            let index = self.page * MUSIC_ROWS + row;
            if contains((104, 34 + row as i32 * 32, 560, 64 + row as i32 * 32), x, y) {
                if let Some(entry) = self.music.get(index) {
                    if entry.unlocked {
                        return Some((10 + row, UiCommand::PlayMusic(entry.file.clone())));
                    }
                }
            }
        }
        // Navigation baked into musicBase.png.
        if contains((604, 578, 690, 618), x, y) && self.page > 0 {
            return Some((50, UiCommand::PrevPage));
        }
        if contains((730, 578, 828, 618), x, y)
            && (self.page + 1) * MUSIC_ROWS < self.music.len()
        {
            return Some((51, UiCommand::NextPage));
        }
        if contains((860, 578, 948, 618), x, y) {
            return Some((52, UiCommand::Back));
        }
        None
    }

    fn hit_config(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        // Sliders baked into configBase.png: left column (text speed / auto
        // wait) and right column (BGM / voice / SE).
        let sliders = [
            ((143, 104, 343, 126), 3),
            ((143, 186, 343, 208), 4),
            ((680, 104, 880, 126), 0),
            ((680, 186, 880, 208), 2),
            ((680, 266, 880, 288), 1),
        ];
        for (i, (rect, index)) in sliders.iter().enumerate() {
            if contains(*rect, x, y) {
                let delta = if x < rect.0 + 100 { -5 } else { 5 };
                return Some((1 + i, UiCommand::ConfigAdjust(*index, delta)));
            }
        }
        if contains((128, 548, 300, 572), x, y) {
            return Some((10, UiCommand::SetAuto(true)));
        }
        if contains((128, 578, 300, 602), x, y) {
            return Some((11, UiCommand::SetSkip(true)));
        }
        if contains((690, 560, 750, 596), x, y) || contains((805, 560, 870, 596), x, y) {
            return Some((12, UiCommand::Back));
        }
        None
    }

    fn hit_extra(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        // Labels are baked into memories.png.
        if contains((810, 86, 960, 122), x, y) {
            return Some((0, UiCommand::OpenGallery));
        }
        if contains((780, 166, 960, 202), x, y) {
            return Some((1, UiCommand::PlayMovie));
        }
        if contains((780, 246, 960, 282), x, y) {
            return Some((2, UiCommand::OpenMusic));
        }
        if contains((855, 580, 950, 618), x, y) {
            return Some((3, UiCommand::Back));
        }
        None
    }
}

fn contains(rect: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    x >= rect.0 && y >= rect.1 && x < rect.2 && y < rect.3
}

const TITLE_ITEMS: &[((i32, i32, i32, i32), UiCommand)] = &[
    ((850, 98, 1010, 128), UiCommand::StartMain),
    ((850, 134, 1010, 164), UiCommand::OpenLoad),
    ((850, 170, 1010, 200), UiCommand::OpenConfig),
    ((850, 206, 1010, 236), UiCommand::OpenExtra),
    ((850, 242, 1010, 272), UiCommand::Quit),
    ((850, 280, 1010, 312), UiCommand::StartPlusMosaic),
];

const GAME_BUTTONS: &[((i32, i32, i32, i32), UiCommand)] = &[
    ((884, 596, 936, 638), UiCommand::SetAuto(true)),
    ((936, 596, 988, 638), UiCommand::SetSkip(true)),
    ((780, 596, 824, 638), UiCommand::OpenSave),
    ((824, 596, 868, 638), UiCommand::OpenLoad),
    ((868, 596, 884, 638), UiCommand::OpenTitle),
];

const BACKLOG_TOP: i32 = 70;
const BACKLOG_ROWS: usize = 18;
const BACKLOG_ROW_H: i32 = 29;
const GALLERY_COLS: usize = 5;
const GALLERY_ROWS: usize = 5;
const MUSIC_ROWS: usize = 16;

fn slot_rect(index: usize) -> (i32, i32, i32, i32) {
    let col = index % 2;
    let row = index / 2;
    let x = if col == 0 { 144 } else { 540 };
    let y = 47 + row as i32 * 107;
    (x, y, x + 344, y + 90)
}

fn gallery_rect(cell: usize) -> (i32, i32, i32, i32) {
    let col = cell % GALLERY_COLS;
    let row = cell / GALLERY_COLS;
    // Slots are baked into cgmode0box.png/cgmode1box.png (5x5, 128x80).
    let x = 96 + col as i32 * 176;
    let y = 30 + row as i32 * 113;
    (x, y, x + 128, y + 80)
}

/// Compose the current UI over `game_frame` (or a title/gallery base).
pub fn render(
    model: &UiModel,
    assets: &UiAssets,
    game_frame: &RgbaImage,
    font: Option<&FontRenderer>,
) -> RgbaImage {
    let edition = assets.cg_mode.min(assets.title.len().saturating_sub(1));
    let mut out = match model.screen {
        UiScreen::Title => clone_or_black(assets.title.get(edition)),
        UiScreen::Save => clone_or_black(
            assets
                .save_base
                .get(assets.cg_mode.min(assets.save_base.len().saturating_sub(1)))
                .or_else(|| assets.save_base.first()),
        ),
        UiScreen::Load => clone_or_black(
            assets
                .load_base
                .get(assets.cg_mode.min(assets.load_base.len().saturating_sub(1)))
                .or_else(|| assets.load_base.first()),
        ),
        UiScreen::Gallery | UiScreen::GalleryView => {
            // cgmode1box/cgmode1menu are the pink "eden* snaps" gallery used
            // by the main edition; cgmode0 is the green PLUS+MOSAIC variant.
            let index = if assets.cg_box.len() > 1 {
                1 - assets.cg_mode.min(1)
            } else {
                0
            };
            let mut base = clone_or_black(
                assets
                    .cg_box
                    .get(index)
                    .or_else(|| assets.cg_base.get(index)),
            );
            if model.screen == UiScreen::Gallery {
                if let Some(menu) = assets.cg_menu.get(index) {
                    blit_at(&mut base, menu, 600, 570);
                }
            }
            base
        }
        UiScreen::Music => clone_or_black(assets.music_base.as_ref()),
        UiScreen::Config => clone_or_black(assets.config_base.as_ref().or(assets.extra.as_ref())),
        UiScreen::Extra => clone_or_black(assets.extra.as_ref()),
        UiScreen::Game | UiScreen::Backlog => game_frame.clone(),
    };
    match model.screen {
        UiScreen::Title => draw_title(&mut out, model, assets, font),
        UiScreen::Game => draw_game(&mut out, model, assets, font),
        UiScreen::Save | UiScreen::Load => draw_slots(&mut out, model, font, model.screen == UiScreen::Save),
        UiScreen::Backlog => draw_backlog(&mut out, model, assets, font),
        UiScreen::Gallery => draw_gallery(&mut out, model, font),
        UiScreen::GalleryView => draw_gallery_view(&mut out, model, font),
        UiScreen::Music => draw_music(&mut out, model, font),
        UiScreen::Config => draw_config(&mut out, model, font),
        UiScreen::Extra => draw_extra(&mut out, model, font),
    }
    if let Some(toast) = &model.toast {
        draw_toast(&mut out, toast, font);
    }
    if model.cursor_visible {
        let (x, y) = model.cursor;
        for dy in 0..12 {
            for dx in 0..12 {
                let px = x + dx - 6;
                let py = y + dy - 6;
                if px >= 0 && py >= 0 {
                    out.blend_pixel(px as u32, py as u32, [255, 255, 255, 160], 255);
                }
            }
        }
    }
    out
}

fn clone_or_black(image: Option<&Arc<RgbaImage>>) -> RgbaImage {
    match image {
        Some(image) => (**image).clone(),
        None => RgbaImage::filled(1024, 640, [0, 0, 0, 255]),
    }
}

fn draw_title(out: &mut RgbaImage, model: &UiModel, assets: &UiAssets, font: Option<&FontRenderer>) {
    // Hover highlight over the baked-in menu text.
    if let Some(index) = model.hover {
        if let Some((rect, _)) = TITLE_ITEMS.get(index) {
            fill_rect(out, *rect, [255, 255, 255, 26]);
            stroke_rect(out, *rect, [255, 210, 120, 170]);
        }
    }
    // PLUS+MOSAIC is not baked into topmenu*.png; draw it below Exit.
    fill_rect(out, (852, 282, 1004, 310), [12, 18, 32, 140]);
    if let Some(font) = font {
        font.draw_text(
            out,
            "PLUS+MOSAIC",
            864.0,
            285.0,
            17.0,
            [255, 232, 170, 255],
            150.0,
        );
    }
    // Edition indicator for the three retail title artworks.
    for i in 0..assets.title.len().min(3) {
        let x = 40 + i as i32 * 18;
        let color = if i == assets.cg_mode {
            [255, 255, 255, 220]
        } else {
            [255, 255, 255, 80]
        };
        fill_rect(out, (x, 570, x + 12, 574), color);
    }
}

fn draw_game(out: &mut RgbaImage, model: &UiModel, assets: &UiAssets, font: Option<&FontRenderer>) {
    if model.hide_ui {
        return;
    }
    // Bottom-right quick bar.
    fill_rect(out, (776, 592, 992, 640), [0, 0, 0, 110]);
    let icons: [(&Option<Arc<RgbaImage>>, &str); 5] = [
        (&assets.icon_save, "S"),
        (&assets.icon_load, "L"),
        (&assets.icon_system, "M"),
        (&assets.icon_auto, "A"),
        (&assets.icon_skip, "K"),
    ];
    for (i, (image, label)) in icons.iter().enumerate() {
        let x = 780 + i as i32 * 42;
        if let Some(image) = image {
            blit_scaled(out, image, x, 598, 36, 36);
        } else if let Some(font) = font {
            font.draw_text(out, label, x as f32 + 8.0, 602.0, 20.0, [255, 255, 255, 220], 30.0);
        }
    }
    if model.auto {
        fill_rect(out, (924, 596, 970, 638), [80, 200, 120, 110]);
    }
    if model.skip {
        fill_rect(out, (966, 596, 1012, 638), [220, 120, 60, 130]);
    }
    if let Some(font) = font {
        if model.auto {
            font.draw_text(out, "AUTO", 926.0, 606.0, 13.0, [255, 255, 255, 255], 44.0);
        }
        if model.skip {
            font.draw_text(out, "SKIP", 968.0, 606.0, 13.0, [255, 255, 255, 255], 44.0);
        }
    }
}

fn draw_slots(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>, save: bool) {
    for index in 0..10 {
        let rect = slot_rect(index);
        if model.hover == Some(index) {
            stroke_rect(out, rect, [255, 210, 90, 255]);
        }
        let Some(slot) = model.slots.get(index) else {
            continue;
        };
        if let Some(thumb) = &slot.thumbnail {
            blit_scaled(
                out,
                thumb,
                rect.0 + 6,
                rect.1 + 6,
                160,
                (rect.3 - rect.1 - 12) as u32,
            );
        } else {
            fill_rect(
                out,
                (rect.0 + 6, rect.1 + 6, rect.0 + 166, rect.3 - 6),
                [30, 40, 34, 255],
            );
        }
        if let Some(font) = font {
            let text_x = (rect.0 + 174) as f32;
            let width = (rect.2 - rect.0 - 182) as f32;
            let title = if slot.empty {
                if save {
                    format!("{:02}  — empty —", index + 1)
                } else {
                    format!("{:02}  — no data —", index + 1)
                }
            } else {
                let stamp = slot.timestamp.get(..16).unwrap_or(&slot.timestamp);
                format!("{:02}  {}", index + 1, stamp)
            };
            font.draw_text(
                out,
                &fit_text(font, &title, 13.0, width),
                text_x,
                (rect.1 + 7) as f32,
                13.0,
                [28, 44, 34, 255],
                width,
            );
            if !slot.empty {
                font.draw_text(
                    out,
                    &fit_text(font, &slot.summary, 12.0, width),
                    text_x,
                    (rect.1 + 33) as f32,
                    12.0,
                    [40, 60, 48, 255],
                    width,
                );
                font.draw_text(
                    out,
                    &fit_text(font, &slot.script, 10.0, width),
                    text_x,
                    (rect.3 - 19) as f32,
                    10.0,
                    [60, 80, 66, 255],
                    width,
                );
            }
        }
    }
    if let Some(font) = font {
        font.draw_text(out, "Back", 40.0, 596.0, 18.0, [240, 240, 240, 220], 120.0);
    }
}

/// Truncate `text` with an ellipsis so it never wraps inside a UI slot.
fn fit_text(font: &FontRenderer, text: &str, size: f32, max_width: f32) -> String {
    let text = text.replace(['\n', '\r'], " ");
    if font.measure_line(&text, size) <= max_width {
        return text;
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push('…');
        if font.measure_line(&candidate, size) > max_width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn draw_backlog(
    out: &mut RgbaImage,
    model: &UiModel,
    assets: &UiAssets,
    font: Option<&FontRenderer>,
) {
    fill_rect(out, (0, 0, 1024, 640), [4, 8, 10, 225]);
    if let Some(gauge) = &assets.backlog_gauge {
        blit_scaled(out, gauge, 992, 76, 18, 480);
    }
    if let Some(font) = font {
        font.draw_text(out, "Backlog", 34.0, 20.0, 26.0, [255, 255, 255, 255], 300.0);
        let total = model.backlog.len();
        for row in 0..BACKLOG_ROWS {
            let index = model.backlog_offset + row;
            let Some(entry) = model.backlog.get(index) else {
                break;
            };
            let y = BACKLOG_TOP + row as i32 * BACKLOG_ROW_H;
            if let Some(speaker) = &entry.speaker {
                font.draw_text(out, speaker, 40.0, y as f32, 16.0, [255, 214, 130, 255], 150.0);
            }
            let color = if entry.voice.is_some() {
                [225, 238, 255, 255]
            } else {
                [200, 205, 210, 255]
            };
            let text = entry.text.replace('\n', " ");
            font.draw_text(out, &text, 190.0, y as f32, 16.0, color, 760.0);
            if entry.voice.is_some() {
                fill_rect(out, (176, y + 3, 180, y + 15), [120, 190, 255, 200]);
            }
        }
        font.draw_text(
            out,
            &format!(
                "{}–{} / {}   (swipe to scroll)",
                model.backlog_offset + 1,
                (model.backlog_offset + BACKLOG_ROWS).min(total),
                total
            ),
            620.0,
            604.0,
            13.0,
            [200, 210, 220, 220],
            380.0,
        );
        font.draw_text(out, "Back", 40.0, 594.0, 18.0, [240, 240, 240, 220], 120.0);
    }
}

fn draw_gallery(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>) {
    let per_page = GALLERY_COLS * GALLERY_ROWS;
    for cell in 0..per_page {
        let index = model.page * per_page + cell;
        let Some(entry) = model.gallery.get(index) else {
            break;
        };
        let rect = gallery_rect(cell);
        if entry.unlocked {
            if let Some(thumb) = &entry.thumb {
                blit_scaled(out, thumb, rect.0, rect.1, 128, 78);
            }
        } else {
            // Cover the retail placeholder so no locked artwork leaks through.
            fill_rect(out, rect, [112, 118, 114, 255]);
            if let Some(font) = font {
                font.draw_text(
                    out,
                    "LOCKED",
                    (rect.0 + 36) as f32,
                    (rect.1 + 28) as f32,
                    14.0,
                    [60, 68, 62, 230],
                    80.0,
                );
            }
        }
        if model.hover == Some(1 + cell) {
            stroke_rect(out, rect, [255, 220, 120, 255]);
        }
    }
    if let Some(font) = font {
        let pages = (model.gallery.len() + per_page - 1) / per_page.max(1);
        font.draw_text(
            out,
            &format!("Page {}/{}", model.page + 1, pages.max(1)),
            40.0,
            10.0,
            15.0,
            [230, 230, 230, 255],
            160.0,
        );
    }
}

fn draw_gallery_view(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>) {
    fill_rect(out, (0, 0, 1024, 640), [0, 0, 0, 255]);
    if let Some(index) = model.selected_gallery {
        if let Some(entry) = model.gallery.get(index) {
            if let Some(full) = &entry.full {
                let scale = (1024.0 / full.width as f32).min(640.0 / full.height as f32);
                let w = (full.width as f32 * scale) as u32;
                let h = (full.height as f32 * scale) as u32;
                blit_scaled(
                    out,
                    full,
                    ((1024 - w) / 2) as i32,
                    ((640 - h) / 2) as i32,
                    w,
                    h,
                );
            } else if let Some(thumb) = &entry.thumb {
                let scale = (1024.0 / thumb.width as f32).min(640.0 / thumb.height as f32);
                let w = (thumb.width as f32 * scale) as u32;
                let h = (thumb.height as f32 * scale) as u32;
                blit_scaled(out, thumb, ((1024 - w) / 2) as i32, ((640 - h) / 2) as i32, w, h);
            }
            if let Some(font) = font {
                font.draw_text(
                    out,
                    &entry.full_name,
                    30.0,
                    608.0,
                    14.0,
                    [230, 230, 230, 220],
                    500.0,
                );
            }
        }
    }
    if let Some(font) = font {
        font.draw_text(
            out,
            "Tap to return",
            840.0,
            608.0,
            14.0,
            [230, 230, 230, 200],
            160.0,
        );
    }
}

fn draw_music(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>) {
    fill_rect(out, (88, 16, 620, 570), [6, 10, 8, 170]);
    for row in 0..MUSIC_ROWS {
        let index = model.page * MUSIC_ROWS + row;
        let Some(entry) = model.music.get(index) else {
            break;
        };
        let y = 34 + row as i32 * 32;
        if model.selected_music == Some(index) || model.hover == Some(10 + row) {
            fill_rect(out, (96, y - 2, 610, y + 28), [255, 255, 255, 34]);
        }
        if let Some(font) = font {
            let (label, color) = if entry.unlocked {
                let name = entry
                    .title
                    .strip_prefix("edenBGM")
                    .map(|n| format!("BGM {n}"))
                    .unwrap_or_else(|| entry.title.clone());
                (format!("{:02}.  {}", index + 1, name), [245, 245, 245, 255])
            } else {
                (format!("{:02}.  — LOCKED —", index + 1), [110, 120, 112, 255])
            };
            font.draw_text(out, &label, 104.0, y as f32, 20.0, color, 500.0);
        }
    }
    if let Some(font) = font {
        let _pages = (model.music.len() + MUSIC_ROWS - 1) / MUSIC_ROWS.max(1);
        font.draw_text(
            out,
            &format!("Page#{:02}", model.page + 1),
            520.0,
            18.0,
            14.0,
            [230, 235, 230, 220],
            100.0,
        );
    }
}

fn draw_config(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>) {
    let knob = |out: &mut RgbaImage, x0: i32, x1: i32, y: i32, value: u8| {
        let x = x0 + ((x1 - x0) as f32 * value as f32 / 100.0) as i32;
        fill_rect(out, (x - 2, y - 12, x + 3, y + 12), [250, 250, 250, 235]);
        fill_rect(out, (x - 1, y - 11, x + 2, y + 11), [90, 170, 120, 255]);
    };
    knob(out, 143, 343, 115, model.config.text_speed);
    knob(out, 143, 343, 197, model.config.auto_speed);
    knob(out, 680, 879, 115, model.config.bgm_volume);
    knob(out, 680, 879, 197, model.config.voice_volume);
    knob(out, 680, 879, 277, model.config.se_volume);
    // Check marks over the baked 自动 / 跳过 boxes.
    if let Some(font) = font {
        if model.auto {
            font.draw_text(out, "✓", 132.0, 550.0, 20.0, [120, 240, 150, 255], 30.0);
        }
        if model.skip {
            font.draw_text(out, "✓", 132.0, 580.0, 20.0, [120, 240, 150, 255], 30.0);
        }
    }
    if let Some(index) = model.hover {
        let rect = match index {
            1 => Some((143, 104, 343, 126)),
            2 => Some((143, 186, 343, 208)),
            3 => Some((680, 104, 880, 126)),
            4 => Some((680, 186, 880, 208)),
            5 => Some((680, 266, 880, 288)),
            _ => None,
        };
        if let Some(rect) = rect {
            stroke_rect(out, rect, [120, 240, 150, 200]);
        }
    }
}

fn draw_extra(out: &mut RgbaImage, model: &UiModel, font: Option<&FontRenderer>) {
    // Labels 快照 / 播放影片 / 播放音乐 / Return are baked into memories.png.
    if let Some(index) = model.hover {
        let rect = match index {
            0 => Some((820, 86, 955, 122)),
            1 => Some((790, 166, 955, 202)),
            2 => Some((790, 246, 955, 282)),
            3 => Some((855, 580, 950, 618)),
            _ => None,
        };
        if let Some(rect) = rect {
            stroke_rect(out, rect, [255, 220, 130, 190]);
        }
    }
    if let Some(font) = font {
        let cgs = model.gallery.iter().filter(|g| g.unlocked).count();
        let tracks = model.music.iter().filter(|m| m.unlocked).count();
        font.draw_text(
            out,
            &format!(
                "{} CG{} / {} track{}",
                cgs,
                if cgs == 1 { "" } else { "s" },
                tracks,
                if tracks == 1 { "" } else { "s" }
            ),
            40.0,
            40.0,
            15.0,
            [220, 235, 225, 200],
            300.0,
        );
    }
}

fn draw_toast(out: &mut RgbaImage, text: &str, font: Option<&FontRenderer>) {
    fill_rect(out, (300, 540, 724, 586), [0, 0, 0, 190]);
    if let Some(font) = font {
        font.draw_text(out, text, 320.0, 550.0, 16.0, [255, 240, 200, 255], 390.0);
    }
}

fn fill_rect(out: &mut RgbaImage, rect: (i32, i32, i32, i32), color: [u8; 4]) {
    let x0 = rect.0.max(0) as u32;
    let y0 = rect.1.max(0) as u32;
    let x1 = rect.2.min(out.width as i32).max(0) as u32;
    let y1 = rect.3.min(out.height as i32).max(0) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            out.blend_pixel(x, y, color, 255);
        }
    }
}

fn stroke_rect(out: &mut RgbaImage, rect: (i32, i32, i32, i32), color: [u8; 4]) {
    fill_rect(out, (rect.0, rect.1, rect.2, rect.1 + 2), color);
    fill_rect(out, (rect.0, rect.3 - 2, rect.2, rect.3), color);
    fill_rect(out, (rect.0, rect.1, rect.0 + 2, rect.3), color);
    fill_rect(out, (rect.2 - 2, rect.1, rect.2, rect.3), color);
}

fn blit_scaled(
    out: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) {
    if src.width == 0 || src.height == 0 || w == 0 || h == 0 {
        return;
    }
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w as i32).min(out.width as i32).max(0) as u32;
    let y1 = (y + h as i32).min(out.height as i32).max(0) as u32;
    for sy in y0..y1 {
        let v = (sy as f32 - y as f32 + 0.5) / h as f32;
        let src_y = (v * src.height as f32) as u32;
        for sx in x0..x1 {
            let u = (sx as f32 - x as f32 + 0.5) / w as f32;
            let src_x = (u * src.width as f32) as u32;
            let p = src.pixel(src_x.min(src.width - 1), src_y.min(src.height - 1));
            out.blend_pixel(sx, sy, p, 255);
        }
    }
}

fn blit_at(out: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32) {
    blit_scaled(out, src, x, y, src.width, src.height);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_hit_regions_cover_every_item() {
        let mut ui = UiModel::default();
        assert_eq!(ui.tap(900, 110), Some(UiCommand::StartMain));
        assert_eq!(ui.tap(900, 140), Some(UiCommand::OpenLoad));
        assert_eq!(ui.tap(900, 176), Some(UiCommand::OpenConfig));
        assert_eq!(ui.tap(900, 212), Some(UiCommand::OpenExtra));
        assert_eq!(ui.tap(900, 248), Some(UiCommand::Quit));
        assert_eq!(ui.tap(900, 290), Some(UiCommand::StartPlusMosaic));
    }

    #[test]
    fn gallery_uses_5x5_grid_and_blocks_locked() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Gallery;
        ui.gallery = (0..30)
            .map(|i| GalleryEntry {
                thumb_name: format!("cgthumb\\{i}.png"),
                full_name: format!("{i}.png"),
                thumb: None,
                full: None,
                unlocked: i == 0,
            })
            .collect();
        assert_eq!(ui.tap(100, 35), Some(UiCommand::OpenGalleryView(0)));
        assert_eq!(ui.tap(280, 35), None); // locked cell does nothing
        assert_eq!(ui.tap(780, 590), Some(UiCommand::NextPage)); // baked Next
    }

    #[test]
    fn music_nav_and_transport_hit_regions() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Music;
        ui.music = (0..20)
            .map(|i| MusicEntry {
                file: format!("edenBGM{i:03}.ogg"),
                title: format!("edenBGM{i:03}"),
                unlocked: true,
            })
            .collect();
        assert_eq!(ui.tap(710, 180), Some(UiCommand::PrevMusic));
        assert_eq!(ui.tap(775, 180), Some(UiCommand::Continue));
        assert_eq!(ui.tap(835, 180), Some(UiCommand::StopMusic));
        assert_eq!(ui.tap(900, 180), Some(UiCommand::NextMusic));
        assert_eq!(ui.tap(780, 590), Some(UiCommand::NextPage));
        assert_eq!(ui.tap(900, 590), Some(UiCommand::Back));
    }

    #[test]
    fn config_slider_and_extra_hit_regions() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Config;
        assert_eq!(ui.tap(200, 115), Some(UiCommand::ConfigAdjust(3, -5)));
        assert_eq!(ui.tap(800, 115), Some(UiCommand::ConfigAdjust(0, 5)));
        ui.screen = UiScreen::Extra;
        assert_eq!(ui.tap(900, 100), Some(UiCommand::OpenGallery));
        assert_eq!(ui.tap(900, 180), Some(UiCommand::PlayMovie));
        assert_eq!(ui.tap(900, 260), Some(UiCommand::OpenMusic));
        assert_eq!(ui.tap(900, 600), Some(UiCommand::Back));
    }

    #[test]
    fn save_slot_hit_and_back() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Save;
        assert_eq!(ui.tap(200, 60), Some(UiCommand::SaveSlot(0)));
        assert_eq!(ui.tap(600, 60), Some(UiCommand::SaveSlot(1)));
        assert_eq!(ui.tap(40, 600), Some(UiCommand::Back));
    }

    #[test]
    fn backlog_scroll_is_clamped() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Backlog;
        ui.backlog = (0..40)
            .map(|i| BacklogEntry {
                id: i,
                speaker: None,
                text: format!("line {i}"),
                voice: None,
            })
            .collect();
        ui.scroll(100);
        assert_eq!(ui.backlog_offset, 22);
        ui.scroll(-100);
        assert_eq!(ui.backlog_offset, 0);
    }
}
