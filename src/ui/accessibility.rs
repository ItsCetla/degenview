//! Screen-reader labels for hand-painted and icon-only widgets.
//!
//! egui's accesskit integration (enabled by `eframe`'s default features) already names text
//! buttons, checkboxes, sliders, collapsing headers and plain labels from their on-screen
//! text. This module fills the gaps: painter regions (the candle canvas) and buttons whose
//! visible glyph (`×`, `🔔`, `/`) is not a useful spoken name.

use eframe::egui::{Response, WidgetInfo, WidgetType};

/// Attach a spoken name to a widget response, defaulting to a button.
///
/// The label is registered against the response's `Id` and may be read in a later frame, so
/// pass an owned `String` or a `&'static str` rather than a temporary.
pub fn label(response: Response, text: impl Into<String>) -> Response {
    label_as(response, WidgetType::Button, text)
}

/// [`label`] with an explicit [`WidgetType`], for panels, images, links and other regions.
pub fn label_as(response: Response, typ: WidgetType, text: impl Into<String>) -> Response {
    let text = text.into();
    let enabled = response.enabled();
    response.widget_info(move || WidgetInfo::labeled(typ, enabled, text.clone()));
    response
}
