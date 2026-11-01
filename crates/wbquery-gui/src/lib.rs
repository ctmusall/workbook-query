//! The Workbook Query desktop app (egui).

pub mod app;
pub mod chart_state;
pub mod conditions_view;
pub mod plot_view;
pub mod state;
pub mod table_view;
pub mod tree;
pub mod widgets;

pub use app::WbqApp;
pub use state::AppState;

/// Fonts: egui's defaults, with Hack (arrows, ▲ ▼ ≡) and Noto Emoji (⚠) as fallbacks for symbols the
/// UI font lacks.
pub fn setup_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "NotoEmoji".into(),
        Arc::new(
            egui::FontData::from_static(epaint_default_fonts::NOTO_EMOJI_REGULAR)
                .tweak(egui::FontTweak { scale: 0.81, ..Default::default() }),
        ),
    );
    let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    for name in ["Hack", "NotoEmoji"] {
        if !proportional.iter().any(|n| n == name) {
            proportional.push(name.into());
        }
    }
    ctx.set_fonts(fonts);
}

/// The window icon.
pub fn icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(
        include_bytes!("../../../assets/icon.png"),
        image::ImageFormat::Png,
    )
    .ok()?
    .into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}
