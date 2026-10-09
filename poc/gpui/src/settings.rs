//! View settings shared by all windows.

use std::collections::HashSet;

use gpui::{App, Font, FontFallbacks, Global, Pixels, SharedString, font, px};

pub struct FontPreset {
    pub label: &'static str,
    pub family: &'static str,
    pub fallbacks: &'static [&'static str],
}

/// Fonts to compare for the rendering check (docs/poc/ui-framework.md #2).
#[cfg(target_os = "macos")]
pub const FONT_PRESETS: &[FontPreset] = &[
    FontPreset {
        label: "Menlo ＋ ヒラギノ角ゴシック",
        family: "Menlo",
        fallbacks: &["Hiragino Sans"],
    },
    FontPreset {
        label: "Monaco ＋ ヒラギノ角ゴシック",
        family: "Monaco",
        fallbacks: &["Hiragino Sans"],
    },
    FontPreset {
        label: "Osaka（プロポーショナル）",
        family: "Osaka",
        fallbacks: &[],
    },
    FontPreset {
        label: "BIZ UDゴシック",
        family: "BIZ UDGothic",
        fallbacks: &[],
    },
    FontPreset {
        label: "ヒラギノ角ゴシック",
        family: "Hiragino Sans",
        fallbacks: &[],
    },
];

#[cfg(target_os = "windows")]
pub const FONT_PRESETS: &[FontPreset] = &[
    FontPreset {
        label: "BIZ UDゴシック",
        family: "BIZ UDGothic",
        fallbacks: &[],
    },
    FontPreset {
        label: "ＭＳ ゴシック",
        family: "MS Gothic",
        fallbacks: &[],
    },
    FontPreset {
        label: "Consolas ＋ メイリオ",
        family: "Consolas",
        fallbacks: &["Meiryo"],
    },
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const FONT_PRESETS: &[FontPreset] = &[
    FontPreset {
        label: "Noto Sans Mono CJK JP",
        family: "Noto Sans Mono CJK JP",
        fallbacks: &[],
    },
    FontPreset {
        label: "DejaVu Sans Mono ＋ Noto Sans CJK JP",
        family: "DejaVu Sans Mono",
        fallbacks: &["Noto Sans CJK JP"],
    },
];

pub const FONT_SIZES: &[f32] = &[12., 13., 14., 16., 18.];

pub struct Settings {
    pub font_family: SharedString,
    pub font_fallbacks: Vec<String>,
    pub font_size: Pixels,
    /// Lays out full-width characters in two half-width cells, as Sakura does, instead of
    /// trusting the advances of the fonts.
    pub grid_layout: bool,
    pub show_eol: bool,
    /// The preset families installed on this machine, once known. Listing the fonts takes about
    /// 75 ms on a Mac, so it runs in the background after startup.
    installed: Option<HashSet<&'static str>>,
}

impl Global for Settings {}

impl Settings {
    /// The first preset, unless `UME_POC_FONT` and `UME_POC_FONT_SIZE` say otherwise.
    pub fn new() -> Self {
        let preset = &FONT_PRESETS[0];
        let (font_family, font_fallbacks) = match std::env::var("UME_POC_FONT") {
            Ok(family) if !family.is_empty() => (family.into(), Vec::new()),
            _ => (
                preset.family.into(),
                preset.fallbacks.iter().map(|f| f.to_string()).collect(),
            ),
        };
        let font_size = std::env::var("UME_POC_FONT_SIZE")
            .ok()
            .and_then(|size| size.parse::<f32>().ok())
            .filter(|size| (6.0..=72.0).contains(size))
            .unwrap_or(14.);
        Self {
            font_family,
            font_fallbacks,
            font_size: px(font_size),
            grid_layout: true,
            show_eol: true,
            installed: None,
        }
    }

    /// Whether the preset's font is installed. True until the fonts have been listed.
    pub fn is_installed(&self, preset: &FontPreset) -> bool {
        self.installed
            .as_ref()
            .is_none_or(|installed| installed.contains(preset.family))
    }

    pub fn font(&self) -> Font {
        let mut font = font(self.font_family.clone());
        if !self.font_fallbacks.is_empty() {
            font.fallbacks = Some(FontFallbacks::from_fonts(self.font_fallbacks.clone()));
        }
        font
    }

    pub fn line_height(&self) -> Pixels {
        (self.font_size * 1.4).round()
    }

    pub fn is_preset(&self, preset: &FontPreset) -> bool {
        self.font_family.as_ref() == preset.family
    }

    pub fn set_preset(&mut self, preset: &FontPreset) {
        self.font_family = preset.family.into();
        self.font_fallbacks = preset.fallbacks.iter().map(|f| f.to_string()).collect();
    }

    /// The label of the current font for the status bar.
    pub fn font_label(&self) -> SharedString {
        FONT_PRESETS
            .iter()
            .find(|preset| self.is_preset(preset))
            .map_or_else(|| self.font_family.clone(), |preset| preset.label.into())
    }
}

/// Lists the installed fonts in the background, then greys out the presets that are missing.
pub fn find_installed_fonts(cx: &mut App) {
    let text_system = cx.text_system().clone();
    let names = cx
        .background_executor()
        .spawn(async move { text_system.all_font_names() });
    cx.spawn(async move |cx| {
        let names: HashSet<String> = names.await.into_iter().collect();
        cx.update(|cx| {
            cx.global_mut::<Settings>().installed = Some(
                FONT_PRESETS
                    .iter()
                    .map(|preset| preset.family)
                    .filter(|family| names.contains(*family))
                    .collect(),
            );
            crate::menus::set_menus(cx);
        });
    })
    .detach();
}
