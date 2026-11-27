use gpui::{hsla, px, rgb, Hsla, Pixels, Rgba};

#[derive(Clone, Copy, Debug)]
pub struct VeloTheme {
    pub is_dark: bool,
    pub font_size: f32,
    pub window_bg: Hsla,
    pub sidebar_bg: Hsla,
    pub titlebar_bg: Hsla,
    pub panel_bg: Hsla,
    pub surface_1: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text_1: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub green: Hsla,
    pub green_soft: Hsla,
    pub amber: Hsla,
    pub amber_soft: Hsla,
    pub red: Hsla,
    pub red_soft: Hsla,
    pub purple: Hsla,
    pub purple_soft: Hsla,
    pub cyan: Hsla,
    pub cyan_soft: Hsla,
    pub row_hover: Hsla,
    pub row_selected: Hsla,
}

impl VeloTheme {
    pub fn from_mode(
        is_dark: bool,
        vibrancy: bool,
        conn_accent_hex: Option<&str>,
        font_size: f32,
    ) -> Self {
        let base_fs = font_size.clamp(11.0, 16.0);
        let mut t = if is_dark {
            Self {
                is_dark: true,
                font_size: base_fs,
                window_bg: if vibrancy { hsla(222.0 / 360.0, 0.18, 0.11, 0.88) } else { rgb(0x171a21).into() },
                sidebar_bg: if vibrancy { hsla(223.0 / 360.0, 0.20, 0.14, 0.72) } else { rgb(0x1c202a).into() },
                titlebar_bg: if vibrancy { hsla(223.0 / 360.0, 0.19, 0.15, 0.82) } else { rgb(0x1e222c).into() },
                panel_bg: if vibrancy { hsla(222.0 / 360.0, 0.19, 0.11, 0.95) } else { rgb(0x161920).into() },
                surface_1: hsla(0.0, 0.0, 1.0, 0.04),
                surface_2: hsla(0.0, 0.0, 1.0, 0.07),
                surface_3: hsla(0.0, 0.0, 1.0, 0.12),
                border: hsla(0.0, 0.0, 1.0, 0.09),
                border_strong: hsla(0.0, 0.0, 1.0, 0.16),
                text_1: rgb(0xf3f5f8).into(),
                text_2: rgb(0xa3adbd).into(),
                text_3: rgb(0x6c7689).into(),
                accent: rgb(0x3b82f6).into(),
                accent_soft: hsla(217.0 / 360.0, 0.91, 0.60, 0.17),
                green: rgb(0x34d399).into(),
                green_soft: hsla(158.0 / 360.0, 0.64, 0.52, 0.14),
                amber: rgb(0xfbbf24).into(),
                amber_soft: hsla(43.0 / 360.0, 0.96, 0.56, 0.15),
                red: rgb(0xf87171).into(),
                red_soft: hsla(0.0, 0.91, 0.71, 0.15),
                purple: rgb(0xa78bfa).into(),
                purple_soft: hsla(255.0 / 360.0, 0.92, 0.76, 0.15),
                cyan: rgb(0x22d3ee).into(),
                cyan_soft: hsla(188.0 / 360.0, 0.86, 0.53, 0.14),
                row_hover: hsla(0.0, 0.0, 1.0, 0.035),
                row_selected: hsla(217.0 / 360.0, 0.91, 0.60, 0.16),
            }
        } else {
            Self {
                is_dark: false,
                font_size: base_fs,
                window_bg: if vibrancy { hsla(220.0 / 360.0, 0.50, 0.99, 0.92) } else { rgb(0xfcfdff).into() },
                sidebar_bg: if vibrancy { hsla(218.0 / 360.0, 0.36, 0.96, 0.80) } else { rgb(0xf1f4f9).into() },
                titlebar_bg: if vibrancy { hsla(220.0 / 360.0, 0.38, 0.98, 0.86) } else { rgb(0xf6f8fc).into() },
                panel_bg: if vibrancy { hsla(0.0, 0.0, 1.0, 0.97) } else { rgb(0xffffff).into() },
                surface_1: hsla(222.0 / 360.0, 0.47, 0.11, 0.035),
                surface_2: hsla(222.0 / 360.0, 0.47, 0.11, 0.06),
                surface_3: hsla(222.0 / 360.0, 0.47, 0.11, 0.10),
                border: hsla(222.0 / 360.0, 0.47, 0.11, 0.09),
                border_strong: hsla(222.0 / 360.0, 0.47, 0.11, 0.16),
                text_1: rgb(0x0f172a).into(),
                text_2: rgb(0x475569).into(),
                text_3: rgb(0x94a3b8).into(),
                accent: rgb(0x2563eb).into(),
                accent_soft: hsla(221.0 / 360.0, 0.83, 0.53, 0.11),
                green: rgb(0x059669).into(),
                green_soft: hsla(160.0 / 360.0, 0.94, 0.30, 0.11),
                amber: rgb(0xd97706).into(),
                amber_soft: hsla(32.0 / 360.0, 0.95, 0.44, 0.12),
                red: rgb(0xdc2626).into(),
                red_soft: hsla(0.0, 0.72, 0.51, 0.11),
                purple: rgb(0x7c3aed).into(),
                purple_soft: hsla(262.0 / 360.0, 0.83, 0.58, 0.11),
                cyan: rgb(0x0891b2).into(),
                cyan_soft: hsla(192.0 / 360.0, 0.91, 0.36, 0.11),
                row_hover: hsla(222.0 / 360.0, 0.47, 0.11, 0.03),
                row_selected: hsla(221.0 / 360.0, 0.83, 0.53, 0.09),
            }
        };

        if let Some(hex) = conn_accent_hex {
            if let Some(c) = parse_hex(hex) {
                t.accent = c;
                t.accent_soft = with_alpha(c, if is_dark { 0.17 } else { 0.11 });
                t.row_selected = with_alpha(c, if is_dark { 0.16 } else { 0.09 });
            }
        }
        t
    }

