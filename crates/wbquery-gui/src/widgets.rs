//! Small widgets the window reuses: a combo box you can type in to search, a text box that suggests
//! values, and a highlight-color swatch.

use egui::{Color32, Id, Response, RichText, Ui};
use wbquery_core::pyfmt::casefold;

/// `#rrggbb` as a color.
pub fn color32(hex: &str) -> Option<Color32> {
    let h = hex.strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    (h.len() == 6).then(|| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// A combo box over `options` with a search box at the top of its list (matching anywhere, ignoring
/// case). `tooltip(option)` adds hover text to each choice. Returns true when the choice changed.
pub fn search_combo(
    ui: &mut Ui,
    id: Id,
    name: &str,
    value: &mut String,
    options: &[String],
    width: f32,
    tooltip: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let mut changed = false;
    let filter_id = id.with("filter");
    let shown = if value.is_empty() { "(choose)".to_string() } else { value.clone() };
    let inner =
        egui::ComboBox::from_id_salt(id).selected_text(shown).width(width).height(320.0).show_ui(ui, |ui| {
            let mut filter: String = ui.data_mut(|d| d.get_temp::<String>(filter_id).unwrap_or_default());
            let edit = ui.add(
                egui::TextEdit::singleline(&mut filter).hint_text("Search").desired_width(f32::INFINITY),
            );
            if !edit.has_focus() && !ui.memory(|m| m.focused().is_some()) {
                edit.request_focus();
            }
            let needle = casefold(filter.trim());
            let mut first: Option<&String> = None;
            for o in options.iter().filter(|o| needle.is_empty() || casefold(o).contains(&needle)) {
                first.get_or_insert(o);
                let mut r = ui.selectable_label(o == value, o.as_str());
                if let Some(t) = tooltip(o) {
                    r = r.on_hover_text(t);
                }
                if r.clicked() {
                    *value = o.clone();
                    changed = true;
                }
            }
            if edit.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(f) = first
            {
                *value = f.clone();
                changed = true;
                ui.close();
            }
            ui.data_mut(|d| d.insert_temp(filter_id, filter));
        });
    let (label, enabled) = (format!("{name}: {value}"), ui.is_enabled());
    inner.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, &label));
    if !inner.response.is_pointer_button_down_on() && inner.inner.is_none() {
        ui.data_mut(|d| d.remove::<String>(filter_id));
    }
    changed
}

/// A plain combo box over fixed choices. Returns true when the choice changed.
pub fn combo(ui: &mut Ui, id: Id, name: &str, value: &mut String, options: &[&str], width: f32) -> bool {
    let mut changed = false;
    let inner =
        egui::ComboBox::from_id_salt(id).selected_text(value.as_str()).width(width).show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(value == o, *o).clicked() && value != o {
                    *value = (*o).to_string();
                    changed = true;
                }
            }
        });
    let (label, enabled) = (format!("{name}: {value}"), ui.is_enabled());
    inner.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, &label));
    changed
}

/// What happened in a [`suggest_field`].
#[derive(Default)]
pub struct FieldEvents {
    pub changed: bool,
    /// Enter was pressed (without picking a suggestion).
    pub submitted: bool,
    pub response: Option<Response>,
}

/// A one-line text box; while it has focus, values from `suggestions` that contain what's typed show
/// in a list under it (click one to use it).
pub fn suggest_field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    suggestions: &[String],
    hint: &str,
    width: f32,
) -> FieldEvents {
    let mut ev = FieldEvents::default();
    let r = ui.add(egui::TextEdit::singleline(text).id(id).hint_text(hint).desired_width(width));
    ev.changed = r.changed();
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        ev.submitted = true;
    }
    let open = r.has_focus() || ui.data(|d| d.get_temp::<bool>(id.with("hovering")).unwrap_or(false));
    if open && !suggestions.is_empty() {
        let needle = casefold(text.trim());
        let matches: Vec<&String> = suggestions
            .iter()
            .filter(|s| needle.is_empty() || casefold(s).contains(&needle))
            .filter(|s| s.as_str() != text.as_str())
            .take(200)
            .collect();
        if !matches.is_empty() {
            let area = egui::Area::new(id.with("suggestions"))
                .order(egui::Order::Foreground)
                .fixed_pos(r.rect.left_bottom())
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(r.rect.width());
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for m in matches {
                                if ui.selectable_label(false, m.as_str()).clicked() {
                                    *text = m.clone();
                                    ev.changed = true;
                                }
                            }
                        });
                    });
                });
            let hovering = area.response.contains_pointer();
            ui.data_mut(|d| d.insert_temp(id.with("hovering"), hovering));
        }
    } else {
        ui.data_mut(|d| d.insert_temp(id.with("hovering"), false));
    }
    ev.response = Some(r);
    ev
}

/// A highlight-color swatch: click to choose a color, right-click for "No color". `mixed` shows that
/// the conditions under it differ. Returns the new color when it changed (`""` for none).
pub fn color_button(ui: &mut Ui, id: Id, color: &str, mixed: bool) -> Option<String> {
    let mut out = None;
    let fill = color32(color);
    let label = if mixed {
        RichText::new("…")
    } else if fill.is_some() {
        RichText::new("   ")
    } else {
        RichText::new("–")
    };
    let mut button = egui::Button::new(label).min_size(egui::vec2(26.0, 18.0));
    if let Some(c) = fill {
        button = button.fill(c);
    }
    let r = ui.add(button).on_hover_text(
        "Highlight color for the cells this matches. Click to choose; right-click for no color.",
    );
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Highlight color"));
    // The picker's open state is kept here: egui's toggle popup closed on the very click that opened it.
    let open_id = id.with("open");
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    if r.clicked() {
        open = !open;
    }
    if open {
        let area = egui::Area::new(id.with("picker"))
            .order(egui::Order::Foreground)
            .fixed_pos(r.rect.left_bottom() + egui::vec2(0.0, 2.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let mut c = fill.unwrap_or(Color32::from_rgb(0xff, 0xd5, 0x4f));
                    if egui::widgets::color_picker::color_picker_color32(
                        ui,
                        &mut c,
                        egui::widgets::color_picker::Alpha::Opaque,
                    ) {
                        out = Some(hex(c));
                    }
                    ui.horizontal(|ui| {
                        for preset in
                            ["#ffd54f", "#ff8a80", "#80d8ff", "#b9f6ca", "#ea80fc", "#ff0000", "#0000ff"]
                        {
                            let pc = color32(preset).expect("preset");
                            let b = ui.add(egui::Button::new("  ").fill(pc)).on_hover_text(preset);
                            let label = format!("Highlight {preset}");
                            b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                            if b.clicked() {
                                out = Some(preset.to_string());
                                open = false;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("No color").clicked() {
                            out = Some(String::new());
                            open = false;
                        }
                        if ui.button("Done").clicked() {
                            open = false;
                        }
                    });
                });
            });
        let clicked_elsewhere = ui.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer()
            && !r.contains_pointer();
        if clicked_elsewhere || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    r.context_menu(|ui| {
        if ui.button("No color").clicked() {
            out = Some(String::new());
            ui.close();
        }
    });
    out.filter(|c| c != color || mixed)
}
