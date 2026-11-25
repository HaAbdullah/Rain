//! The mapping from the colour keys of a VS Code theme to the colour tokens of Rain
//! (`src/styles/tokens.css`). Pure functions.
//!
//! Every token of a Rain theme gets a value, so no token falls back to another theme. A key
//! that the theme does not have is replaced by a mix of neighbouring colours, or (for the
//! terminal colours) by the colour VS Code itself uses by default. Then the colours that carry
//! meaning are nudged in lightness until they reach the contrast Rain promises:
//!
//! - text on every surface: 4.5 to 1 (`foreground`, `muted-foreground`, the status colours);
//! - quiet text (`text-3`, hints): 4.5 to 1 like the built-in themes (the least asked for is 3);
//! - input borders, the focus ring, chart colours: 3 to 1;
//! - the divider `border`: 1.4 to 1, so that it can be seen at all;
//! - the labels on a heat step or a file block: 4.5 to 1.
//!
//! The layers of a Rain window, from the back to the front: `surface-0` (wells), `background`,
//! `surface-1` (header, panes, popovers), `surface-2` (cards, inputs), `hover`, `selected`.

use std::collections::BTreeMap;

use super::color::{BLACK, Dir, Rgb, WHITE, contrast, nudge, parse};

/// What `GET /themes/vscode/theme` fills in. The names are the custom properties of
/// tokens.css without the leading `--`.
pub type Tokens = BTreeMap<String, String>;

/// VS Code's own default terminal colours, used when a theme does not set one.
/// (dark theme value, light theme value)
const DEFAULT_ANSI: [(&str, u32, u32); 6] = [
    ("Blue", 0x2472c8, 0x0451a5),
    ("Cyan", 0x11a8cd, 0x0598bc),
    ("Green", 0x0dbc79, 0x00bc00),
    ("Magenta", 0xbc3fbc, 0xbc05bc),
    ("Red", 0xcd3131, 0xcd3131),
    ("Yellow", 0xe5e510, 0x949800),
];

/// The near-black used for text on a bright fill.
const INK: Rgb = Rgb::from_u32(0x0b0b0c);

struct Palette<'a> {
    colors: &'a BTreeMap<String, String>,
}

impl Palette<'_> {
    /// The first of `keys` that holds a colour, laid over `base` when it is see-through.
    fn first(&self, keys: &[&str], base: Rgb) -> Option<Rgb> {
        keys.iter()
            .find_map(|key| self.colors.get(*key).and_then(|text| parse(text)))
            .map(|colour| colour.over(base))
    }

    /// The first of `keys` that holds a colour that is (nearly) solid. A see-through accent
    /// colour would turn dull when laid over the background, so it is not taken here.
    fn first_solid(&self, keys: &[&str]) -> Option<Rgb> {
        keys.iter()
            .filter_map(|key| self.colors.get(*key).and_then(|text| parse(text)))
            .find(|colour| colour.alpha >= 0.95)
            .map(|colour| colour.rgb)
    }
}

/// Text that reads on `bg`: `preferred` when it reaches 4.5 to 1, else white or near-black,
/// whichever is better.
fn readable_on(bg: Rgb, preferred: Option<Rgb>) -> Rgb {
    let candidates = preferred.into_iter().chain([WHITE, INK]);
    let mut best = WHITE;
    let mut best_ratio = 0.0;
    for candidate in candidates {
        let ratio = contrast(candidate, bg);
        if ratio >= 4.5 {
            return candidate;
        }
        if ratio > best_ratio {
            best = candidate;
            best_ratio = ratio;
        }
    }
    best
}

fn rgb_alpha(colour: Rgb, alpha: f64) -> String {
    let (r, g, b) = colour.bytes();
    format!("rgb({r} {g} {b} / {alpha})")
}

