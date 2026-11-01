//! `wbquery`: the Workbook Query app. Files given on the command line (or dropped on the program)
//! open in it; any `-option` runs the command line instead.

// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use wbquery_gui::{AppState, WbqApp, state::APP};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !wbquery_cli::wants_gui(&args) {
        return wbquery_cli::main_with(std::env::args_os());
    }
    // The windowed build has no console: show anything that goes wrong in a message box.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Something went wrong: {info}");
        eprintln!("{msg}");
        rfd::MessageDialog::new()
            .set_title(APP)
            .set_description(msg)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("{APP} {}", wbquery_core::VERSION))
        .with_inner_size([1500.0, 900.0])
        .with_app_id("wbquery")
        .with_drag_and_drop(true);
    if let Some(icon) = wbquery_gui::icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    let result = eframe::run_native(
        APP,
        options,
        Box::new(move |cc| {
            wbquery_gui::setup_fonts(&cc.egui_ctx);
            let mut app = WbqApp::new(AppState::default());
            app.open_paths(&paths);
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title(APP)
                .set_description(format!("The window couldn't open: {e}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            ExitCode::FAILURE
        }
    }
}
