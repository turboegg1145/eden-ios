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
    QuickSave,
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
    ConfigToggle(usize),
    ConfigTest(usize),
    ConfigFont(i32),
    ConfigConfirm,
    ConfigCancel,
    TitleEdition(usize),
    TitleArt(i32),
    RadialOpen(i32, i32),
    RadialClose,
    Quit,
}

/// Native title artwork and menu layout for one retail edition.
#[derive(Clone, Default)]
pub struct TitleSet {
    pub art: Vec<Arc<RgbaImage>>,
    /// `topmenuNover.png`: same artwork with the menu text re-rendered
    /// brighter. Used as a per-row hover sprite.
    pub over: Vec<Arc<RgbaImage>>,
    /// `PLUS+MOSAIC` text lifted from the PM artwork as an alpha mask.
    pub plus_mosaic: Option<Arc<RgbaImage>>,
}

impl TitleSet {
    pub fn is_empty(&self) -> bool {
        self.art.is_empty()
    }

    pub fn art_index(&self, index: usize) -> Option<&Arc<RgbaImage>> {
        if self.art.is_empty() {
            None
        } else {
            self.art.get(index % self.art.len())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleItem {
    pub rect: (i32, i32, i32, i32),
    pub row: (i32, i32),
    pub command: UiCommand,
}

const MAIN_TITLE_ITEMS: &[TitleItem] = &[
    TitleItem { rect: (850, 100, 1012, 128), row: (840, 103), command: UiCommand::StartMain },
    TitleItem { rect: (850, 130, 1012, 158), row: (840, 133), command: UiCommand::OpenLoad },
    TitleItem { rect: (850, 160, 1012, 188), row: (840, 161), command: UiCommand::OpenConfig },
    TitleItem { rect: (850, 187, 1012, 215), row: (840, 188), command: UiCommand::OpenExtra },
    TitleItem { rect: (850, 217, 1012, 245), row: (850, 219), command: UiCommand::StartPlusMosaic },
    TitleItem { rect: (850, 245, 1012, 272), row: (840, 245), command: UiCommand::Quit },
];

const PM_TITLE_ITEMS: &[TitleItem] = &[
    TitleItem { rect: (850, 100, 1012, 128), row: (840, 103), command: UiCommand::StartMain },
    TitleItem { rect: (850, 130, 1012, 158), row: (840, 133), command: UiCommand::OpenLoad },
    TitleItem { rect: (850, 160, 1012, 188), row: (840, 161), command: UiCommand::OpenConfig },
    TitleItem { rect: (850, 187, 1012, 215), row: (840, 188), command: UiCommand::OpenExtra },
    TitleItem { rect: (850, 217, 1012, 245), row: (850, 219), command: UiCommand::StartPlusMosaic },
    TitleItem { rect: (850, 245, 1012, 272), row: (840, 245), command: UiCommand::Quit },
];

pub fn title_items(edition: usize) -> &'static [TitleItem] {
    if edition == 1 {
        PM_TITLE_ITEMS
    } else {
        MAIN_TITLE_ITEMS
    }
}

/// The 36 tracks baked into `music1.png`..`music3.png`, in page order.
pub const OST_TRACKS: [(&str, &str); 36] = [
    ("edenBGM003.ogg", "Presentient"),
    ("edenBGM004.ogg", "Solitude"),
    ("edenBGM005.ogg", "Simply"),
    ("edenBGM006.ogg", "Elica"),
    ("edenBGM006_2.ogg", "Felix"),
    ("edenBGM007.ogg", "Android"),
    ("edenBGM008.ogg", "Rule"),
    ("edenBGM008_2.ogg", "Instruction"),
    ("edenBGM009.ogg", "Past desire"),
    ("edenBGM010.ogg", "Bird cage"),
    ("edenBGM011.ogg", "Unstable"),
    ("edenBGM012.ogg", "Geniality"),
    ("edenBGM013.ogg", "Yearning to the sky"),
    ("edenBGM014.ogg", "Lavinia"),
    ("edenBGM015.ogg", "Separation"),
    ("edenBGM016.ogg", "To the new world"),
    ("edenBGM017.ogg", "Liberating"),
    ("edenBGM018.ogg", "Nostalgia feeling"),
    ("edenBGM019a.ogg", "Calm talking"),
    ("edenBGM019b.ogg", "Silent night"),
    ("edenBGM020.ogg", "Maya"),
    ("edenBGM021.ogg", "Lively girl"),
    ("edenBGM021b.ogg", "Sion"),
    ("edenBGM022b.ogg", "Other side of sadness"),
    ("edenBGM023.ogg", "Sleeping Beauty"),
    ("edenBGM024.ogg", "Last wish"),
    ("edenBGM025.ogg", "Estranged"),
    ("edenBGM026_A.ogg", "Can't leave you alone"),
    ("edenBGM026_E3.ogg", "Time left"),
    ("edenBGM026_F2.ogg", "Eternal sleep"),
    ("edenBGM027_A.ogg", "Desire"),
    ("edenBGM027_B.ogg", "Burial man"),
    ("edenBGM028.ogg", "You laugh under sky"),
    ("edenBGM029.ogg", "For you now"),
    ("edenBGM030.ogg", "Bonds of knife and gun"),
    ("edenBGM031.ogg", "Miracle"),
];

pub fn ost_title(file: &str) -> Option<&'static str> {
    OST_TRACKS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(file))
        .map(|(_, title)| *title)
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
    pub text_speed_unread: u8,
    pub text_speed_read: u8,
    pub auto_speed: u8,
    pub fullscreen: bool,
    pub show_cg_in_skip: bool,
    pub special_effects: bool,
    pub text_shadow: bool,
    pub animation_effects: bool,
    pub play_voice_in_backlog: bool,
    pub stop_voice_next_line: bool,
    pub keep_running: bool,
    pub mute_bgm: bool,
    pub mute_voice: bool,
    pub mute_se: bool,
    pub auto_mode: bool,
    pub skip_mode: bool,
    pub font_index: usize,
    /// 诗音 / 榊名亮 / 艾丽卡 / 稻叶直人 / 拉维 / 塔野真夜 / 夏目 / 其他
    pub voice_actors: [bool; 8],
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            bgm_volume: 80,
            se_volume: 80,
            voice_volume: 90,
            text_speed_unread: 60,
            text_speed_read: 80,
            auto_speed: 60,
            fullscreen: true,
            show_cg_in_skip: false,
            special_effects: true,
            text_shadow: true,
            animation_effects: true,
            play_voice_in_backlog: true,
            stop_voice_next_line: false,
            keep_running: true,
            mute_bgm: false,
            mute_voice: false,
            mute_se: false,
            auto_mode: false,
            skip_mode: false,
            font_index: 0,
            voice_actors: [true; 8],
        }
    }
}