/// All tokens of one Rain theme, made from the `colors` of a VS Code theme. `dark` says whether
/// it is a dark theme (it picks the direction of every nudge and the defaults for missing keys).
pub fn map_tokens(colors: &BTreeMap<String, String>, dark: bool) -> Tokens {
    let p = Palette { colors };
    let pick = |dark_value: u32, light_value: u32| {
        Rgb::from_u32(if dark { dark_value } else { light_value })
    };
    // Surfaces move away from the text colour, text moves toward it: lighter on a dark theme.
    let toward_text = if dark { Dir::Lighter } else { Dir::Darker };

    // ---- surfaces ----
    let fallback_bg = pick(0x1e1e1e, 0xffffff);
    let background = p
        .first(&["editor.background"], fallback_bg)
        .unwrap_or(fallback_bg);
    let fg_raw = p
        .first(&["editor.foreground", "foreground"], background)
        .unwrap_or_else(|| pick(0xcccccc, 0x333333));

    let surface_0 = background.mix(BLACK, if dark { 0.3 } else { 0.05 });
    let surface_1 = {
        let raw = p
            .first(
                &["editorWidget.background", "dropdown.background"],
                background,
            )
            .unwrap_or_else(|| background.mix(fg_raw, 0.05));
        nudge(raw, &[background], 1.06, toward_text)
    };
    let surface_2 = nudge(surface_1.mix(fg_raw, 0.05), &[surface_1], 1.06, toward_text);
    let hover = {
        let raw = p
            .first(&["list.hoverBackground"], surface_1)
            .unwrap_or_else(|| surface_2.mix(fg_raw, 0.06));
        nudge(raw, &[surface_2], 1.1, toward_text)
    };
    let selected = {
        let raw = p
            .first(
                &[
                    "list.activeSelectionBackground",
                    "list.inactiveSelectionBackground",
                ],
                surface_1,
            )
            .unwrap_or_else(|| hover.mix(fg_raw, 0.08));
        nudge(raw, &[surface_1, hover], 1.15, toward_text)
    };
    let surfaces = [background, surface_0, surface_1, surface_2, hover, selected];
    let plain = [background, surface_1, surface_2];

    // ---- text ----
    let foreground = nudge(fg_raw, &surfaces, 4.5, toward_text);
    let muted_foreground = {
        let raw = p
            .first(&["descriptionForeground"], surface_1)
            .unwrap_or_else(|| foreground.mix(background, 0.3));
        nudge(raw, &surfaces, 4.5, toward_text)
    };
    let text_3 = nudge(
        muted_foreground.mix(background, 0.3),
        &surfaces,
        4.5,
        toward_text,
    );

    // ---- the rail ----
    let sidebar = p
        .first(
            &["activityBar.background", "sideBar.background"],
            background,
        )
        .unwrap_or(surface_1);
    let sidebar_foreground = {
        let raw = p
            .first(&["sideBar.foreground", "foreground"], sidebar)
            .unwrap_or(foreground);
        nudge(raw, &[sidebar, hover], 4.5, toward_text)
    };

    // ---- lines and focus ----
    let border = {
        let raw = p
            .first(
                &["panel.border", "editorGroup.border", "sideBar.border"],
                background,
            )
            .unwrap_or_else(|| background.mix(fg_raw, 0.15));
        nudge(raw, &plain, 1.4, toward_text)
    };
    let border_strong = {
        let raw = p
            .first(
                &["input.border", "dropdown.border", "checkbox.border"],
                background,
            )
            .unwrap_or_else(|| background.mix(fg_raw, 0.4));
        nudge(
            raw,
            &[background, surface_0, surface_1, surface_2],
            3.0,
            toward_text,
        )
    };

    // ---- the accent ----
    let accent_raw = p
        .first_solid(&[
            "button.background",
            "activityBarBadge.background",
            "badge.background",
            "progressBar.background",
            "textLink.foreground",
        ])
        .or_else(|| p.first(&["button.background", "focusBorder"], background))
        .unwrap_or_else(|| Rgb::from_u32(0x007acc));
    let primary = nudge(accent_raw, &[background, surface_1], 3.0, toward_text);
    let primary_foreground = readable_on(primary, p.first(&["button.foreground"], primary));
    let ring = {
        let raw = p.first(&["focusBorder"], background).unwrap_or(primary);
        nudge(raw, &plain, 3.0, toward_text)
    };
    let selection_bg = p
        .first(
            &["selection.background", "editor.selectionBackground"],
            background,
        )
        .unwrap_or(primary);
    let selection_fg = readable_on(selection_bg, Some(foreground));

    // ---- status and charts: the terminal colours ----
    let ansi = |name: &str| -> Rgb {
        let (_, dark_value, light_value) = DEFAULT_ANSI
            .iter()
            .find(|(candidate, _, _)| *candidate == name)
            .copied()
            .unwrap_or(("", 0x808080, 0x808080));
        let normal = format!("terminal.ansi{name}");
        let bright = format!("terminal.ansiBright{name}");
        p.first(&[normal.as_str(), bright.as_str()], background)
            .unwrap_or_else(|| pick(dark_value, light_value))
    };
    // Readable on the plain surfaces, and still visible on the selected row.
    let status = |colour: Rgb| {
        let readable = nudge(colour, &plain, 4.5, toward_text);
        nudge(readable, &[selected], 3.0, toward_text)
    };
    let status_failed = {
        let raw = p
            .first(
                &[
                    "terminal.ansiRed",
                    "errorForeground",
                    "editorError.foreground",
                ],
                background,
            )
            .unwrap_or_else(|| ansi("Red"));
        status(raw)
    };
    let chart = |colour: Rgb| nudge(colour, &[background, surface_2], 3.0, toward_text);
    let (blue, cyan, yellow, magenta, green, red) = (
        ansi("Blue"),
        ansi("Cyan"),
        ansi("Yellow"),
        ansi("Magenta"),
        ansi("Green"),
        ansi("Red"),
    );

    // ---- the heat steps: one hue, from the accent ----
    let on_heat = if dark {
        background.mix(BLACK, 0.8)
    } else {
        WHITE
    };
    let heat_base = nudge(primary, &[on_heat], 4.5, toward_text);
    let heat_end = if dark {
        heat_base.lightness().max(0.85)
    } else {
        heat_base.lightness().min(0.15)
    };
    let heat_step = |i: usize| {
        let t = i as f64 / 3.0;
        let l = heat_base.lightness() + (heat_end - heat_base.lightness()) * t;
        heat_base.with_lightness(l)
    };

    // ---- file categories: a label on each block ----
    let on_file = on_heat;
    let file = |colour: Rgb| nudge(colour, &[on_file], 4.5, toward_text);

    let mut out = Tokens::new();
    let mut put = |name: &str, colour: Rgb| {
        out.insert(name.to_owned(), colour.hex());
    };

    // shadcn-vue variables
    put("background", background);
    put("foreground", foreground);
    put("card", surface_2);
    put("card-foreground", foreground);
    put("popover", surface_1);
    put("popover-foreground", foreground);
    put("primary", primary);
    put("primary-foreground", primary_foreground);
    put("secondary", surface_2);
    put("secondary-foreground", foreground);
    put("muted", surface_2);
    put("muted-foreground", muted_foreground);
    put("accent", hover);
    put("accent-foreground", foreground);
    put("destructive", status_failed);
    put("destructive-foreground", readable_on(status_failed, None));
    put("border", border);
    put("input", border_strong);
    put("ring", ring);
    // charts
    put("chart-1", chart(blue));
    put("chart-2", chart(cyan));
    put("chart-3", chart(yellow));
    put("chart-4", chart(magenta));
    put("chart-5", chart(green));
    // sidebar
    put("sidebar", sidebar);
    put("sidebar-foreground", sidebar_foreground);
    put("sidebar-primary", primary);
    put("sidebar-primary-foreground", primary_foreground);
    put("sidebar-accent", hover);
    put("sidebar-accent-foreground", foreground);
    put("sidebar-border", border);
    put("sidebar-ring", ring);
    // Rain surfaces
    put("surface-0", surface_0);
    put("surface-1", surface_1);
    put("surface-2", surface_2);
    put("hover", hover);
    put("selected", selected);
    put("border-strong", border_strong);
    put("text-3", text_3);
    // status
    put("status-running", status(blue));
    put("status-waiting", status(yellow));
    put("status-success", status(green));
    put("status-failed", status_failed);
    put("status-idle", muted_foreground);
    // heat
    put("heat-0", surface_2.mix(fg_raw, 0.12));
    for i in 1..=4 {
        put(&format!("heat-{i}"), heat_step(i - 1));
    }
    put("on-heat", on_heat);
    // files
    put("file-video", file(magenta));
    put("file-image", file(cyan));
    put("file-audio", file(red.mix(yellow, 0.5)));
    put("file-archive", file(yellow));
    put("file-code", file(blue));
    put("file-document", file(green));
    put("file-app", file(red.mix(magenta, 0.4)));
    put("file-other", file(foreground.mix(background, 0.5)));
    put("on-file", on_file);
    // scrollbar, selection
    let thumb = {
        let raw = p
            .first(&["scrollbarSlider.background"], background)
            .unwrap_or_else(|| background.mix(fg_raw, 0.2));
        nudge(raw, &[background, surface_1], 2.0, toward_text)
    };
    let thumb_hover = {
        let raw = p
            .first(&["scrollbarSlider.hoverBackground"], background)
            .unwrap_or_else(|| background.mix(fg_raw, 0.35));
        nudge(raw, &[background, surface_1], 3.0, toward_text)
    };
    put("scrollbar-thumb", thumb);
    put("scrollbar-thumb-hover", thumb_hover);
    put("selection-bg", selection_bg);
    put("selection-fg", selection_fg);

    // overlays are not plain colours
    let (scrim, scrim_strong, shadow) = if dark {
        let tint = background.mix(BLACK, 0.75);
        (
            rgb_alpha(tint, 0.4),
            rgb_alpha(tint, 0.8),
            "0 8px 24px rgb(0 0 0 / 0.5), inset 0 1px 0 rgb(255 255 255 / 0.08)",
        )
    } else {
        let tint = foreground.mix(BLACK, 0.3);
        (
            rgb_alpha(tint, 0.2),
            rgb_alpha(tint, 0.6),
            "0 8px 24px rgb(20 28 48 / 0.18), 0 0 0 1px rgb(20 28 48 / 0.08)",
        )
    };
    out.insert("scrim".to_owned(), scrim);
    out.insert("scrim-strong".to_owned(), scrim_strong);
    out.insert("shadow-overlay".to_owned(), shadow.to_owned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn colors(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn colour(tokens: &Tokens, name: &str) -> Rgb {
        parse(&tokens[name])
            .unwrap_or_else(|| panic!("{name} is not a colour"))
            .rgb
    }

    /// A made-up dark theme: only a few keys, some of them see-through or too dim.
    fn dim_dark() -> BTreeMap<String, String> {
        colors(&[
            ("editor.background", "#14171c"),
            ("editor.foreground", "#9aa3ad"),
            ("sideBar.background", "#0f1216"),
            ("editorWidget.background", "#1b2027"),
            ("descriptionForeground", "#9aa3ad66"),
            ("focusBorder", "#2c3540"),
            ("button.background", "#3070d080"),
            ("badge.background", "#3a7bd5"),
            ("list.hoverBackground", "#ffffff10"),
            ("list.activeSelectionBackground", "#ffffff1c"),
            ("panel.border", "#07080a"),
            ("input.border", "#262c33"),
            ("terminal.ansiGreen", "#1e7a4d"),
            ("terminal.ansiRed", "#7a1f2b"),
            ("terminal.ansiYellow", "#6e5a10"),
            ("terminal.ansiBlue", "#1f4f8a"),
            ("scrollbarSlider.background", "#9aa3ad18"),
        ])
    }

    fn pale_light() -> BTreeMap<String, String> {
        colors(&[
            ("editor.background", "#fbfaf5"),
            ("editor.foreground", "#a8a49a"),
            ("editorWidget.background", "#f2efe6"),
            ("descriptionForeground", "#c0bcb0"),
            ("focusBorder", "#e0dccf"),
            ("button.background", "#f0c040"),
            ("panel.border", "#f6f3ea"),
            ("terminal.ansiGreen", "#a6e3a1"),
            ("terminal.ansiRed", "#f4a0a0"),
        ])
    }

    /// The names of the custom properties of one theme block of tokens.css, read from the file.
    fn css_token_names(theme: &str) -> Vec<String> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/styles/tokens.css");
        let css = std::fs::read_to_string(path).expect("tokens.css");
        let marker = format!("data-theme=\"{theme}\"]");
        let start = css.find(&marker).expect("the theme block");
        let body_start = start + css[start..].find('{').unwrap();
        let body_end = body_start + css[body_start..].find("\n}").unwrap();
        css[body_start..body_end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let name = line.strip_prefix("--")?;
                Some(name.split(':').next()?.trim().to_owned())
            })
            .collect()
    }

    #[test]
    fn every_token_of_every_builtin_theme_gets_a_value_and_nothing_else() {
        let mut expected = css_token_names("dark");
        expected.sort();
        assert!(expected.len() > 60);
        for (label, theme, dark) in [
            ("empty dark", BTreeMap::new(), true),
            ("empty light", BTreeMap::new(), false),
            ("sparse dark", dim_dark(), true),
            ("sparse light", pale_light(), false),
        ] {
            let tokens = map_tokens(&theme, dark);
            let names: Vec<String> = tokens.keys().cloned().collect();
            assert_eq!(names, expected, "{label}");
            for (name, value) in &tokens {
                assert!(!value.trim().is_empty(), "{label}: {name}");
            }
        }
    }

    #[test]
    fn the_light_block_defines_the_same_names_as_the_dark_block() {
        // The test above takes the dark block as the list; this keeps the list honest.
        let mut dark = css_token_names("dark");
        let mut light = css_token_names("light");
        dark.sort();
        light.sort();
        // color-scheme is not a custom property, so it is not in either list.
        assert_eq!(dark, light);
    }

    /// The promises of the module, checked on a theme.
    fn assert_readable(label: &str, tokens: &Tokens) {
        let c = |name: &str| colour(tokens, name);
        let surfaces: Vec<Rgb> = [
            "background",
            "surface-0",
            "surface-1",
            "surface-2",
            "hover",
            "selected",
        ]
        .iter()
        .map(|name| c(name))
        .collect();
        let plain = [c("background"), c("surface-1"), c("surface-2")];
        let at_least = |what: &str, colour: Rgb, against: &[Rgb], min: f64| {
            for bg in against {
                let ratio = contrast(colour, *bg);
                assert!(
                    ratio >= min - 0.001,
                    "{label}: {what} {} on {} is {ratio:.2}, wanted {min}",
                    colour.hex(),
                    bg.hex()
                );
            }
        };
        at_least("foreground", c("foreground"), &surfaces, 4.5);
        at_least("card-foreground", c("card-foreground"), &surfaces, 4.5);
        at_least("muted-foreground", c("muted-foreground"), &surfaces, 4.5);
        at_least("text-3", c("text-3"), &surfaces, 4.5);
        at_least(
            "sidebar-foreground",
            c("sidebar-foreground"),
            &[c("sidebar"), c("hover")],
            4.5,
        );
        at_least(
            "primary-foreground",
            c("primary-foreground"),
            &[c("primary")],
            4.5,
        );
        at_least(
            "destructive-foreground",
            c("destructive-foreground"),
            &[c("destructive")],
            4.5,
        );
        at_least("selection-fg", c("selection-fg"), &[c("selection-bg")], 4.5);
        for status in ["running", "waiting", "success", "failed", "idle"] {
            at_least(status, c(&format!("status-{status}")), &plain, 4.5);
        }
        at_least("ring", c("ring"), &plain, 3.0);
        at_least("input", c("input"), &plain, 3.0);
        at_least("border", c("border"), &plain, 1.4);
        for i in 1..=5 {
            at_least(
                &format!("chart-{i}"),
                c(&format!("chart-{i}")),
                &[c("background"), c("surface-2")],
                3.0,
            );
        }
        for step in 1..=4 {
            at_least(
                &format!("heat-{step}"),
                c(&format!("heat-{step}")),
                &[c("on-heat")],
                4.5,
            );
        }
        for kind in [
            "video", "image", "audio", "archive", "code", "document", "app", "other",
        ] {
            at_least(kind, c(&format!("file-{kind}")), &[c("on-file")], 4.5);
        }
        at_least(
            "scrollbar-thumb",
            c("scrollbar-thumb"),
            &[c("background"), c("surface-1")],
            2.0,
        );
    }

    #[test]
    fn readable_contrast_is_guaranteed_whatever_the_theme_sets() {
        assert_readable("empty dark", &map_tokens(&BTreeMap::new(), true));
        assert_readable("empty light", &map_tokens(&BTreeMap::new(), false));
        assert_readable("sparse dark", &map_tokens(&dim_dark(), true));
        assert_readable("pale light", &map_tokens(&pale_light(), false));
    }

    #[test]
    fn the_layers_keep_their_order_in_a_dark_theme() {
        let tokens = map_tokens(&dim_dark(), true);
        let lum = |name: &str| colour(&tokens, name).luminance();
        assert!(lum("surface-0") < lum("background"));
        assert!(lum("background") < lum("surface-1"));
        assert!(lum("surface-1") < lum("surface-2"));
        assert!(lum("surface-2") < lum("hover"));
    }

    #[test]
    fn the_layers_keep_their_order_in_a_light_theme() {
        let tokens = map_tokens(&pale_light(), false);
        let lum = |name: &str| colour(&tokens, name).luminance();
        assert!(lum("surface-0") < lum("background"));
        assert!(lum("surface-1") < lum("background"));
        assert!(lum("surface-2") < lum("surface-1"));
    }

    #[test]
    fn the_colours_of_the_theme_win_over_the_defaults() {
        let theme = colors(&[
            ("editor.background", "#101820"),
            ("editor.foreground", "#e8eef4"),
            ("sideBar.background", "#0a1016"),
            ("badge.background", "#d0407f"),
            ("terminal.ansiGreen", "#33cc77"),
        ]);
        let tokens = map_tokens(&theme, true);
        assert_eq!(tokens["background"], "#101820");
        assert_eq!(tokens["sidebar"], "#0a1016");
        assert_eq!(tokens["primary"], "#d0407f");
        assert_eq!(tokens["status-success"], "#33cc77");
    }

    #[test]
    fn a_see_through_button_colour_is_skipped_for_a_solid_accent() {
        let theme = colors(&[
            ("editor.background", "#101820"),
            ("button.background", "#ff000080"),
            ("badge.background", "#2090e0"),
        ]);
        assert_eq!(map_tokens(&theme, true)["primary"], "#2090e0");
    }

    #[test]
    fn a_see_through_button_colour_is_used_when_nothing_solid_exists() {
        let theme = colors(&[
            ("editor.background", "#000000"),
            ("button.background", "#ffffff80"),
        ]);
        let tokens = map_tokens(&theme, true);
        // White at half over black is mid grey, and that is what the button looks like.
        assert_eq!(tokens["primary"], "#808080");
    }

    #[test]
    fn values_that_are_not_colours_are_ignored() {
        let theme = colors(&[("editor.background", "transparent"), ("foreground", "red")]);
        let tokens = map_tokens(&theme, true);
        assert_eq!(tokens["background"], "#1e1e1e");
    }

    #[test]
    fn the_overlay_tokens_are_complete_css_values() {
        let dark = map_tokens(&dim_dark(), true);
        assert!(dark["scrim"].starts_with("rgb(") && dark["scrim"].ends_with(" / 0.4)"));
        assert!(dark["scrim-strong"].ends_with(" / 0.8)"));
        assert!(dark["shadow-overlay"].contains("inset"));
        let light = map_tokens(&pale_light(), false);
        assert!(light["scrim"].ends_with(" / 0.2)"));
    }
}
