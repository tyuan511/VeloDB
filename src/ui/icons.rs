use crate::adapter::DbDriver;
use gpui::{
    div, img, linear_color_stop, linear_gradient, px, rgb, svg, AnyElement, Hsla, Image,
    ImageFormat, IntoElement, ParentElement, Styled, Svg,
};
use std::sync::{Arc, OnceLock};

macro_rules! hgi_svg {
    ($body:expr) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">"#,
            $body,
            r#"</svg>"#
        )
        .as_bytes()
    };
}

/// Returns the static SVG bytes for a Hugeicons (Stroke Rounded) icon by name.
pub fn hgi_bytes(name: &str) -> &'static [u8] {
    match name {
        "search" => hgi_svg!(r#"<circle cx="11" cy="11" r="8"/><path d="M17.5 17.5L22 22"/>"#),
        "arrow-down" => hgi_svg!(r#"<path d="M6 9l6 6 6-6"/>"#),
        "arrow-left" => hgi_svg!(r#"<path d="M15 6l-6 6 6 6"/>"#),
        "arrow-right" => hgi_svg!(r#"<path d="M9 6l6 6-6 6"/>"#),
        "activity" => hgi_svg!(r#"<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>"#),
        "shield-check" => hgi_svg!(
            r#"<path d="M12 2.5L4.5 5.5v6c0 5.2 3.2 8.7 7.5 10 4.3-1.3 7.5-4.8 7.5-10v-6L12 2.5z"/><path d="M9 12l2 2 4-4"/>"#
        ),
        "alert" => hgi_svg!(
            r#"<path d="M12 9v4m0 3.5h.01M10.3 3.9L2.6 17.3A2 2 0 0 0 4.3 20h15.4a2 2 0 0 0 1.7-2.7L13.7 3.9a2 2 0 0 0-3.4 0z"/>"#
        ),
        "plug" => hgi_svg!(r#"<path d="M12 22v-5m-3-9V3m6 5V3M6 8h12v3a6 6 0 0 1-12 0V8z"/>"#),
        "sidebar-left" => hgi_svg!(r#"<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M9 3v18"/>"#),
        "sidebar-right" => hgi_svg!(r#"<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M15 3v18"/>"#),
        "moon" => hgi_svg!(r#"<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/>"#),
        "sun" => hgi_svg!(
            r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4l1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4m11.4-11.4l1.4-1.4"/>"#
        ),
        "settings" => hgi_svg!(
            r#"<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>"#
        ),
        "refresh" => hgi_svg!(r#"<path d="M21 12a9 9 0 1 1-2.6-6.4L21 8m0-5v5h-5"/>"#),
        "plus" => hgi_svg!(r#"<path d="M12 5v14M5 12h14"/>"#),
        "code" => hgi_svg!(r#"<path d="M16 18l6-6-6-6M8 6l-6 6 6 6"/>"#),
        "table" => hgi_svg!(r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 10h18M9 10v10"/>"#),
        "view" => hgi_svg!(r#"<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>"#),
        "func" => hgi_svg!(r#"<path d="M9 17c2 0 2.8-1 2.8-2.8V10c0-2 1-3.3 3.2-3m-6 4.2h5.7"/>"#),
        "tag" => hgi_svg!(r#"<path d="M12.6 2.6L21 11l-8.4 8.4a2 2 0 0 1-2.8 0L3 12.6V4a1 1 0 0 1 1-1h8.6z"/><circle cx="7.5" cy="7.5" r="1.2"/>"#),
        "puzzle" => hgi_svg!(
            r#"<path d="M19.4 14.6a2 2 0 1 0 0-4H18V7a2 2 0 0 0-2-2h-3V3.6a2 2 0 1 0-4 0V5H6a2 2 0 0 0-2 2v3h1.4a2 2 0 1 1 0 4H4v3a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-2.4h1.4z"/>"#
        ),
        "columns" => hgi_svg!(r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9 4v16M15 4v16"/>"#),
        "trash" => hgi_svg!(r#"<path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m2 0v14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6"/>"#),
        "edit" => hgi_svg!(r#"<path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/>"#),
        "upload" => hgi_svg!(r#"<path d="M12 16V4m0 0l-4 4m4-4l4 4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/>"#),
        "download" => hgi_svg!(r#"<path d="M12 4v12m0 0l-4-4m4 4l4-4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/>"#),
        "play" => hgi_svg!(r#"<path d="M7 4.8v14.4a1 1 0 0 0 1.5.85l11.2-7.2a1 1 0 0 0 0-1.7L8.5 3.95A1 1 0 0 0 7 4.8z"/>"#),
        "flash" => hgi_svg!(r#"<path d="M13 2L4 14h7l-1 8 9-12h-7l1-8z"/>"#),
        "magic" => hgi_svg!(
            r#"<path d="M15 4V2m0 14v-2M8 9h2m10 0h2m-4.2-3.8l1.4-1.4M10.8 13.2l-6.6 6.6a1.5 1.5 0 0 1-2.1-2.1l6.6-6.6m8.5 2.1l1.4 1.4M10.8 5.2L9.4 3.8"/>"#
        ),
        "cancel" => hgi_svg!(r#"<path d="M18 6L6 18M6 6l12 12"/>"#),
        "key" => hgi_svg!(r#"<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9m-2 2l2 2m-4-1l2 2"/>"#),
        "link-up" => hgi_svg!(r#"<path d="M7 17L17 7m0 0H9m8 0v8"/>"#),
        "paint" => hgi_svg!(
            r#"<circle cx="12" cy="12" r="9"/><circle cx="8.5" cy="10.5" r="1"/><circle cx="12" cy="8" r="1"/><circle cx="15.5" cy="10.5" r="1"/><path d="M15 15.5a2 2 0 0 1-2 2h-1a1.5 1.5 0 0 0 0 3"/>"#
        ),
        "keyboard" => hgi_svg!(
            r#"<rect x="2" y="5" width="20" height="14" rx="2.5"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M18 13h.01M9 15h6"/>"#
        ),
        "database" => hgi_svg!(
            r#"<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.66 3.58 3 8 3s8-1.34 8-3V5"/><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"/>"#
        ),
        "sliders" => hgi_svg!(r#"<path d="M4 21v-7m0-4V3m8 18v-9m0-4V3m8 18v-5m0-4V3M2 14h4m4-6h4m4 8h4"/>"#),
        "star" => hgi_svg!(r#"<path d="M12 2.8l2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.34l-5.7 2.99 1.09-6.34-4.61-4.49 6.37-.93L12 2.8z"/>"#),
        "star-filled" => concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="black" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
            r#"<path d="M12 2.8l2.85 5.77 6.37.93-4.61 4.49 1.09 6.34L12 17.34l-5.7 2.99 1.09-6.34-4.61-4.49 6.37-.93L12 2.8z"/>"#,
            r#"</svg>"#
        ).as_bytes(),
        "copy" => hgi_svg!(r#"<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>"#),
        "file-code" => hgi_svg!(r#"<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6M10 13l-2 2 2 2M14 13l2 2-2 2"/>"#),
        "cloud" => hgi_svg!(r#"<path d="M6.5 19h11a4.5 4.5 0 0 0 .9-8.9A6 6 0 0 0 6.7 8.4 4.2 4.2 0 0 0 6.5 19z"/>"#),
        _ => hgi_svg!(r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 10h18M9 10v10"/>"#),
    }
}

/// Renders a crisp Hugeicons SVG element tinted with `color`.
pub fn hgi(name: &str, size_px: f32, color: Hsla) -> Svg {
    svg()
        .data(hgi_bytes(name))
        .size(px(size_px))
        .flex_shrink_0()
        .text_color(color)
}

/// Renders a macOS System Settings squircle icon (22x22, rounded 6px, gradient background, 13px white Hugeicon).
pub fn macos_icon(top_hex: u32, bottom_hex: u32, icon_name: &str) -> AnyElement {
    div()
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .shadow_sm()
        .bg(linear_gradient(
            180.0,
            linear_color_stop(rgb(top_hex), 0.0),
            linear_color_stop(rgb(bottom_hex), 1.0),
        ))
        .child(hgi(icon_name, 13.0, gpui::white()))
        .into_any_element()
}

fn db_svg_image(driver: DbDriver, mono: bool) -> Arc<Image> {
    match (driver, mono) {
        (DbDriver::Postgres, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M17.2 4.3c-1.4-.9-3.2-.8-4.5-.1-.8-.4-1.8-.6-2.8-.5-2.6.2-4.6 2.1-5.1 4.8-.5 2.5.4 5.5 2 7.2.7.8 1.6 1.1 2.4 1 .3 1.4.8 2.8 1.9 3.5.7.4 1.6.4 2.2-.1.5-.4.7-1.1.7-1.8 0-.4.2-.7.6-.7.6 0 1.4.2 2.1-.2 1.2-.7 1.7-2.2 1.8-3.5.1-1.3.1-2.7.5-3.9.5-1.5.7-3.4-.3-4.6-.4-.5-.9-.8-1.5-1.1z" fill="#ffffff"/><circle cx="9.6" cy="9.2" r="1" fill="#336791"/><path d="M12.8 11.4c.5 1.8.6 3.9.3 5.7" stroke="#336791" stroke-width="1.3" stroke-linecap="round"/><path d="M16.4 8.2c.6 1.2.4 3-.2 4.4" stroke="#336791" stroke-width="1.2" stroke-linecap="round"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Postgres, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M17.2 4.3c-1.4-.9-3.2-.8-4.5-.1-.8-.4-1.8-.6-2.8-.5-2.6.2-4.6 2.1-5.1 4.8-.5 2.5.4 5.5 2 7.2.7.8 1.6 1.1 2.4 1 .3 1.4.8 2.8 1.9 3.5.7.4 1.6.4 2.2-.1.5-.4.7-1.1.7-1.8 0-.4.2-.7.6-.7.6 0 1.4.2 2.1-.2 1.2-.7 1.7-2.2 1.8-3.5.1-1.3.1-2.7.5-3.9.5-1.5.7-3.4-.3-4.6-.4-.5-.9-.8-1.5-1.1z" fill="#336791"/><circle cx="9.6" cy="9.2" r="1" fill="#ffffff"/><path d="M12.8 11.4c.5 1.8.6 3.9.3 5.7" stroke="#ffffff" stroke-width="1.3" stroke-linecap="round"/><path d="M16.4 8.2c.6 1.2.4 3-.2 4.4" stroke="#ffffff" stroke-width="1.2" stroke-linecap="round"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Mysql, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M4.5 17.5c1.5-4.2 4.3-8.6 8.6-10.3 2.1-.8 4.4-.4 5.9 1.1.9.9 1.3 2.1 1 3.3-.4 1.5-1.7 2.4-3.1 2.7-1.8.4-3.4-.2-4.4-1.2-.5 1.3-1.4 2.6-2.7 3.4-.8.5-1.8.8-2.8.8" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><path d="M15.2 9.4l2.4-1.6" stroke="#ffffff" stroke-width="1.8" stroke-linecap="round"/><circle cx="15.8" cy="10.4" r="1" fill="#ffffff"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Mysql, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M4.5 17.5c1.5-4.2 4.3-8.6 8.6-10.3 2.1-.8 4.4-.4 5.9 1.1.9.9 1.3 2.1 1 3.3-.4 1.5-1.7 2.4-3.1 2.7-1.8.4-3.4-.2-4.4-1.2-.5 1.3-1.4 2.6-2.7 3.4-.8.5-1.8.8-2.8.8" stroke="#00758F" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><path d="M15.2 9.4l2.4-1.6" stroke="#F29111" stroke-width="1.8" stroke-linecap="round"/><circle cx="15.8" cy="10.4" r="1" fill="#00758F"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Sqlite, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="3.5" y="4" width="13.5" height="16" rx="2.5" fill="rgba(255,255,255,0.22)" stroke="#ffffff" stroke-width="1.5"/><path d="M19.5 4.5c-3.8 2.2-6.8 6.4-8.2 12.2 2.4-2.5 5.4-4.3 8.4-5.2-.3-2.5-.2-4.8-.2-7z" fill="#ffffff"/><path d="M7 9h4M7 12.5h3" stroke="#ffffff" stroke-width="1.4" stroke-linecap="round"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Sqlite, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="3.5" y="4" width="13.5" height="16" rx="2.5" fill="#003B57" stroke="#0F80CC" stroke-width="1.5"/><path d="M19.5 4.5c-3.8 2.2-6.8 6.4-8.2 12.2 2.4-2.5 5.4-4.3 8.4-5.2-.3-2.5-.2-4.8-.2-7z" fill="#38BDF8"/><path d="M7 9h4M7 12.5h3" stroke="#93C5FD" stroke-width="1.4" stroke-linecap="round"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Redis, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M12 15.5l8.5-3.3v2.6L12 18.2 3.5 14.8v-2.6L12 15.5z" fill="rgba(255,255,255,0.65)"/><path d="M12 12.2l8.5-3.3v2.5L12 14.8 3.5 11.4V8.9L12 12.2z" fill="rgba(255,255,255,0.82)"/><path d="M12 5.4l8.5 3.4L12 12.2 3.5 8.8 12 5.4z" fill="#ffffff"/><circle cx="12" cy="8.5" r="1.2" fill="#DC382D"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Redis, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M12 15.5l8.5-3.3v2.6L12 18.2 3.5 14.8v-2.6L12 15.5z" fill="#A41E11"/><path d="M12 12.2l8.5-3.3v2.5L12 14.8 3.5 11.4V8.9L12 12.2z" fill="#C62828"/><path d="M12 5.4l8.5 3.4L12 12.2 3.5 8.8 12 5.4z" fill="#DC382D"/><circle cx="12" cy="8.5" r="1.2" fill="#ffffff"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Clickhouse, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="3.5" y="5" width="2.4" height="14" rx="1" fill="#ffffff"/><rect x="3.5" y="15.5" width="2.4" height="3.5" rx="0.8" fill="rgba(255,255,255,0.55)"/><rect x="7.6" y="5" width="2.4" height="14" rx="1" fill="#ffffff"/><rect x="11.7" y="5" width="2.4" height="14" rx="1" fill="#ffffff"/><rect x="15.8" y="5" width="2.4" height="14" rx="1" fill="#ffffff"/><rect x="19.9" y="9.5" width="2.2" height="5" rx="1" fill="#ffffff"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Clickhouse, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="3.5" y="5" width="2.4" height="14" rx="1" fill="#FFCC01"/><rect x="3.5" y="15.5" width="2.4" height="3.5" rx="0.8" fill="#EF4444"/><rect x="7.6" y="5" width="2.4" height="14" rx="1" fill="#FFCC01"/><rect x="11.7" y="5" width="2.4" height="14" rx="1" fill="#FFCC01"/><rect x="15.8" y="5" width="2.4" height="14" rx="1" fill="#FFCC01"/><rect x="19.9" y="9.5" width="2.2" height="5" rx="1" fill="#FFCC01"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Mariadb, true) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M3.5 16.5c2.2-1.2 4-3.8 5.8-6.4 1.4-2 3.3-3.2 5.4-2.8 1.7.3 3.1 1.6 4.8 1.5-1.1 1.7-2.6 2.6-4.1 3.2.8 1.3.7 3-.3 4.2-1.4 1.5-4.4 1.7-7.2 1.2-1.6-.3-3.1-.4-4.4-.9z" fill="#ffffff"/><circle cx="15.4" cy="9.6" r="0.9" fill="#003545"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
        (DbDriver::Mariadb, false) => {
            static IMG: OnceLock<Arc<Image>> = OnceLock::new();
            IMG.get_or_init(|| {
                Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><path d="M3.5 16.5c2.2-1.2 4-3.8 5.8-6.4 1.4-2 3.3-3.2 5.4-2.8 1.7.3 3.1 1.6 4.8 1.5-1.1 1.7-2.6 2.6-4.1 3.2.8 1.3.7 3-.3 4.2-1.4 1.5-4.4 1.7-7.2 1.2-1.6-.3-3.1-.4-4.4-.9z" fill="#003545"/><circle cx="15.4" cy="9.6" r="0.9" fill="#ffffff"/></svg>"##.to_vec(),
                ))
            })
            .clone()
        }
    }
}

/// Renders an official Database Brand SVG mark (mono white for colored squircles, or full brand colors).
pub fn db_driver_icon(driver: DbDriver, size_px: f32, mono: bool) -> AnyElement {
    img(db_svg_image(driver, mono))
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
        .into_any_element()
}