/// Images loaded from `sys.paz` / `bg.paz` by the runtime.
#[derive(Default)]
pub struct UiAssets {
    pub title_sets: Vec<TitleSet>,
    pub title_edition: usize,
    pub save_base: Vec<Arc<RgbaImage>>,
    pub load_base: Vec<Arc<RgbaImage>>,
    /// `slmenu0.png` (main) / `slmenu1.png` (PLUS+MOSAIC).
    pub save_menu: Vec<Arc<RgbaImage>>,
    /// `slpage00.png` (QuickSave) followed by `slpage01.png`..`slpage09.png`.
    pub save_pages: Vec<Arc<RgbaImage>>,
    /// `saveloadSelect.png`, the native green slot selection frame.
    pub slot_select: Option<Arc<RgbaImage>>,
    /// `notsaved.png`, the native "nodata" thumbnail.
    pub notsaved: Option<Arc<RgbaImage>>,
    pub music_base: Option<Arc<RgbaImage>>,
    pub music_pages: Vec<Arc<RgbaImage>>,
    pub music_pages_over: Vec<Arc<RgbaImage>>,
    pub cg_base: Vec<Arc<RgbaImage>>,
    pub cg_box: Vec<Arc<RgbaImage>>,
    pub cg_menu: Vec<Arc<RgbaImage>>,
    pub cg_mode: usize,
    pub config_base: Option<Arc<RgbaImage>>,
    /// `checkmark_small.png`
    pub checkmark: Option<Arc<RgbaImage>>,
    /// `knob.png`
    pub knob: Option<Arc<RgbaImage>>,
    pub msg_panel: Option<Arc<RgbaImage>>,
    pub backlog_gauge: Option<Arc<RgbaImage>>,
    pub icon_auto: Option<Arc<RgbaImage>>,
    pub icon_skip: Option<Arc<RgbaImage>>,
    pub icon_save: Option<Arc<RgbaImage>>,
    pub icon_load: Option<Arc<RgbaImage>>,
    pub icon_system: Option<Arc<RgbaImage>>,
    pub icon_back: Option<Arc<RgbaImage>>,
    /// `icons.png`: LOAD/SAVE/LOG/SKIP/SYSTEM strip for the message panel.
    pub msg_icons: Option<Arc<RgbaImage>>,
    /// `iconson.png`: 70x29 hover bar for the strip above.
    pub msg_icon_son: Option<Arc<RgbaImage>>,
    /// `pauseAnime.png`: 12x22 frame click-wait cursor.
    pub wait_cursor: Option<Arc<RgbaImage>>,
    /// `pauseAnimeNextPage.png`: 9x22 frame page-flip cursor.
    pub next_cursor: Option<Arc<RgbaImage>>,
    /// `SelectFocus.png` / `SelectActive.png` / `SelectBLur.png` (800x50).
    pub select_focus: Option<Arc<RgbaImage>>,
    pub select_active: Option<Arc<RgbaImage>>,
    pub select_blur: Option<Arc<RgbaImage>>,
    pub music_level: Option<Arc<RgbaImage>>,
    pub music_note: Option<Arc<RgbaImage>>,
    pub album_back: Option<Arc<RgbaImage>>,
    pub album_next: Option<Arc<RgbaImage>>,
    pub ball: Option<Arc<RgbaImage>>,
    /// `gameMenuBack.png` / `gameMenuBackAuto.png` (172x169).
    pub radial_menu: Vec<Arc<RgbaImage>>,
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
    pub playing_music: Option<usize>,
    pub music_progress: f32,
    pub music_energy: f32,
    pub toast: Option<String>,
    pub toast_frames: u32,
    pub cursor: (i32, i32),
    pub cursor_visible: bool,
    /// 60 Hz frame counter used by native cursor/level animations.
    pub frames: u64,
    /// True while the VM is waiting for a click at the end of a message.
    pub waiting: bool,
    /// The message panel (or the full-screen panel) is on screen.
    pub panel_visible: bool,
    /// Main (0) or PLUS+MOSAIC (1) title artwork.
    pub title_edition: usize,
    /// Which of the three title artworks is shown.
    pub title_art: usize,
    pub radial: bool,
    pub radial_pos: (i32, i32),
    pub radial_hover: Option<usize>,
}

