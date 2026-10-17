use gpui::{App, Hsla, SharedString, TitlebarOptions, Window, WindowAppearance, point, px, rgb};
use gpui_component::{Colorize, Theme, ThemeMode};
use std::sync::atomic::{AtomicU32, Ordering};

pub const BG: u32 = 0x0F1117;
pub const BG_ELEVATED: u32 = 0x161A23;
pub const BG_PANEL: u32 = 0x1B202B;
pub const BG_TERMINAL: u32 = 0x11141A;
pub const BG_COLUMN: u32 = 0x131821;
pub const BG_HEADER: u32 = 0x181D27;
pub const BG_ROW_HOVER: u32 = 0x202736;
pub const BG_ROW_ACTIVE: u32 = 0x19314E;
pub const BORDER: u32 = 0x2A3040;
pub const BORDER_SOFT: u32 = 0x2D3545;
pub const TEXT: u32 = 0xE7EAF0;
pub const TEXT_MUTED: u32 = 0x97A1B3;
pub const TEXT_DIM: u32 = 0x687286;
// Prefer the system accent once GPUI exposes it directly; blue is the fallback.
pub const ACCENT: u32 = 0x2F80ED;
pub const ORANGE: u32 = 0xE6A35C;
pub const GREEN: u32 = 0x78D891;
pub const RED: u32 = 0xF87171;
pub const STATUS_BAR: u32 = 0x1C1F25;

static DYNAMIC_BG: AtomicU32 = AtomicU32::new(BG);
static DYNAMIC_BG_ELEVATED: AtomicU32 = AtomicU32::new(BG_ELEVATED);
static DYNAMIC_BG_PANEL: AtomicU32 = AtomicU32::new(BG_PANEL);
static DYNAMIC_BG_TERMINAL: AtomicU32 = AtomicU32::new(BG_TERMINAL);
static DYNAMIC_BG_COLUMN: AtomicU32 = AtomicU32::new(BG_COLUMN);
static DYNAMIC_BG_HEADER: AtomicU32 = AtomicU32::new(BG_HEADER);
static DYNAMIC_BG_ROW_HOVER: AtomicU32 = AtomicU32::new(BG_ROW_HOVER);
static DYNAMIC_BG_ROW_ACTIVE: AtomicU32 = AtomicU32::new(BG_ROW_ACTIVE);
static DYNAMIC_BORDER: AtomicU32 = AtomicU32::new(BORDER);
static DYNAMIC_BORDER_SOFT: AtomicU32 = AtomicU32::new(BORDER_SOFT);
static DYNAMIC_TEXT: AtomicU32 = AtomicU32::new(TEXT);
static DYNAMIC_TEXT_MUTED: AtomicU32 = AtomicU32::new(TEXT_MUTED);
static DYNAMIC_TEXT_DIM: AtomicU32 = AtomicU32::new(TEXT_DIM);
static DYNAMIC_ACCENT: AtomicU32 = AtomicU32::new(ACCENT);
static DYNAMIC_ORANGE: AtomicU32 = AtomicU32::new(ORANGE);
static DYNAMIC_GREEN: AtomicU32 = AtomicU32::new(GREEN);
static DYNAMIC_RED: AtomicU32 = AtomicU32::new(RED);
static DYNAMIC_STATUS_BAR: AtomicU32 = AtomicU32::new(STATUS_BAR);

pub fn color(hex: u32) -> Hsla {
    rgb(dynamic_color(hex)).into()
}

pub fn fixed_color(hex: u32) -> Hsla {
    raw_color(hex)
}

