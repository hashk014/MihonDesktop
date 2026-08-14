//! Font fallbacks.
//!
//! egui ships a Latin font only, so Japanese, Korean and Chinese titles — most
//! of a manga library — would render as empty boxes. System fonts are appended
//! as fallbacks at startup; each is optional, and the app degrades gracefully
//! when one is missing.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// (key, file, face index inside a `.ttc` collection, what it covers)
#[cfg(target_os = "windows")]
const CANDIDATES: &[(&str, &str, u32, &str)] = &[
    (
        "symbols",
        r"C:\Windows\Fonts\seguisym.ttf",
        0,
        "arrows and symbols",
    ),
    ("japanese", r"C:\Windows\Fonts\YuGothM.ttc", 0, "Japanese"),
    (
        "japanese-alt",
        r"C:\Windows\Fonts\msgothic.ttc",
        0,
        "Japanese",
    ),
    ("korean", r"C:\Windows\Fonts\malgun.ttf", 0, "Korean"),
    (
        "chinese",
        r"C:\Windows\Fonts\msyh.ttc",
        0,
        "Simplified Chinese",
    ),
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[(&str, &str, u32, &str)] = &[
    (
        "japanese",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        0,
        "Japanese",
    ),
    ("cjk", "/System/Library/Fonts/PingFang.ttc", 0, "Chinese"),
    (
        "korean",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        0,
        "Korean",
    ),
];

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
const CANDIDATES: &[(&str, &str, u32, &str)] = &[
    (
        "cjk",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        0,
        "CJK",
    ),
    (
        "cjk-alt",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        0,
        "CJK",
    ),
];

/// Installs the bundled fonts plus whatever system fallbacks are available.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut loaded = Vec::new();

    for (key, path, index, coverage) in CANDIDATES {
        // Skip a redundant fallback when an earlier one already covers the script.
        if loaded.iter().any(|(_, c)| c == coverage) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };

        let data = FontData {
            font: std::borrow::Cow::Owned(bytes),
            index: *index,
            tweak: Default::default(),
        };
        fonts.font_data.insert((*key).to_string(), Arc::new(data));
        loaded.push((key.to_string(), coverage.to_string()));
    }

    if loaded.is_empty() {
        log::warn!("no system font fallback found; non-Latin titles may not render");
        return;
    }

    // Fallbacks go after the bundled fonts: egui walks the list per glyph and
    // keeps the first font that has it.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let entry = fonts.families.entry(family).or_default();
        for (key, _) in &loaded {
            entry.push(key.clone());
        }
    }

    log::info!(
        "font fallbacks loaded: {}",
        loaded
            .iter()
            .map(|(_, coverage)| coverage.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    ctx.set_fonts(fonts);
}

/// Sample strings used by the glyph probe to check script coverage.
pub const SCRIPT_SAMPLES: &[(&str, &str)] = &[
    ("Latin", "Chainsaw Man"),
    ("Japanese", "チェンソーマン 呪術廻戦"),
    ("Korean", "나 혼자만 레벨업"),
    ("Chinese", "斗破苍穹 完结"),
    ("Cyrillic", "Атака титанов"),
    ("Greek", "Ελληνικά"),
];