    /// Micro labels: badges, data-type pills, keyboard hints, sidebar group headers.
    pub fn fs_xs(&self) -> Pixels {
        px((self.font_size - 2.0).max(9.5))
    }

    /// Secondary UI text: status bar, segmented tabs, metadata, row numbers, form labels.
    pub fn fs_sm(&self) -> Pixels {
        px((self.font_size - 1.0).max(10.5))
    }

    /// Primary UI & data text: sidebar items, search/form inputs, grid cells, buttons, editors.
    pub fn fs_md(&self) -> Pixels {
        px(self.font_size)
    }

    /// Section & modal titles.
    pub fn fs_lg(&self) -> Pixels {
        px(self.font_size + 1.5)
    }

    /// Page headings.
    pub fn fs_xl(&self) -> Pixels {
        px(self.font_size + 3.0)
    }

    /// Hero titles & large KPI numbers.
    pub fn fs_2xl(&self) -> Pixels {
        px(self.font_size + 8.0)
    }

    /// Extra vertical breathing room added when global font size exceeds 12px.
    fn scale_extra(&self) -> f32 {
        (self.font_size - 12.0).max(0.0)
    }

    /// Standard single-line control height (buttons, inputs, tabs, sidebar rows).
    pub fn control_h(&self) -> Pixels {
        px(28.0 + self.scale_extra() * 1.5)
    }

    /// Data grid header height.
    pub fn table_header_h(&self) -> Pixels {
        px(32.0 + self.scale_extra() * 1.5)
    }

    /// Data grid row height adjusted by row density and global font size.
    pub fn table_row_h(&self, density: &str) -> Pixels {
        let base = match density {
            "compact" => 28.0,
            "spacious" => 38.0,
            _ => 32.0,
        };
        px(base + self.scale_extra() * 1.5)
    }

    /// SQL result grid row & header height.
    pub fn sql_row_h(&self) -> Pixels {
        px(30.0 + self.scale_extra() * 1.5)
    }
}

pub fn parse_hex(hex: &str) -> Option<Hsla> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    let rgba = Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    };
    Some(rgba.into())
}

pub fn with_alpha(mut c: Hsla, alpha: f32) -> Hsla {
    c.a = alpha;
    c
}