impl Default for UiModel {
    fn default() -> Self {
        Self {
            screen: UiScreen::Title,
            previous: UiScreen::Title,
            hover: None,
            page: 0,
            slots: (0..SLOT_COUNT + 1)
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
            playing_music: None,
            music_progress: 0.0,
            music_energy: 0.0,
            toast: None,
            toast_frames: 0,
            cursor: (0, 0),
            cursor_visible: false,
            frames: 0,
            waiting: false,
            panel_visible: false,
            title_edition: 0,
            title_art: 0,
            radial: false,
            radial_pos: (0, 0),
            radial_hover: None,
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
        self.frames = self.frames.wrapping_add(1);
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
        if self.radial {
            self.radial_hover = radial_hit(self.radial_pos, x, y);
        }
        self.hover = self.hit(x, y).map(|(i, _)| i);
    }

    /// Handle a primary tap. Returns the action to apply.
    pub fn tap(&mut self, x: i32, y: i32) -> Option<UiCommand> {
        self.cursor = (x, y);
        self.cursor_visible = true;
        if self.radial {
            if let Some(index) = radial_hit(self.radial_pos, x, y) {
                self.radial = false;
                self.radial_hover = None;
                return Some(radial_command(index));
            }
            self.radial = false;
            self.radial_hover = None;
            return Some(UiCommand::RadialClose);
        }
        let (index, command) = self.hit(x, y)?;
        self.hover = Some(index);
        Some(command)
    }

    /// Secondary click: open the native radial menu at the pointer.
    pub fn right_click(&mut self, x: i32, y: i32) -> Option<UiCommand> {
        if self.screen != UiScreen::Game || self.hide_ui {
            return None;
        }
        self.radial = !self.radial;
        if self.radial {
            let max_x = (1024 - 172).max(0);
            let max_y = (640 - 169).max(0);
            self.radial_pos = (x.clamp(0, max_x), y.clamp(0, max_y));
            self.radial_hover = radial_hit(self.radial_pos, x, y);
            Some(UiCommand::RadialOpen(x, y))
        } else {
            Some(UiCommand::RadialClose)
        }
    }

    pub fn set_title_edition(&mut self, edition: usize) {
        self.title_edition = edition.min(1);
        self.title_art = 0;
        self.hover = None;
    }

    pub fn cycle_title_art(&mut self, delta: i32, count: usize) {
        if count == 0 {
            return;
        }
        let next = self.title_art as i32 + delta;
        self.title_art = next.rem_euclid(count as i32) as usize;
        self.hover = None;
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
        // Native BACK/NEXT artwork selector at the top-left.
        if contains((24, 50, 82, 70), x, y) {
            return Some((90, UiCommand::TitleArt(-1)));
        }
        if contains((150, 50, 208, 70), x, y) {
            return Some((91, UiCommand::TitleArt(1)));
        }
        if contains((18, 14, 102, 40), x, y) {
            return Some((92, UiCommand::TitleEdition(0)));
        }
        if contains((108, 14, 266, 40), x, y) {
            return Some((93, UiCommand::TitleEdition(1)));
        }
        for (i, item) in title_items(self.title_edition).iter().enumerate() {
            if contains(item.rect, x, y) {
                return Some((i, item.command.clone()));
            }
        }
        None
    }

    fn hit_game(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        if self.radial {
            return None;
        }
        // Native icons.png strip at the right edge of the message panel.
        if self.panel_visible {
            for (i, command) in MSG_ICONS.iter().enumerate() {
                if contains(msg_icon_rect(i), x, y) {
                    return Some((20 + i, command.clone()));
                }
            }
        }
        None
    }

    fn hit_slot(&self, x: i32, y: i32, save: bool) -> Option<(usize, UiCommand)> {
        for i in 0..SLOT_PAGE_SIZE {
            if contains(slot_rect(i), x, y) {
                let index = self.page * SLOT_PAGE_SIZE + i;
                let command = if save {
                    UiCommand::SaveSlot(index)
                } else {
                    UiCommand::LoadSlot(index)
                };
                return Some((i, command));
            }
        }
        // QuickSave label on the save screen (slpage00.png at 412,568).
        if save && contains((412, 568, 532, 608), x, y) {
            return Some((80, UiCommand::QuickSave));
        }
        // slmenu0/1.png is composited at (512, 579).
        if contains((542, 578, 706, 640), x, y) {
            return Some((81, UiCommand::PrevPage));
        }
        if contains((732, 578, 840, 640), x, y) {
            return Some((82, UiCommand::NextPage));
        }
        if contains((852, 578, 960, 640), x, y) {
            return Some((83, UiCommand::Back));
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
        if contains((690, 160, 736, 208), x, y) {
            return Some((1, UiCommand::PrevMusic));
        }
        if contains((752, 160, 798, 208), x, y) {
            return Some((2, UiCommand::Continue));
        }
        if contains((816, 160, 862, 208), x, y) {
            return Some((3, UiCommand::StopMusic));
        }
        if contains((880, 160, 926, 208), x, y) {
            return Some((4, UiCommand::NextMusic));
        }
        for row in 0..MUSIC_ROWS {
            let index = self.page * MUSIC_ROWS + row;
            if contains((72, music_row_y(row) - 8, 600, music_row_y(row) + 20), x, y) {
                if let Some(entry) = self.music.get(index) {
                    if entry.unlocked {
                        return Some((10 + row, UiCommand::PlayMusic(entry.file.clone())));
                    }
                }
            }
        }
        // Page labels baked into music1/2/3.png.
        if contains((604, 578, 706, 610), x, y) && self.page > 0 {
            return Some((50, UiCommand::PrevPage));
        }
        if contains((732, 578, 836, 610), x, y) && (self.page + 1) * MUSIC_ROWS < self.music.len()
        {
            return Some((51, UiCommand::NextPage));
        }
        if contains((852, 578, 940, 610), x, y) {
            return Some((52, UiCommand::Back));
        }
        None
    }

    fn hit_config(&self, x: i32, y: i32) -> Option<(usize, UiCommand)> {
        // Sliders measured from configBase.png: the groove is at y=120/195/270
        // on the left and 116/191/266 on the right, with a 15x25 native knob.
        let sliders = [
            ((144, 108, 342, 132), 3),
            ((144, 183, 342, 207), 4),
            ((144, 258, 342, 282), 5),
            ((680, 104, 878, 128), 0),
            ((680, 179, 878, 203), 2),
            ((680, 254, 878, 278), 1),
        ];
        for (i, (rect, index)) in sliders.iter().enumerate() {
            if contains(*rect, x, y) {
                let mid = (rect.0 + rect.2) / 2;
                let delta = if x < mid { -5 } else { 5 };
                return Some((1 + i, UiCommand::ConfigAdjust(*index, delta)));
            }
        }
        // TEST buttons.
        for (i, rect) in CONFIG_TEST_RECTS.iter().enumerate() {
            if contains(*rect, x, y) {
                return Some((10 + i, UiCommand::ConfigTest(i)));
            }
        }
        // Checkboxes, measured from the baked squares.
        for (i, rect) in CONFIG_CHECK_RECTS.iter().enumerate() {
            if contains(*rect, x, y) {
                return Some((20 + i, UiCommand::ConfigToggle(i)));
            }
        }
        // Font up/down arrows.
        if contains((340, 440, 372, 462), x, y) {
            return Some((60, UiCommand::ConfigFont(-1)));
        }
        if contains((340, 470, 372, 496), x, y) {
            return Some((61, UiCommand::ConfigFont(1)));
        }
        if contains((690, 552, 752, 600), x, y) {
            return Some((70, UiCommand::ConfigConfirm));
        }
        if contains((800, 552, 875, 600), x, y) {
            return Some((71, UiCommand::ConfigCancel));
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

/// `icons.png` rows on the message panel: LOAD, SAVE, LOG, SKIP, SYSTEM.
const MSG_ICONS: &[UiCommand] = &[
    UiCommand::OpenLoad,
    UiCommand::OpenSave,
    UiCommand::OpenBacklog,
    UiCommand::SetSkip(true),
    UiCommand::OpenConfig,
];

const MSG_ICON_X: i32 = 946;
const MSG_ICON_Y: i32 = 488;
const MSG_ICON_W: i32 = 70;
const MSG_ICON_H: i32 = 29;

fn msg_icon_rect(index: usize) -> (i32, i32, i32, i32) {
    let y = MSG_ICON_Y + index as i32 * MSG_ICON_H;
    (MSG_ICON_X, y, MSG_ICON_X + MSG_ICON_W, y + MSG_ICON_H)
}

/// Checkbox hit rects in the same order as [`config_checkbox_rect`].
const CONFIG_CHECK_RECTS: &[(i32, i32, i32, i32)] = &[
    (414, 77, 433, 95),   // 全屏
    (414, 121, 433, 139), // 窗口
    (414, 206, 433, 224), // 在跳过模式中显示CG
    (414, 249, 433, 267), // 特殊效果
    (414, 292, 433, 310), // 文字阴影
    (414, 334, 433, 352), // 动画效果
    (414, 421, 433, 439), // 在回放模式中播放语音
    (414, 476, 433, 494), // 在下一行停止语音
    (414, 569, 433, 587), // 后台保持游戏运行
    (135, 544, 154, 562), // 自动
    (135, 578, 154, 596), // 跳过
    (780, 74, 796, 90),   // BGM 静音
    (780, 149, 796, 165), // 语音 静音
    (780, 222, 796, 238), // 效果音 静音
    (669, 386, 687, 404), // 诗音
    (790, 386, 808, 404), // 榊名亮
    (669, 423, 687, 441), // 艾丽卡
    (790, 423, 808, 441), // 稻叶直人
    (669, 461, 687, 479), // 拉维
    (790, 461, 808, 479), // 塔野真夜
    (669, 499, 687, 517), // 夏目
    (790, 499, 808, 517), // 其他
];

/// Inner rectangle of the baked checkbox (used to place the native check).
fn config_checkbox_rect(index: usize) -> (i32, i32, i32, i32) {
    match index {
        0 => (417, 80, 430, 92),
        1 => (417, 124, 430, 136),
        2 => (417, 209, 430, 221),
        3 => (417, 252, 430, 264),
        4 => (417, 295, 430, 307),
        5 => (417, 337, 430, 349),
        6 => (417, 424, 430, 436),
        7 => (417, 479, 430, 491),
        8 => (417, 572, 430, 584),
        9 => (138, 547, 150, 558),
        10 => (138, 581, 150, 592),
        11 => (783, 77, 793, 86),
        12 => (783, 152, 793, 161),
        13 => (783, 225, 793, 234),
        14 => (672, 389, 684, 401),
        15 => (793, 389, 805, 401),
        16 => (672, 426, 684, 438),
        17 => (793, 426, 805, 438),
        18 => (672, 464, 684, 476),
        19 => (793, 464, 805, 476),
        20 => (672, 502, 684, 514),
        21 => (793, 502, 805, 514),
        _ => (0, 0, 0, 0),
    }
}

const CONFIG_TEST_RECTS: &[(i32, i32, i32, i32)] = &[
    (845, 66, 905, 96),   // BGM TEST
    (845, 146, 905, 176), // 语音 TEST
    (845, 221, 905, 251), // 效果音 TEST
];

const BACKLOG_TOP: i32 = 70;
const BACKLOG_ROWS: usize = 18;
const BACKLOG_ROW_H: i32 = 29;
const GALLERY_COLS: usize = 5;
const GALLERY_ROWS: usize = 5;
const MUSIC_ROWS: usize = 16;

/// 9 pages of 10 slots, plus a dedicated QuickSave slot.
pub const SLOT_PAGE_SIZE: usize = 10;
pub const SLOT_PAGES: usize = 9;
pub const SLOT_COUNT: usize = SLOT_PAGE_SIZE * SLOT_PAGES;
pub const QUICK_SLOT: usize = SLOT_COUNT;

/// Exact white card rectangles measured from `saveBase0.png`.
pub fn slot_rect(index: usize) -> (i32, i32, i32, i32) {
    let col = index % 2;
    let row = index / 2;
    let x = if col == 0 { 145 } else { 539 };
    let y = 48 + row as i32 * 106;
    (x, y, x + 343, y + 95)
}

pub fn music_row_y(row: usize) -> i32 {
    42 + row as i32 * 32
}

fn gallery_rect(cell: usize) -> (i32, i32, i32, i32) {
    let col = cell % GALLERY_COLS;
    let row = cell / GALLERY_COLS;
    // Slots are baked into cgmode0box.png/cgmode1box.png (5x5, 128x80).
    let x = 96 + col as i32 * 176;
    let y = 30 + row as i32 * 113;
    (x, y, x + 128, y + 80)
}

/// Hit test for the 172x169 `gameMenuBack*.png` radial menu.
fn radial_hit(pos: (i32, i32), x: i32, y: i32) -> Option<usize> {
    let (px, py) = pos;
    let lx = x - px;
    let ly = y - py;
    if !(0..172).contains(&lx) || !(0..169).contains(&ly) {
        return None;
    }
    // SKIP/AUTO (top-left), SAVE (left), LOAD (bottom-left), Q.SAVE (right),
    // SYSTEM (bottom-right) as painted into the native sprite.
    if lx < 66 && ly < 44 {
        Some(0)
    } else if lx < 58 && (44..100).contains(&ly) {
        Some(1)
    } else if lx < 82 && ly >= 128 {
        Some(2)
    } else if lx >= 118 && (40..104).contains(&ly) {
        Some(3)
    } else if lx >= 108 && (104..156).contains(&ly) {
        Some(4)
    } else {
        None
    }
}

fn radial_command(index: usize) -> UiCommand {
    match index {
        0 => UiCommand::SetSkip(true),
        1 => UiCommand::OpenSave,
        2 => UiCommand::OpenLoad,
        3 => UiCommand::QuickSave,
        4 => UiCommand::OpenConfig,
        _ => UiCommand::RadialClose,
    }
}

/// Compose the current UI over `game_frame` (or a title/gallery base).
pub fn render(
    model: &UiModel,
    assets: &UiAssets,
    game_frame: &RgbaImage,
    font: Option<&FontRenderer>,
) -> RgbaImage {
    let mut out = match model.screen {
        UiScreen::Title => {
            let set = assets
                .title_sets
                .get(model.title_edition)
                .filter(|s| !s.is_empty())
                .or_else(|| assets.title_sets.iter().find(|s| !s.is_empty()));
            match set.and_then(|s| s.art_index(model.title_art)) {
                Some(image) => (**image).clone(),
                None => RgbaImage::filled(1024, 640, [0, 0, 0, 255]),
            }
        }
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
        UiScreen::Music => {
            let page = assets.music_pages.get(model.page);
            clone_or_black(page.or(assets.music_base.as_ref()))
        }
        UiScreen::Config => clone_or_black(assets.config_base.as_ref().or(assets.extra.as_ref())),
        UiScreen::Extra => clone_or_black(assets.extra.as_ref()),
        UiScreen::Game | UiScreen::Backlog => game_frame.clone(),
    };
    match model.screen {
        UiScreen::Title => draw_title(&mut out, model, assets, font),
        UiScreen::Game => draw_game(&mut out, model, assets, font),
        UiScreen::Save | UiScreen::Load => {
            draw_slots(&mut out, model, assets, font, model.screen == UiScreen::Save)
        }
        UiScreen::Backlog => draw_backlog(&mut out, model, assets, font),
        UiScreen::Gallery => draw_gallery(&mut out, model, font),
        UiScreen::GalleryView => draw_gallery_view(&mut out, model, font),
        UiScreen::Music => draw_music(&mut out, model, assets, font),
        UiScreen::Config => draw_config(&mut out, model, assets, font),
        UiScreen::Extra => draw_extra(&mut out, model, font),
    }
    if model.radial {
        draw_radial(&mut out, model, assets);
    }
    if let Some(toast) = &model.toast {
        draw_toast(&mut out, toast, font);
    }
    if model.cursor_visible && !model.radial {
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
    let set = assets
        .title_sets
        .get(model.title_edition)
        .filter(|s| !s.is_empty())
        .or_else(|| assets.title_sets.iter().find(|s| !s.is_empty()));
    let items = title_items(model.title_edition);
    // The main artwork has no PLUS+MOSAIC row; composite the native PM text
    // mask at the exact baked coordinates.
    if model.title_edition == 0 {
        if let Some(mask) = set.and_then(|s| s.plus_mosaic.as_ref()) {
            let opacity = if model.hover == Some(4) { 255 } else { 225 };
            out.blit(mask, 856, 223, opacity);
        }
    }
    // Native `topmenuNover.png` rows are the retail hover state: blit just the
    // hovered row so the rest of the artwork stays untouched.
    if let (Some(index), Some(set)) = (model.hover, set) {
        if let Some(item) = items.get(index) {
            if let Some(over) = set.over.get(model.title_art % set.over.len().max(1)) {
                let (x0, y0) = item.row;
                let w = 172;
                let h = 22;
                let sx = x0.max(0);
                let sy = y0.max(0);
                let sw = w.min(over.width as i32 - sx).max(0) as u32;
                let sh = h.min(over.height as i32 - sy).max(0) as u32;
                if sw > 0 && sh > 0 {
                    let patch = over.crop(sx as u32, sy as u32, sw, sh);
                    out.blit(&patch, x0, y0, 255);
                }
            }
        }
    }
    // Edition selector and artwork paging, using the native album buttons.
    // Native blue glow (iconson.png) behind the active edition tab.
    if let Some(glow) = &assets.msg_icon_son {
        let (active_x, active_w) = if model.title_edition == 0 {
            (18, 84)
        } else {
            (108, 158)
        };
        blit_scaled(out, glow, active_x, 15, active_w, 24);
    }
    if let Some(font) = font {
        font.draw_text(out, "eden*", 30.0, 18.0, 15.0, [255, 250, 236, 255], 90.0);
        font.draw_text(
            out,
            "PLUS+MOSAIC",
            126.0,
            18.0,
            15.0,
            [255, 242, 206, 255],
            160.0,
        );
    }
    if let Some(prev) = &assets.album_back {
        blit_scaled(out, prev, 24, 50, 58, 17);
    }
    if let Some(next) = &assets.album_next {
        blit_scaled(out, next, 150, 50, 58, 17);
    }
    if let Some(font) = font {
        font.draw_text(
            out,
            &format!(
                "{}/{}",
                model.title_art + 1,
                set.map(|s| s.art.len()).unwrap_or(0)
            ),
            96.0,
            52.0,
            12.0,
            [255, 255, 255, 220],
            50.0,
        );
    }
}

fn draw_game(out: &mut RgbaImage, model: &UiModel, assets: &UiAssets, font: Option<&FontRenderer>) {
    if model.hide_ui {
        return;
    }
    if model.panel_visible {
        // Native icons.png strip, right-aligned on the message panel.
        if let Some(icons) = &assets.msg_icons {
            if let Some(index) = model.hover.filter(|i| (20..25).contains(i)) {
                if let Some(son) = &assets.msg_icon_son {
                    blit_at(out, son, MSG_ICON_X, MSG_ICON_Y + (index - 20) as i32 * MSG_ICON_H);
                }
            }
            blit_at(out, icons, MSG_ICON_X, MSG_ICON_Y);
        }
        // 12-frame click-wait cursor, bottom-right of the text area.
        if model.waiting && !model.auto && !model.skip {
            if let Some(sprite) = &assets.wait_cursor {
                let frames = (sprite.width / 22).max(1);
                let frame = ((model.frames / 7) % u64::from(frames)) as u32;
                let patch = sprite.crop(frame * 22, 0, 22, 22);
                blit_at(out, &patch, 910, 600);
            }
        }
        if model.auto || model.skip {
            if let Some(font) = font {
                let (label, color) = if model.skip {
                    ("SKIP", [255, 210, 120, 235])
                } else {
                    ("AUTO", [180, 240, 190, 235])
                };
                font.draw_text(out, label, 930.0, 600.0, 14.0, color, 60.0);
            }
        }
        return;
    }
    // Retail shows no persistent overlay when the message window is hidden;
    // the native radial menu is reached with a right click / two-finger tap.
}

fn draw_radial(out: &mut RgbaImage, model: &UiModel, assets: &UiAssets) {
    let index = if model.auto && assets.radial_menu.len() > 1 { 1 } else { 0 };
    if let Some(menu) = assets.radial_menu.get(index) {
        out.blit(menu, model.radial_pos.0, model.radial_pos.1, 255);
    }
    if let Some(hover) = model.radial_hover {
        let (px, py) = model.radial_pos;
        let rect = match hover {
            0 => (px + 4, py + 4, px + 64, py + 40),
            1 => (px, py + 48, px + 56, py + 96),
            2 => (px + 16, py + 130, px + 80, py + 166),
            3 => (px + 120, py + 44, px + 172, py + 100),
            4 => (px + 110, py + 108, px + 172, py + 152),
            _ => return,
        };
        fill_rect(out, rect, [255, 255, 255, 26]);
    }
}

fn draw_slots(
    out: &mut RgbaImage,
    model: &UiModel,
    assets: &UiAssets,
    font: Option<&FontRenderer>,
    save: bool,
) {
    for cell in 0..SLOT_PAGE_SIZE {
        let index = model.page * SLOT_PAGE_SIZE + cell;
        let rect = slot_rect(cell);
        let Some(slot) = model.slots.get(index) else {
            continue;
        };
        if model.hover == Some(cell) {
            if let Some(frame) = &assets.slot_select {
                blit_at(out, frame, rect.0 - 1, rect.1 - 1);
            }
        }
        // Native 128x80 thumbnail with a crisp inner border.
        let thumb_rect = (rect.0 + 8, rect.1 + 8, rect.0 + 136, rect.1 + 88);
        if let Some(thumb) = &slot.thumbnail {
            stroke_rect(out, thumb_rect, [34, 56, 34, 200]);
            blit_scaled(
                out,
                thumb,
                thumb_rect.0 + 1,
                thumb_rect.1 + 1,
                126,
                78,
            );
        } else if let Some(nodata) = &assets.notsaved {
            blit_at(out, nodata, thumb_rect.0, thumb_rect.1);
        } else {
            fill_rect(out, thumb_rect, [118, 124, 118, 255]);
        }
        let Some(font) = font else {
            continue;
        };
        let text_x = (rect.0 + 146) as f32;
        let width = (rect.2 - rect.0 - 154) as f32;
        let number_color = [26, 42, 30, 255];
        let body_color = [44, 62, 48, 255];
        let faint_color = [96, 112, 98, 255];
        font.draw_text(
            out,
            &format!("{:02}", index + 1),
            text_x,
            (rect.1 + 8) as f32,
            15.0,
            number_color,
            40.0,
        );
        if slot.empty {
            font.draw_text(
                out,
                "No Data",
                text_x + 26.0,
                (rect.1 + 10) as f32,
                13.0,
                faint_color,
                width - 26.0,
            );
            continue;
        }
        let stamp = slot.timestamp.get(..16).unwrap_or(&slot.timestamp);
        font.draw_text(
            out,
            &fit_text(font, stamp, 12.0, width - 26.0),
            text_x + 26.0,
            (rect.1 + 10) as f32,
            12.0,
            body_color,
            width - 26.0,
        );
        let lines = wrap_text(font, &slot.summary.replace(['\n', '\r'], " "), 11.0, width, 2);
        for (line, text) in lines.iter().enumerate() {
            font.draw_text(
                out,
                text,
                text_x,
                (rect.1 + 31 + line as i32 * 14) as f32,
                11.0,
                body_color,
                width,
            );
        }
        font.draw_text(
            out,
            &fit_text(font, &slot.script, 9.5, width),
            text_x,
            (rect.1 + 78) as f32,
            9.5,
            faint_color,
            width,
        );
    }
    // Native Back/Next/Return bar, then the page label on top of its green
    // left padding so long labels such as `QuickSave` are never clipped.
    let menu_index = assets.cg_mode.min(assets.save_menu.len().saturating_sub(1));
    if let Some(menu) = assets.save_menu.get(menu_index) {
        blit_at(out, menu, 512, 579);
    }
    let page_sprite = if save {
        assets.save_pages.first()
    } else {
        assets.save_pages.get(model.page + 1)
    };
    if let Some(sprite) = page_sprite {
        blit_at(out, sprite, 412, 568);
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

/// Greedy wrap into at most `max_lines` lines, ellipsising the last one.
fn wrap_text(
    font: &FontRenderer,
    text: &str,
    size: f32,
    max_width: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let mut candidate = current.clone();
        candidate.push(ch);
        if font.measure_line(&candidate, size) > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current.push(ch);
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines && max_lines > 0 {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && font.measure_line(&format!("{last}…"), size) > max_width {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
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

fn draw_music(
    out: &mut RgbaImage,
    model: &UiModel,
    assets: &UiAssets,
    font: Option<&FontRenderer>,
) {
    // The retail pages already carry the track numbers and real OST names.
    // Only the selection tint and now-playing panel are composited.
    for row in 0..MUSIC_ROWS {
        let index = model.page * MUSIC_ROWS + row;
        if model.music.get(index).is_none() {
            break;
        }
        let y = music_row_y(row) - 7;
        if model.playing_music == Some(index) {
            if let Some(active) = &assets.select_active {
                blit_scaled(out, active, 76, y, 524, 26);
            }
        } else if model.hover == Some(10 + row) {
            if let Some(focus) = &assets.select_focus {
                blit_scaled(out, focus, 76, y, 524, 26);
            }
        }
        if model.playing_music == Some(index) {
            if let Some(note) = &assets.music_note {
                blit_at(out, note, 62, music_row_y(row) + 2);
            }
        }
    }
    // Now-playing block in the empty upper-right area of the native page.
    if let Some(index) = model.playing_music {
        if let Some(entry) = model.music.get(index) {
            if let Some(font) = font {
                font.draw_text(
                    out,
                    "Track",
                    700.0,
                    34.0,
                    14.0,
                    [228, 238, 228, 220],
                    80.0,
                );
                font.draw_text(
                    out,
                    &entry.title,
                    700.0,
                    54.0,
                    20.0,
                    [255, 255, 255, 245],
                    260.0,
                );
                font.draw_text(
                    out,
                    &format!("{:02}/36", index + 1),
                    700.0,
                    80.0,
                    12.0,
                    [200, 216, 200, 210],
                    80.0,
                );
            }
            // Native vertical level meter (1x100 gradient) driven by the
            // decoded BGM envelope.
            if let Some(level) = &assets.music_level {
                let h = (model.music_energy.clamp(0.0, 1.0) * 100.0) as u32;
                if h > 1 {
                    let src = level.crop(0, 100 - h, 1, h);
                    blit_scaled(out, &src, 928, 150 - h as i32, 3, h);
                }
            }
            // Progress bar under the track title.
            fill_rect(out, (700, 102, 926, 104), [210, 224, 210, 120]);
            let filled = 226.0 * model.music_progress.clamp(0.0, 1.0);
            fill_rect(
                out,
                (700, 102, 700 + filled as i32, 104),
                [245, 250, 235, 230],
            );
        }
    }
    // Three native dots pulse while a track is playing.
    if model.playing_music.is_some() {
        if let Some(ball) = &assets.ball {
            for i in 0..3 {
                let phase = ((model.frames / 6 + i as u64) % 9) as u8;
                let alpha = 90 + phase * 18;
                let patch = ball.crop(3, 3, 8, 8);
                out.blit(&patch, 700 + i as i32 * 10, 112, alpha.min(230));
            }
        }
    }
}

fn draw_config(
    out: &mut RgbaImage,
    model: &UiModel,
    assets: &UiAssets,
    font: Option<&FontRenderer>,
) {
    let knob = |out: &mut RgbaImage, x0: i32, x1: i32, y: i32, value: u8| {
        if let Some(knob) = &assets.knob {
            let x = x0 + ((x1 - x0) as f32 * value as f32 / 100.0).round() as i32;
            blit_at(out, knob, x - 7, y - 12);
        } else {
            let x = x0 + ((x1 - x0) as f32 * value as f32 / 100.0) as i32;
            fill_rect(out, (x - 2, y - 12, x + 3, y + 12), [250, 250, 250, 235]);
        }
    };
    // Measured grooves: left x=144..342 at y=120/195/270, right x=680..878
    // at y=116/191/266.
    knob(out, 144, 342, 120, model.config.text_speed_unread);
    knob(out, 144, 342, 195, model.config.text_speed_read);
    knob(out, 144, 342, 270, model.config.auto_speed);
    knob(out, 680, 878, 116, model.config.bgm_volume);
    knob(out, 680, 878, 191, model.config.voice_volume);
    knob(out, 680, 878, 266, model.config.se_volume);
    // Native checkmark_small.png, scaled into the baked checkbox interior.
    let checked = [
        model.config.fullscreen,
        !model.config.fullscreen,
        model.config.show_cg_in_skip,
        model.config.special_effects,
        model.config.text_shadow,
        model.config.animation_effects,
        model.config.play_voice_in_backlog,
        model.config.stop_voice_next_line,
        model.config.keep_running,
        model.config.auto_mode || model.auto,
        model.config.skip_mode || model.skip,
        model.config.mute_bgm,
        model.config.mute_voice,
        model.config.mute_se,
        model.config.voice_actors[0],
        model.config.voice_actors[1],
        model.config.voice_actors[2],
        model.config.voice_actors[3],
        model.config.voice_actors[4],
        model.config.voice_actors[5],
        model.config.voice_actors[6],
        model.config.voice_actors[7],
    ];
    if let Some(mark) = &assets.checkmark {
        for (i, on) in checked.iter().enumerate() {
            if !on {
                continue;
            }
            let (x0, y0, x1, y1) = config_checkbox_rect(i);
            let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
            blit_scaled(out, mark, x0, y0, w, h);
        }
    }
    if let Some(index) = model.hover {
        if (20..42).contains(&index) {
            stroke_rect(out, CONFIG_CHECK_RECTS[index - 20], [255, 255, 255, 120]);
        } else if (1..7).contains(&index) {
            let rect = match index {
                1 => (144, 108, 342, 132),
                2 => (144, 183, 342, 207),
                3 => (144, 258, 342, 282),
                4 => (680, 104, 878, 128),
                5 => (680, 179, 878, 203),
                6 => (680, 254, 878, 278),
                _ => (0, 0, 0, 0),
            };
            stroke_rect(out, rect, [120, 240, 150, 140]);
        } else if (10..13).contains(&index) {
            stroke_rect(out, CONFIG_TEST_RECTS[index - 10], [255, 255, 255, 140]);
        }
    }
    if let Some(font) = font {
        let font_names = ["明朝", "ゴシック", "丸ゴシック"];
        let name = font_names[model.config.font_index.min(font_names.len() - 1)];
        font.draw_text(out, name, 380.0, 446.0, 14.0, [240, 244, 236, 230], 90.0);
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
        assert_eq!(ui.tap(900, 230), Some(UiCommand::StartPlusMosaic));
        assert_eq!(ui.tap(900, 258), Some(UiCommand::Quit));
        // Native edition and artwork selectors.
        assert_eq!(ui.tap(60, 30), Some(UiCommand::TitleEdition(0)));
        assert_eq!(ui.tap(160, 30), Some(UiCommand::TitleEdition(1)));
        assert_eq!(ui.tap(40, 58), Some(UiCommand::TitleArt(-1)));
        assert_eq!(ui.tap(170, 58), Some(UiCommand::TitleArt(1)));
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
        assert_eq!(ui.tap(200, 120), Some(UiCommand::ConfigAdjust(3, -5)));
        assert_eq!(ui.tap(800, 116), Some(UiCommand::ConfigAdjust(0, 5)));
        assert_eq!(ui.tap(200, 195), Some(UiCommand::ConfigAdjust(4, -5)));
        assert_eq!(ui.tap(800, 266), Some(UiCommand::ConfigAdjust(1, 5)));
        assert_eq!(ui.tap(420, 85), Some(UiCommand::ConfigToggle(0)));
        assert_eq!(ui.tap(143, 552), Some(UiCommand::ConfigToggle(9)));
        assert_eq!(ui.tap(870, 80), Some(UiCommand::ConfigTest(0)));
        assert_eq!(ui.tap(720, 570), Some(UiCommand::ConfigConfirm));
        assert_eq!(ui.tap(830, 570), Some(UiCommand::ConfigCancel));
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
        assert_eq!(ui.tap(450, 580), Some(UiCommand::QuickSave));
        assert_eq!(ui.tap(900, 600), Some(UiCommand::Back));
        assert_eq!(ui.tap(780, 600), Some(UiCommand::NextPage));
        ui.page = 1;
        assert_eq!(ui.tap(200, 60), Some(UiCommand::SaveSlot(10)));
        assert_eq!(ui.tap(600, 60), Some(UiCommand::SaveSlot(11)));
        assert_eq!(ui.tap(620, 600), Some(UiCommand::PrevPage));
    }

    #[test]
    fn music_uses_the_retail_ost_names() {
        assert_eq!(ost_title("edenBGM003.ogg"), Some("Presentient"));
        assert_eq!(ost_title("edenBGM031.ogg"), Some("Miracle"));
        assert_eq!(ost_title("edenBGM_H4.ogg"), None);
        assert_eq!(OST_TRACKS.len(), 36);
    }

    #[test]
    fn radial_menu_opens_and_targets_native_labels() {
        let mut ui = UiModel::default();
        ui.screen = UiScreen::Game;
        assert_eq!(ui.right_click(500, 300), Some(UiCommand::RadialOpen(500, 300)));
        assert!(ui.radial);
        // SKIP is painted in the top-left quadrant of gameMenuBack.png.
        assert_eq!(ui.tap(510, 310), Some(UiCommand::SetSkip(true)));
        assert!(!ui.radial);
        // Q.SAVE on the right edge.
        ui.right_click(400, 200);
        assert_eq!(ui.tap(400 + 150, 200 + 60), Some(UiCommand::QuickSave));
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
