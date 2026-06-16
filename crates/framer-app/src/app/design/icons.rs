//! Bundled Lucide icon font + a typed icon vocabulary.
//!
//! The font (`assets/lucide.ttf`, MIT) is compiled in and registered under its
//! own font family so it never affects text rendering. Reference glyphs through
//! [`Icon`] rather than raw code points.

use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily, FontId, RichText};

/// Name of the registered icon font family.
const FAMILY: &str = "lucide";

/// The icon vocabulary used across the app. Each maps to a Lucide glyph.
///
/// This is a complete vocabulary consumed across reskin phases; not every
/// variant is referenced at every commit.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    // Project / file.
    New,
    Open,
    Save,
    Export,
    Folder,
    // Header / status.
    Saved,
    Help,
    Profile,
    ThemeLight,
    ThemeDark,
    Ready,
    Warning,
    Error,
    // Workspace / view.
    Design,
    Plan,
    Shell,
    Wall,
    View3d,
    // Build catalog.
    Door,
    Window,
    GarageDoor,
    // Dimension.
    Linear,
    Aligned,
    Angular,
    // Tools.
    Select,
    Move,
    Measure,
    Options,
    // Editing / canvas toolbar.
    Edit,
    Duplicate,
    Delete,
    // Browser / tree.
    Search,
    Filter,
    Menu,
    ChevronDown,
    ChevronRight,
    Plus,
    Minus,
    More,
    Pin,
    Eye,
    // Status bar.
    Snap,
    LayoutGrid,
    LayoutColumns,
    Fullscreen,
    PanelLeft,
    PanelRight,
}

impl Icon {
    /// The private-use code point for this icon in the Lucide font.
    pub(crate) fn glyph(self) -> char {
        match self {
            Icon::New => '\u{e0c9}',           // file-plus
            Icon::Open => '\u{e247}',          // folder-open
            Icon::Save => '\u{e14d}',          // save
            Icon::Export => '\u{e19e}',        // upload
            Icon::Folder => '\u{e0d7}',        // folder
            Icon::Saved => '\u{e226}',         // circle-check
            Icon::Help => '\u{e082}',          // circle-help
            Icon::Profile => '\u{e54b}',       // book-text
            Icon::ThemeLight => '\u{e178}',    // sun
            Icon::ThemeDark => '\u{e11e}',     // moon
            Icon::Ready => '\u{e226}',         // circle-check
            Icon::Warning => '\u{e193}',       // triangle-alert
            Icon::Error => '\u{e084}',         // circle-x
            Icon::Design => '\u{e4f1}',        // pencil-ruler
            Icon::Plan => '\u{e086}',          // clipboard-list
            Icon::Shell => '\u{e0f3}',         // hexagon
            Icon::Wall => '\u{e581}',          // brick-wall
            Icon::View3d => '\u{e061}',        // box
            Icon::Door => '\u{e3d6}',          // door-open
            Icon::Window => '\u{e426}',        // app-window
            Icon::GarageDoor => '\u{e3e6}',    // warehouse
            Icon::Linear => '\u{e14b}',        // ruler
            Icon::Aligned => '\u{e1c5}',       // move-diagonal-2
            Icon::Angular => '\u{e192}',       // triangle
            Icon::Select => '\u{e1c3}',        // mouse-pointer-2
            Icon::Move => '\u{e121}',          // move
            Icon::Measure => '\u{e662}',       // ruler-dimension-line
            Icon::Options => '\u{e245}',       // settings-2
            Icon::Edit => '\u{e1f9}',          // pencil
            Icon::Duplicate => '\u{e09e}',     // copy
            Icon::Delete => '\u{e18e}',        // trash-2
            Icon::Search => '\u{e151}',        // search
            Icon::Filter => '\u{e460}',        // list-filter
            Icon::Menu => '\u{e184}',          // align-justify
            Icon::ChevronDown => '\u{e06d}',   // chevron-down
            Icon::ChevronRight => '\u{e06f}',  // chevron-right
            Icon::Plus => '\u{e13d}',          // plus
            Icon::Minus => '\u{e11c}',         // minus
            Icon::More => '\u{e0b6}',          // ellipsis
            Icon::Pin => '\u{e259}',           // pin
            Icon::Eye => '\u{e0ba}',           // eye
            Icon::Snap => '\u{e2b5}',          // magnet
            Icon::LayoutGrid => '\u{e0ff}',    // layout-grid
            Icon::LayoutColumns => '\u{e098}', // columns-2
            Icon::Fullscreen => '\u{e112}',    // maximize
            Icon::PanelLeft => '\u{e12a}',     // panel-left
            Icon::PanelRight => '\u{e431}',    // panel-right
        }
    }
}

/// The `FontFamily` icons render under.
pub(crate) fn family() -> FontFamily {
    FontFamily::Name(FAMILY.into())
}

/// A [`FontId`] for rendering icon glyphs with a [`egui::Painter`].
pub(crate) fn icon_font(size: f32) -> FontId {
    FontId::new(size, family())
}

/// A [`RichText`] for an icon glyph at the given size.
pub(crate) fn icon_text(icon: Icon, size: f32) -> RichText {
    RichText::new(icon.glyph().to_string()).font(icon_font(size))
}

/// Register the bundled icon font into the context's font stack.
///
/// Adds the font under a dedicated family so default proportional/monospace text
/// is untouched; icons are rendered by explicitly selecting [`family`].
pub(crate) fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        FAMILY.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/lucide.ttf"
        ))),
    );
    fonts
        .families
        .entry(family())
        .or_default()
        .push(FAMILY.to_owned());
    ctx.set_fonts(fonts);
}