fn raw_color(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn dynamic_color(hex: u32) -> u32 {
    match hex {
        BG => DYNAMIC_BG.load(Ordering::Relaxed),
        BG_ELEVATED => DYNAMIC_BG_ELEVATED.load(Ordering::Relaxed),
        BG_PANEL => DYNAMIC_BG_PANEL.load(Ordering::Relaxed),
        BG_TERMINAL => DYNAMIC_BG_TERMINAL.load(Ordering::Relaxed),
        BG_COLUMN => DYNAMIC_BG_COLUMN.load(Ordering::Relaxed),
        BG_HEADER => DYNAMIC_BG_HEADER.load(Ordering::Relaxed),
        BG_ROW_HOVER => DYNAMIC_BG_ROW_HOVER.load(Ordering::Relaxed),
        BG_ROW_ACTIVE => DYNAMIC_BG_ROW_ACTIVE.load(Ordering::Relaxed),
        BORDER => DYNAMIC_BORDER.load(Ordering::Relaxed),
        BORDER_SOFT => DYNAMIC_BORDER_SOFT.load(Ordering::Relaxed),
        TEXT => DYNAMIC_TEXT.load(Ordering::Relaxed),
        TEXT_MUTED => DYNAMIC_TEXT_MUTED.load(Ordering::Relaxed),
        TEXT_DIM => DYNAMIC_TEXT_DIM.load(Ordering::Relaxed),
        ACCENT => DYNAMIC_ACCENT.load(Ordering::Relaxed),
        ORANGE => DYNAMIC_ORANGE.load(Ordering::Relaxed),
        GREEN => DYNAMIC_GREEN.load(Ordering::Relaxed),
        RED => DYNAMIC_RED.load(Ordering::Relaxed),
        STATUS_BAR => DYNAMIC_STATUS_BAR.load(Ordering::Relaxed),
        _ => hex,
    }
}

fn set_dynamic_color(cell: &AtomicU32, value: u32) {
    cell.store(value, Ordering::Relaxed);
}

fn rgba_to_u32(value: Hsla) -> u32 {
    let rgba = value.to_rgb();
    let channel = |component: f32| -> u32 { (component.clamp(0.0, 1.0) * 255.0).round() as u32 };
    (channel(rgba.r) << 16) | (channel(rgba.g) << 8) | channel(rgba.b)
}

fn mix_hex(foreground: u32, background: u32, background_ratio: f32) -> u32 {
    let background_ratio = background_ratio.clamp(0.0, 1.0);
    let foreground_ratio = 1.0 - background_ratio;
    let channel = |shift: u32| -> u32 {
        let foreground_channel = ((foreground >> shift) & 0xFF) as f32;
        let background_channel = ((background >> shift) & 0xFF) as f32;
        (foreground_channel * foreground_ratio + background_channel * background_ratio).round()
            as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn mix_towards(color: Hsla, target: Hsla, amount: f32) -> Hsla {
    raw_color(mix_hex(rgba_to_u32(color), rgba_to_u32(target), amount))
}

pub fn divider_for_surface(surface: Hsla) -> Hsla {
    let rgb = surface.to_rgb();
    let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
    if luminance > 0.5 {
        raw_color(0x000000).opacity(0.10)
    } else {
        raw_color(0xFFFFFF).opacity(0.12)
    }
}

/// Default frosted-glass opacity for the UI chrome (used when no setting is
/// stored yet). The terminal body derives its own, more-opaque value from this.
pub const DEFAULT_VIBRANCY_ALPHA: f32 = 0.80;
/// Content panels (task column, right sidebar) sit this much more opaque than
/// the chrome so their lists/cards read clearly.
pub const PANEL_ALPHA_BOOST: f32 = 0.10;
/// The terminal body is kept this much more opaque than the UI chrome so the
/// shell text stays legible while the surrounding chrome reads as frosted glass.
pub const TERMINAL_ALPHA_BOOST: f32 = 0.20;

// 0 = solid (opaque), 1 = transparent (frosted). Alpha stored as f32 bits.
static DYNAMIC_WINDOW_TRANSPARENT: AtomicU32 = AtomicU32::new(1);
static DYNAMIC_VIBRANCY_ALPHA: AtomicU32 = AtomicU32::new(0); // set in `init` below

fn load_alpha(cell: &AtomicU32, fallback: f32) -> f32 {
    let bits = cell.load(Ordering::Relaxed);
    if bits == 0 {
        fallback
    } else {
        f32::from_bits(bits).clamp(0.0, 1.0)
    }
}

/// Whether the window paints solid (opaque) instead of frosted/transparent.
pub fn window_is_solid() -> bool {
    DYNAMIC_WINDOW_TRANSPARENT.load(Ordering::Relaxed) == 0
}

pub fn vibrancy_alpha() -> f32 {
    load_alpha(&DYNAMIC_VIBRANCY_ALPHA, DEFAULT_VIBRANCY_ALPHA)
}

/// The terminal body opacity, derived from the single UI opacity setting plus a
/// fixed boost so the terminal is always a bit more solid than the chrome.
pub fn terminal_alpha() -> f32 {
    (vibrancy_alpha() + TERMINAL_ALPHA_BOOST).min(1.0)
}

/// Apply the persisted appearance to the dynamic state read at render time.
pub fn set_window_appearance(transparent: bool, vibrancy_alpha: f32) {
    DYNAMIC_WINDOW_TRANSPARENT.store(transparent as u32, Ordering::Relaxed);
    set_vibrancy_alpha(vibrancy_alpha);
}

pub fn set_vibrancy_alpha(alpha: f32) {
    DYNAMIC_VIBRANCY_ALPHA.store(alpha.clamp(0.02, 1.0).to_bits(), Ordering::Relaxed);
}

/// Parse a stored opacity percentage string (e.g. "45") into a 0..1 fraction,
/// clamped to a sane range so the UI never becomes fully invisible.
pub fn opacity_fraction(percent: &str, default_percent: i64) -> f32 {
    let pct = percent
        .trim()
        .parse::<f64>()
        .map(|value| value.round() as i64)
        .unwrap_or(default_percent)
        .clamp(20, 100);
    pct as f32 / 100.0
}

/// Tint for frosted-glass regions (sidebar + column headers): a translucent fill
/// over the window's native blur material (macOS `NSVisualEffectView` / Windows
/// acrylic). In solid mode it returns the opaque base so callers degrade cleanly.
pub fn vibrancy(base: Hsla) -> Hsla {
    if window_is_solid() {
        base
    } else {
        base.opacity(vibrancy_alpha())
    }
}

/// Tint for content panels (task column, right sidebar): a step more opaque than
/// the chrome so their content reads clearly while still showing the blur.
pub fn vibrancy_panel(base: Hsla) -> Hsla {
    if window_is_solid() {
        base
    } else {
        base.opacity((vibrancy_alpha() + PANEL_ALPHA_BOOST).min(1.0))
    }
}

/// Whether the resolved app surface is dark — decides whether elevated surfaces
/// brighten (dark UI) or deepen (light UI). Reads the live app background.
fn surface_is_dark() -> bool {
    let bg = DYNAMIC_BG.load(Ordering::Relaxed);
    let r = ((bg >> 16) & 0xFF) as f32 / 255.0;
    let g = ((bg >> 8) & 0xFF) as f32 / 255.0;
    let b = (bg & 0xFF) as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
}

/// A solid surface one elevation step above (dark UI) / below (light UI) the
/// given base. Uses a *blended* tone — not a translucent overlay — so stacked
/// surfaces read as real depth instead of washing the frost out. `amount` is the
/// blend strength toward white / black.
pub fn elevate(base: Hsla, amount: f32) -> Hsla {
    let target = if surface_is_dark() {
        raw_color(0xFFFFFF)
    } else {
        raw_color(0x000000)
    };
    mix_towards(base, target, amount)
}

/// Raised chrome on a frosted panel — title / section bars AND cards. Blends the
/// base a step lighter (dark) / darker (light) so it reads as a genuine
/// elevation over the panel behind it, then inherits the panel opacity so it
/// still frosts. Opaque in solid mode.
pub fn vibrancy_raised(base: Hsla) -> Hsla {
    let raised = elevate(base, if surface_is_dark() { 0.07 } else { 0.06 });
    if window_is_solid() {
        raised
    } else {
        // Cards keep a high opacity floor so they stay legible even when the
        // user dials the window frost far down.
        raised.opacity((vibrancy_alpha() + PANEL_ALPHA_BOOST).clamp(0.85, 1.0))
    }
}

/// Tint for the terminal/workspace body backing. Translucent in transparent mode
/// so the blur shows behind the terminal; opaque in solid mode.
pub fn terminal_fill(base: Hsla) -> Hsla {
    if window_is_solid() {
        base
    } else {
        base.opacity(terminal_alpha())
    }
}

pub fn codux_main_titlebar(title: impl Into<SharedString>) -> TitlebarOptions {
    codux_titlebar(title, CoduxTitlebarKind::Main)
}

pub fn codux_child_titlebar(title: impl Into<SharedString>) -> TitlebarOptions {
    codux_titlebar(title, CoduxTitlebarKind::Child)
}

fn codux_titlebar(title: impl Into<SharedString>, kind: CoduxTitlebarKind) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: true,
        traffic_light_position: codux_traffic_light_position(kind),
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum CoduxTitlebarKind {
    Main,
    Child,
}

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy)]
enum CoduxTitlebarKind {
    Main,
    Child,
}

#[cfg(target_os = "macos")]
fn codux_traffic_light_position(kind: CoduxTitlebarKind) -> Option<gpui::Point<gpui::Pixels>> {
    match kind {
        // Vertically center the lights in the 52px main header.
        CoduxTitlebarKind::Main => Some(point(px(12.0), px(20.0))),
        CoduxTitlebarKind::Child => Some(point(px(12.0), px(17.0))),
    }
}

#[cfg(not(target_os = "macos"))]
fn codux_traffic_light_position(_kind: CoduxTitlebarKind) -> Option<gpui::Point<gpui::Pixels>> {
    None
}

pub fn apply_component_theme(
    theme_name: &str,
    theme_color: &str,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    let appearance = window
        .as_ref()
        .map(|window| window.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    apply_component_theme_for_appearance(theme_name, theme_color, appearance, window, cx);
}

pub fn apply_component_theme_for_appearance(
    theme_name: &str,
    theme_color: &str,
    appearance: WindowAppearance,
    mut window: Option<&mut Window>,
    cx: &mut App,
) {
    let terminal = terminal_theme_palette_for_appearance(theme_name, appearance);
    let app = app_theme_palette_for_appearance(theme_name, appearance);
    let mode = if terminal.is_light {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, window.as_deref_mut(), cx);

    configure_component_theme(cx, terminal, app, theme_color_value(theme_color));
    cx.refresh_windows();
    if let Some(window) = window {
        window.refresh();
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalThemePalette {
    pub background: u32,
    pub foreground: u32,
    pub cursor: u32,
    pub selection: u32,
    pub black: u32,
    pub red: u32,
    pub green: u32,
    pub yellow: u32,
    pub blue: u32,
    pub magenta: u32,
    pub cyan: u32,
    pub white: u32,
    pub bright_black: u32,
    pub bright_red: u32,
    pub bright_green: u32,
    pub bright_yellow: u32,
    pub bright_blue: u32,
    pub bright_magenta: u32,
    pub bright_cyan: u32,
    pub bright_white: u32,
    pub muted_foreground: u32,
    pub is_light: bool,
}

/// Designed app-chrome tokens for a Codux collection theme. Every surface,
/// line and text tone is hand-picked instead of derived from the terminal
/// palette, so panels/borders/hierarchy read as intentional design.
#[derive(Clone, Copy, Debug)]
pub struct AppThemePalette {
    pub ground: u32,
    pub column: u32,
    pub panel: u32,
    pub header: u32,
    pub card: u32,
    pub elevated: u32,
    pub border: u32,
    pub border_soft: u32,
    pub muted: u32,
    pub text: u32,
    pub text_muted: u32,
    pub text_dim: u32,
    pub status_bar: u32,
    pub row_hover: u32,
}

impl TerminalThemePalette {
    fn auto(is_light: bool) -> Self {
        if is_light {
            return terminal_theme_palette("Codux Light");
        }
        terminal_theme_palette("Codux Dark")
    }

    fn from_colors(
        is_light: bool,
        background: u32,
        foreground: u32,
        cursor: u32,
        selection: u32,
        ansi: [u32; 8],
        bright_ansi: [u32; 8],
    ) -> Self {
        let [black, red, green, yellow, blue, magenta, cyan, white] = ansi;
        let [
            bright_black,
            bright_red,
            bright_green,
            bright_yellow,
            bright_blue,
            bright_magenta,
            bright_cyan,
            bright_white,
        ] = bright_ansi;
        Self {
            background,
            foreground,
            cursor,
            selection,
            black,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
            white,
            bright_black,
            bright_red,
            bright_green,
            bright_yellow,
            bright_blue,
            bright_magenta,
            bright_cyan,
            bright_white,
            // ANSI bright black doubles as the readable comment / subdued tone.
            muted_foreground: bright_black,
            is_light,
        }
    }
}

pub fn terminal_theme_palette_for_appearance(
    theme_name: &str,
    appearance: WindowAppearance,
) -> TerminalThemePalette {
    if normalize_theme_name(theme_name) == "auto" {
        return TerminalThemePalette::auto(matches!(
            appearance,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ));
    }
    terminal_theme_palette(theme_name)
}

pub fn terminal_theme_palette(theme_name: &str) -> TerminalThemePalette {
    match normalize_theme_name(theme_name).as_str() {
        "auto" => TerminalThemePalette::auto(false),
        "codux dark" => TerminalThemePalette::from_colors(
            false,
            0x141821,
            0xD8DCE6,
            0x7AB8FF,
            0x21344F,
            [
                0x1E232E, 0xF27979, 0x82D896, 0xE8C66A, 0x74AEF6, 0xC793E8, 0x6CD3E0, 0xD8DCE6,
            ],
            [
                0x77839B, 0xFF9E9E, 0xA9EDB8, 0xF4D98C, 0x9FC9FF, 0xDDB3F5, 0x9EEAF2, 0xF5F7FA,
            ],
        ),
        "carbon" => TerminalThemePalette::from_colors(
            false,
            0x131313,
            0xE6E6E6,
            0xFFFFFF,
            0x282828,
            [
                0x222222, 0xF07070, 0x7BD88F, 0xE6C568, 0x6EB2F0, 0xC490E4, 0x66D4DE, 0xE6E6E6,
            ],
            [
                0x808080, 0xFF9B9B, 0xA6EDB4, 0xF4DA90, 0x9CCCFF, 0xDCB4F5, 0x96EAF0, 0xFFFFFF,
            ],
        ),
        "slate" => TerminalThemePalette::from_colors(
            false,
            0x252B33,
            0xE0E6ED,
            0x9CC6EE,
            0x303C4A,
            [
                0x1D232A, 0xEF8A92, 0xA3CD92, 0xE2C27F, 0x8FB5E7, 0xC3A1DC, 0x80C8CD, 0xCCD5DF,
            ],
            [
                0x97A4B2, 0xFFA4AC, 0xBDE3AC, 0xF2DA9D, 0xAFD0FA, 0xDABEEB, 0xA3E0E2, 0xF4F7FA,
            ],
        ),
        "deep ocean" => TerminalThemePalette::from_colors(
            false,
            0x0C1626,
            0xCCDCF2,
            0x53C0F0,
            0x152B49,
            [
                0x16233A, 0xF07886, 0x5FD4A2, 0xE8C170, 0x5AA7F0, 0xB48EF2, 0x54CBE0, 0xCCDCF2,
            ],
            [
                0x6783AA, 0xFF9DA8, 0x8CEBC2, 0xF5D694, 0x8CC5FF, 0xCFB0FF, 0x86E5F2, 0xF0F6FF,
            ],
        ),
        "forest night" => TerminalThemePalette::from_colors(
            false,
            0x101915,
            0xD3E2D6,
            0x7FD8A0,
            0x1D3729,
            [
                0x1B2921, 0xEF7B7B, 0x7CD98F, 0xD9C36A, 0x6FB3E8, 0xC08FE0, 0x63D6C2, 0xD3E2D6,
            ],
            [
                0x6A8977, 0xFFA2A2, 0xA5EDB4, 0xEDD98F, 0x9BCDF5, 0xD8B2F0, 0x92EBDC, 0xF0F7F1,
            ],
        ),
        "ember" => TerminalThemePalette::from_colors(
            false,
            0x1A1512,
            0xE8DDD0,
            0xF0A860,
            0x3A2D20,
            [
                0x2A221B, 0xF2766B, 0x9CCE6E, 0xF0B860, 0x6FAEE0, 0xD892C0, 0x72CCB8, 0xE8DDD0,
            ],
            [
                0x957E67, 0xFF9C8F, 0xBEE393, 0xFFD088, 0x9CC9F0, 0xEDB4D8, 0x9CE3D2, 0xFAF4EC,
            ],
        ),
        "codux light" => TerminalThemePalette::from_colors(
            true,
            0xFAFBFC,
            0x2A3140,
            0x2F6FE4,
            0xCCE0FA,
            [
                0x323A48, 0xB1303B, 0x156E38, 0x7E5A0B, 0x1F5CC0, 0x8939B8, 0x0B697D, 0x99A2B2,
            ],
            [
                0x576072, 0xBB1E2A, 0x1A6E3D, 0x7B5B0F, 0x165BC6, 0x8C2FC7, 0x10697B, 0xB8C0CC,
            ],
        ),
        "paper ink" => TerminalThemePalette::from_colors(
            true,
            0xEFF1F4,
            0x253248,
            0x315D9A,
            0xD4E0F0,
            [
                0x253248, 0xAC354B, 0x266D45, 0x7B5C00, 0x285CAC, 0x7D4D95, 0x166A79, 0x59677B,
            ],
            [
                0x5D687A, 0xA9384C, 0x276D46, 0x7A5C06, 0x2D60AC, 0x7F4C95, 0x1D6A77, 0x46546A,
            ],
        ),
        "ivory" => TerminalThemePalette::from_colors(
            true,
            0xFCF9F2,
            0x3A3226,
            0xBF8218,
            0xF0E0BC,
            [
                0x423A2C, 0xAC3C32, 0x3E6E20, 0x805C0F, 0x2F62AA, 0x8D459A, 0x176E62, 0xA89B84,
            ],
            [
                0x6C614D, 0xB63125, 0x436D26, 0x7D5D13, 0x3062A8, 0x9140A0, 0x1C6E63, 0xCCC0A8,
            ],
        ),
        "matcha" => TerminalThemePalette::from_colors(
            true,
            0xF8FBF6,
            0x2C3A2A,
            0x3E9450,
            0xCFE8CC,
            [
                0x33422F, 0xA74039, 0x257031, 0x756014, 0x2A63A6, 0x7F4BA3, 0x136E67, 0x97A692,
            ],
            [
                0x566651, 0xB3342B, 0x2A6F35, 0x736117, 0x2B63A7, 0x8641B7, 0x176D67, 0xBCC9B7,
            ],
        ),
        "rose dawn" => TerminalThemePalette::from_colors(
            true,
            0xFBF5F2,
            0x473747,
            0x944C67,
            0xE8D7DE,
            [
                0x473747, 0xA23A52, 0x366848, 0x79591C, 0x40608C, 0x7D4D84, 0x23686A, 0x6F596A,
            ],
            [
                0x756272, 0xA13B52, 0x396848, 0x785920, 0x405F8C, 0x7B4E83, 0x266869, 0x604B5D,
            ],
        ),
        _ => TerminalThemePalette::auto(false),
    }
}

pub fn app_theme_palette_for_appearance(
    theme_name: &str,
    appearance: WindowAppearance,
) -> AppThemePalette {
    let name = normalize_theme_name(theme_name);
    let name = if name == "auto" {
        let is_light = matches!(
            appearance,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        if is_light {
            "codux light"
        } else {
            "codux dark"
        }
        .to_string()
    } else {
        name
    };
    // Shared normalization resolves retired names and the Auto fallback.
    app_theme_palette(&name)
        .or_else(|| app_theme_palette("codux dark"))
        .expect("flagship app palette")
}

/// Designed chrome palettes for the Codux theme collection.
pub fn app_theme_palette(theme_name: &str) -> Option<AppThemePalette> {
    let palette = match normalize_theme_name(theme_name).as_str() {
        "codux dark" => AppThemePalette {
            ground: 0x12151C,
            column: 0x151922,
            panel: 0x191E28,
            header: 0x1B2029,
            card: 0x2A3140,
            elevated: 0x303848,
            border: 0x323A4A,
            border_soft: 0x2A3140,
            muted: 0x1F2530,
            text: 0xE8EBF2,
            text_muted: 0xA3ACBE,
            text_dim: 0x9DA3B1,
            status_bar: 0x161A22,
            row_hover: 0x232A38,
        },
        "carbon" => AppThemePalette {
            ground: 0x111111,
            column: 0x141414,
            panel: 0x191919,
            header: 0x1C1C1C,
            card: 0x2B2B2B,
            elevated: 0x323232,
            border: 0x393939,
            border_soft: 0x2E2E2E,
            muted: 0x202020,
            text: 0xF2F2F2,
            text_muted: 0xABABAB,
            text_dim: 0x9C9C9C,
            status_bar: 0x151515,
            row_hover: 0x252525,
        },
        "slate" => AppThemePalette {
            ground: 0x1E242B,
            column: 0x232A32,
            panel: 0x2B333D,
            header: 0x2E3742,
            card: 0x35404D,
            elevated: 0x3D4957,
            border: 0x4C5C6D,
            border_soft: 0x3E4A59,
            muted: 0x303945,
            text: 0xE4EAF0,
            text_muted: 0xC0CBD6,
            text_dim: 0xB2C0CE,
            status_bar: 0x232A32,
            row_hover: 0x354150,
        },
        "deep ocean" => AppThemePalette {
            ground: 0x0A1220,
            column: 0x0C1526,
            panel: 0x0F1A2E,
            header: 0x101C32,
            card: 0x1D3050,
            elevated: 0x24395E,
            border: 0x264066,
            border_soft: 0x1E3352,
            muted: 0x142138,
            text: 0xD9E4F5,
            text_muted: 0x91A6C7,
            text_dim: 0x96A6BE,
            status_bar: 0x0C1523,
            row_hover: 0x182A45,
        },
        "forest night" => AppThemePalette {
            ground: 0x0E1512,
            column: 0x111915,
            panel: 0x152019,
            header: 0x17231C,
            card: 0x24382C,
            elevated: 0x2A4033,
            border: 0x2F4638,
            border_soft: 0x27392E,
            muted: 0x1A2820,
            text: 0xE2EBE4,
            text_muted: 0x9DB3A4,
            text_dim: 0x9AACA0,
            status_bar: 0x101813,
            row_hover: 0x1E2E24,
        },
        "ember" => AppThemePalette {
            ground: 0x171310,
            column: 0x1B1613,
            panel: 0x211B16,
            header: 0x241D18,
            card: 0x352C23,
            elevated: 0x3C3228,
            border: 0x453930,
            border_soft: 0x392F27,
            muted: 0x282019,
            text: 0xF0E8E0,
            text_muted: 0xBCA997,
            text_dim: 0xA99C8E,
            status_bar: 0x1A1512,
            row_hover: 0x2E251D,
        },
        "codux light" => AppThemePalette {
            ground: 0xEEF0F4,
            column: 0xF2F4F7,
            panel: 0xF8F9FB,
            header: 0xF4F6F9,
            card: 0xFFFFFF,
            elevated: 0xFFFFFF,
            border: 0xD5DAE3,
            border_soft: 0xE1E5EC,
            muted: 0xE7EAF0,
            text: 0x1D2430,
            text_muted: 0x4E5A6E,
            text_dim: 0x5F6878,
            status_bar: 0xF2F4F7,
            row_hover: 0xE6EAF1,
        },
        "paper ink" => AppThemePalette {
            ground: 0xDFE4EB,
            column: 0xE5E9F0,
            panel: 0xEFF1F4,
            header: 0xE9EDF2,
            card: 0xFAFBFD,
            elevated: 0xFFFFFF,
            border: 0xC6CFDC,
            border_soft: 0xD4DCE6,
            muted: 0xE2E7EF,
            text: 0x253248,
            text_muted: 0x4C5B70,
            text_dim: 0x566276,
            status_bar: 0xE5E9F0,
            row_hover: 0xDCE3ED,
        },
        "ivory" => AppThemePalette {
            ground: 0xF2EDE3,
            column: 0xF5F0E7,
            panel: 0xFAF6EE,
            header: 0xF6F2E9,
            card: 0xFFFDF8,
            elevated: 0xFFFEFA,
            border: 0xDED4C2,
            border_soft: 0xE8E0D1,
            muted: 0xEDE6D8,
            text: 0x322A1E,
            text_muted: 0x655A47,
            text_dim: 0x6B6252,
            status_bar: 0xF5F0E7,
            row_hover: 0xEAE2D2,
        },
        "matcha" => AppThemePalette {
            ground: 0xEAF0E8,
            column: 0xEEF3EC,
            panel: 0xF6F9F4,
            header: 0xF1F5EF,
            card: 0xFDFEFC,
            elevated: 0xFFFFFF,
            border: 0xCFDCCB,
            border_soft: 0xDDE6D9,
            muted: 0xE3EBE0,
            text: 0x1F2B1E,
            text_muted: 0x4D5F4A,
            text_dim: 0x5B6958,
            status_bar: 0xEEF3EC,
            row_hover: 0xE0E9DC,
        },
        "rose dawn" => AppThemePalette {
            ground: 0xF0E7E4,
            column: 0xF5ECE8,
            panel: 0xFBF5F2,
            header: 0xF7EEEA,
            card: 0xFFFAF7,
            elevated: 0xFFFDFC,
            border: 0xD9C7CF,
            border_soft: 0xE9DBDF,
            muted: 0xEDE0E4,
            text: 0x473747,
            text_muted: 0x675364,
            text_dim: 0x6F5C6A,
            status_bar: 0xF5ECE8,
            row_hover: 0xEADDE3,
        },
        _ => return None,
    };
    Some(palette)
}

fn normalize_theme_name(value: &str) -> String {
    codux_runtime::settings::canonical_terminal_theme(value).to_ascii_lowercase()
}

pub fn theme_color_value(theme_color: &str) -> u32 {
    match theme_color.to_ascii_lowercase().as_str() {
        "sky" => 0x0EA5E9,
        "cyan" => 0x06B6D4,
        "teal" => 0x14B8A6,
        "emerald" | "moss" => 0x10B981,
        "green" | "sage" => 0x22C55E,
        "lime" => 0x84CC16,
        "amber" | "gold" => 0xF59E0B,
        "orange" | "burnt" => 0xF97316,
        "red" | "crimson" => 0xEF4444,
        "rose" | "plum" => 0xF43F5E,
        "pink" => 0xEC4899,
        "fuchsia" => 0xD946EF,
        "purple" => 0xA855F7,
        "violet" | "iris" | "lavender" => 0x8B5CF6,
        "indigo" => 0x6366F1,
        _ => 0x3B82F6,
    }
}

fn configure_component_theme(
    cx: &mut App,
    terminal: TerminalThemePalette,
    app: AppThemePalette,
    accent_hex: u32,
) {
    let is_dark = !terminal.is_light;
    let theme = Theme::global_mut(cx);
    theme.font_size = px(13.0);
    theme.radius = px(6.0);
    theme.radius_lg = px(10.0);
    let accent_color = raw_color(accent_hex);

    // Mode-scoped values, independent of the surface palette.
    let (control_bg, control_hover, input) = if is_dark {
        (
            raw_color(0xFFFFFF).opacity(0.09),
            raw_color(0xFFFFFF).opacity(0.14),
            raw_color(app.border),
        )
    } else {
        (
            raw_color(0x000000).opacity(0.055),
            raw_color(0x000000).opacity(0.085),
            raw_color(app.border),
        )
    };
    let accent = accent_color;
    let accent_bg = accent_color.opacity(if is_dark { 0.17 } else { 0.12 });
    let (primary_hover, primary_active, primary_foreground) = if is_dark {
        (
            mix_towards(accent_color, raw_color(0xFFFFFF), 0.18),
            mix_towards(accent_color, raw_color(0x000000), 0.16),
            raw_color(0xF6FAFF),
        )
    } else {
        (
            mix_towards(accent_color, raw_color(0x000000), 0.12),
            mix_towards(accent_color, raw_color(0x000000), 0.22),
            raw_color(0xFFFFFF),
        )
    };
    let overlay = raw_color(0x000000).opacity(if is_dark { 0.42 } else { 0.28 });
    let (danger, warning, success, info) = if is_dark {
        (
            raw_color(RED),
            raw_color(ORANGE),
            raw_color(GREEN),
            raw_color(0x60A5FA),
        )
    } else {
        (
            raw_color(0xDC2626),
            raw_color(0xD97706),
            raw_color(0x16A34A),
            raw_color(0x2563EB),
        )
    };
    let hover_surface = if is_dark {
        raw_color(0xFFFFFF).opacity(0.075)
    } else {
        raw_color(0x000000).opacity(0.035)
    };

    // Surfaces: every tone comes from the designed app palette.
    let background = raw_color(app.ground);
    let foreground = raw_color(app.text);
    let muted = raw_color(app.muted);
    let muted_foreground = raw_color(app.text_dim);
    let border = raw_color(app.border);
    let border_soft = raw_color(app.border_soft);
    let popover = raw_color(app.elevated);
    let sidebar = raw_color(app.panel);
    let header = raw_color(app.header);
    let row_hover = raw_color(app.row_hover);
    let title_bar = raw_color(app.status_bar);
    let tab = raw_color(app.ground);
    let tab_bar = raw_color(app.header);
    let scrollbar_thumb = mix_towards(border, muted_foreground, 0.45);
    let task_column = raw_color(app.column);
    let panel_surface = raw_color(app.panel);
    let card_surface = raw_color(app.card);
    let text = raw_color(app.text);
    let text_muted = raw_color(app.text_muted);

    set_dynamic_color(&DYNAMIC_BG, rgba_to_u32(background));
    set_dynamic_color(&DYNAMIC_BG_ELEVATED, rgba_to_u32(popover));
    set_dynamic_color(&DYNAMIC_BG_PANEL, rgba_to_u32(panel_surface));
    set_dynamic_color(&DYNAMIC_BG_TERMINAL, terminal.background);
    set_dynamic_color(&DYNAMIC_BG_COLUMN, rgba_to_u32(task_column));
    set_dynamic_color(&DYNAMIC_BG_HEADER, rgba_to_u32(header));
    set_dynamic_color(&DYNAMIC_BG_ROW_HOVER, rgba_to_u32(row_hover));
    set_dynamic_color(&DYNAMIC_BG_ROW_ACTIVE, rgba_to_u32(accent_bg));
    set_dynamic_color(&DYNAMIC_BORDER, rgba_to_u32(border));
    set_dynamic_color(&DYNAMIC_BORDER_SOFT, rgba_to_u32(border_soft));
    set_dynamic_color(&DYNAMIC_TEXT, rgba_to_u32(text));
    set_dynamic_color(&DYNAMIC_TEXT_MUTED, rgba_to_u32(text_muted));
    set_dynamic_color(&DYNAMIC_TEXT_DIM, rgba_to_u32(muted_foreground));
    set_dynamic_color(&DYNAMIC_ACCENT, accent_hex);
    // Keep the semantic triad mode-aware so `color(GREEN/ORANGE/RED)` callers
    // (incl. cx-less helpers) follow light/dark automatically.
    set_dynamic_color(&DYNAMIC_ORANGE, rgba_to_u32(warning));
    set_dynamic_color(&DYNAMIC_GREEN, rgba_to_u32(success));
    set_dynamic_color(&DYNAMIC_RED, rgba_to_u32(danger));
    set_dynamic_color(&DYNAMIC_STATUS_BAR, rgba_to_u32(title_bar));

    theme.shadow = false;
    theme.radius = gpui::px(6.0);
    theme.radius_lg = gpui::px(8.0);
    theme.background = background;
    theme.foreground = foreground;
    theme.muted = muted;
    theme.muted_foreground = muted_foreground;
    theme.border = border;
    theme.primary = accent;
    theme.primary_hover = primary_hover;
    theme.primary_active = primary_active;
    theme.primary_foreground = primary_foreground;
    theme.button_primary = theme.primary;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.button_primary_foreground = theme.primary_foreground;
    // One raised-card tone for every rest-state control surface — sidebar and
    // settings cards, secondary buttons/pills, and the segmented tab track.
    theme.secondary = card_surface;
    theme.secondary_hover = hover_surface;
    theme.secondary_foreground = foreground;
    theme.secondary_active = accent_bg;
    theme.group_box = card_surface;
    theme.group_box_foreground = foreground;
    theme.accent = accent_bg;
    theme.accent_foreground = foreground;
    theme.input = input;
    theme.caret = accent;
    theme.ring = accent;
    theme.selection = accent.opacity(if is_dark { 0.28 } else { 0.20 });
    let highlight_style = std::sync::Arc::make_mut(&mut theme.highlight_theme)
        .style
        .clone();
    let mut highlight_style = highlight_style;
    highlight_style.editor_background = Some(background);
    highlight_style.editor_active_line = Some(if is_dark {
        raw_color(0x000000).opacity(0.20)
    } else {
        raw_color(0x000000).opacity(0.055)
    });
    highlight_style.editor_line_number = Some(if is_dark {
        raw_color(0xFFFFFF).opacity(0.32)
    } else {
        raw_color(0x000000).opacity(0.34)
    });
    highlight_style.editor_active_line_number = Some(foreground);
    std::sync::Arc::make_mut(&mut theme.highlight_theme).style = highlight_style;
    theme.danger = danger;
    theme.danger_hover = danger.mix(theme.transparent, 0.22);
    theme.danger_active = danger.mix(theme.transparent, 0.34);
    theme.danger_foreground = primary_foreground;
    theme.warning = warning;
    theme.warning_hover = warning.mix(theme.transparent, 0.22);
    theme.warning_active = warning.mix(theme.transparent, 0.34);
    theme.warning_foreground = primary_foreground;
    theme.success = success;
    theme.success_hover = success.mix(theme.transparent, 0.22);
    theme.success_active = success.mix(theme.transparent, 0.34);
    theme.success_foreground = primary_foreground;
    theme.info = info;
    theme.info_hover = info.mix(theme.transparent, 0.22);
    theme.info_active = info.mix(theme.transparent, 0.34);
    theme.info_foreground = primary_foreground;
    theme.link = if is_dark {
        mix_towards(accent_color, color(0xFFFFFF), 0.34)
    } else {
        accent_color
    };
    theme.link_hover = if is_dark {
        mix_towards(accent_color, color(0xFFFFFF), 0.50)
    } else {
        mix_towards(accent_color, color(0x000000), 0.16)
    };
    theme.link_active = if is_dark {
        accent_color
    } else {
        mix_towards(accent_color, color(0x000000), 0.28)
    };
    theme.popover = popover;
    theme.popover_foreground = foreground;
    theme.drop_target = accent.opacity(0.16);
    theme.drag_border = accent.opacity(0.50);
    theme.tiles = muted;
    theme.title_bar = title_bar;
    theme.title_bar_border = border;
    theme.tab = tab;
    theme.tab_active = accent;
    theme.tab_active_foreground = primary_foreground;
    theme.tab_bar = tab_bar;
    theme.tab_bar_segmented = muted;
    theme.tab_foreground = muted_foreground;
    theme.colors.list = background;
    theme.list_hover = hover_surface;
    theme.list_active = accent_bg;
    theme.list_active_border = accent.opacity(if is_dark { 0.46 } else { 0.36 });
    // Flat selection: filled row without the border ring (list + table rows).
    theme.list.active_highlight = false;
    theme.list_head = header;
    theme.list_even = background;
    theme.table = background;
    theme.table_hover = hover_surface;
    theme.table_active = accent_bg;
    theme.table_active_border = accent.opacity(if is_dark { 0.46 } else { 0.36 });
    theme.table_even = background;
    theme.table_head = header;
    theme.table_head_foreground = muted_foreground;
    theme.table_foot = header;
    theme.table_foot_foreground = muted_foreground;
    theme.table_row_border = border;
    theme.switch = control_hover;
    theme.switch_thumb = background;
    theme.scrollbar = sidebar.opacity(0.0);
    theme.scrollbar_thumb = scrollbar_thumb;
    theme.scrollbar_thumb_hover = muted_foreground;
    theme.sidebar = sidebar;
    theme.sidebar_foreground = foreground;
    theme.sidebar_border = border;
    theme.sidebar_accent = accent_bg;
    theme.sidebar_accent_foreground = foreground;
    theme.sidebar_primary = accent;
    theme.sidebar_primary_foreground = primary_foreground;
    theme.skeleton = control_bg;
    theme.accordion = muted;
    theme.accordion_hover = hover_surface;
    theme.overlay = overlay;
    theme.window_border = border;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: u32, b: u32) -> f64 {
        let luminance = |hex: u32| {
            let channel = |shift: u32| {
                let v = ((hex >> shift) & 255u32) as f64 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn curated_themes_have_readable_terminal_text() {
        for option in codux_runtime::settings::TERMINAL_THEMES
            .iter()
            .filter(|t| t.group != codux_runtime::settings::TerminalThemeGroup::System)
        {
            let name = option.name;
            let p = terminal_theme_palette(name);
            for (role, value) in [
                ("foreground", p.foreground),
                ("muted", p.muted_foreground),
                ("comment", p.bright_black),
                ("red", p.red),
                ("green", p.green),
                ("yellow", p.yellow),
                ("blue", p.blue),
                ("magenta", p.magenta),
                ("cyan", p.cyan),
                ("bright red", p.bright_red),
                ("bright green", p.bright_green),
                ("bright yellow", p.bright_yellow),
                ("bright blue", p.bright_blue),
                ("bright magenta", p.bright_magenta),
                ("bright cyan", p.bright_cyan),
            ] {
                assert!(
                    contrast(value, p.background) >= 4.5,
                    "{name} {role}: {:.2}",
                    contrast(value, p.background)
                );
            }
            assert!(
                contrast(p.foreground, p.selection) >= 4.5,
                "{name} selection"
            );
            for value in [
                p.red,
                p.green,
                p.yellow,
                p.blue,
                p.magenta,
                p.cyan,
                p.bright_red,
                p.bright_green,
                p.bright_yellow,
                p.bright_blue,
                p.bright_magenta,
                p.bright_cyan,
            ] {
                assert!(contrast(value, p.selection) >= 4.5, "{name} selected ANSI");
            }
            assert!(contrast(p.cursor, p.background) >= 3.0, "{name} cursor");
        }
    }

    #[test]
    fn codux_collection_has_app_and_terminal_palettes() {
        use codux_runtime::settings::{TERMINAL_THEMES, TerminalThemeGroup};
        let mut backgrounds = std::collections::HashSet::new();
        for option in TERMINAL_THEMES
            .iter()
            .filter(|t| t.group != TerminalThemeGroup::System)
        {
            let name = option.name;
            let terminal = terminal_theme_palette(name);
            assert_eq!(
                terminal.is_light,
                option.group == TerminalThemeGroup::Light,
                "{name}"
            );
            assert!(
                backgrounds.insert(terminal.background),
                "{name} must resolve to its own palette"
            );
            let app =
                app_theme_palette(name).unwrap_or_else(|| panic!("{name} missing app palette"));
            for background in [
                app.ground,
                app.column,
                app.panel,
                app.header,
                app.card,
                app.elevated,
                app.muted,
                app.status_bar,
                app.row_hover,
            ] {
                for foreground in [app.text, app.text_muted, app.text_dim] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{name} app text: {:.2}",
                        contrast(foreground, background)
                    );
                }
            }
        }
        assert_eq!(backgrounds.len(), 11);
    }

    #[test]
    fn legacy_palette_lookups_match_migrated_settings() {
        for (old, new) in [
            ("arctic-night", "Slate"),
            ("Amethyst Dusk", "Slate"),
            ("Rose Noir", "Ember"),
            ("Glacier", "Paper Ink"),
            ("Morning Mist", "Paper Ink"),
            ("Lavender", "Rose Dawn"),
            ("Rosewater", "Rose Dawn"),
            ("Sandstone", "Ivory"),
        ] {
            assert_eq!(
                terminal_theme_palette(old).background,
                terminal_theme_palette(new).background
            );
            assert_eq!(
                app_theme_palette(old).unwrap().ground,
                app_theme_palette(new).unwrap().ground
            );
        }
    }

    #[test]
    fn auto_theme_resolves_codux_flagships() {
        let dark = terminal_theme_palette_for_appearance("Auto", WindowAppearance::Dark);
        assert_eq!(
            dark.background,
            terminal_theme_palette("Codux Dark").background
        );
        let light = terminal_theme_palette_for_appearance("Auto", WindowAppearance::Light);
        assert!(light.is_light);
        // Legacy persisted names fall back to the flagship dark palette.
        let fallback = app_theme_palette_for_appearance("Dracula", WindowAppearance::Dark);
        assert_eq!(
            fallback.ground,
            app_theme_palette("codux dark").expect("flagship").ground
        );
    }
}
