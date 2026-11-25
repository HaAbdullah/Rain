//! Colour arithmetic for the theme mapping: parse a hex colour, mix, measure contrast and move a
//! colour's lightness until the contrast is enough. Pure functions, no I/O.

/// An opaque colour, each channel 0 to 255. Kept as floats so that mixing loses nothing; the
/// rounding happens once, in `hex`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

/// A colour that may be see-through (`#rrggbbaa`). `alpha` is 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub rgb: Rgb,
    pub alpha: f64,
}

/// Which way a nudge moves the lightness first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Lighter,
    Darker,
}

impl Dir {
    fn opposite(self) -> Self {
        match self {
            Dir::Lighter => Dir::Darker,
            Dir::Darker => Dir::Lighter,
        }
    }
}

pub const WHITE: Rgb = Rgb::new(255.0, 255.0, 255.0);
pub const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);

fn byte(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

impl Rgb {
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    pub const fn from_u32(value: u32) -> Self {
        Self::new(
            ((value >> 16) & 0xff) as f64,
            ((value >> 8) & 0xff) as f64,
            (value & 0xff) as f64,
        )
    }

    /// `#rrggbb`, lower case.
    pub fn hex(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            byte(self.r),
            byte(self.g),
            byte(self.b)
        )
    }

    /// The three channels as whole numbers, for `rgb(r g b / a)`.
    pub fn bytes(self) -> (u8, u8, u8) {
        (byte(self.r), byte(self.g), byte(self.b))
    }

    /// WCAG relative luminance, 0 (black) to 1 (white). The channels are rounded first, so the
    /// number is the one of the colour that `hex` writes.
    pub fn luminance(self) -> f64 {
        let channel = |v: f64| {
            let s = byte(v) as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// `self` moved `t` of the way to `other` (0 is self, 1 is other).
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
        )
    }

    fn to_hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (self.r / 255.0, self.g / 255.0, self.b / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d < 1e-9 {
            return (0.0, 0.0, l);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if (max - r).abs() < 1e-9 {
            ((g - b) / d).rem_euclid(6.0)
        } else if (max - g).abs() < 1e-9 {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h * 60.0, s, l)
    }

    fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Rgb::new((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
    }

    /// The same hue and saturation with the lightness (0 to 1) replaced.
    pub fn with_lightness(self, lightness: f64) -> Rgb {
        let (h, s, _) = self.to_hsl();
        Rgb::from_hsl(h, s, lightness.clamp(0.0, 1.0))
    }

    pub fn lightness(self) -> f64 {
        self.to_hsl().2
    }
}

impl Rgba {
    /// This colour laid over an opaque `base`.
    pub fn over(self, base: Rgb) -> Rgb {
        base.mix(self.rgb, self.alpha)
    }
}

/// Reads `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. Anything else is `None`.
pub fn parse(text: &str) -> Option<Rgba> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let single = |i: usize| {
        u8::from_str_radix(&digits[i..i + 1], 16)
            .ok()
            .map(|v| v * 17)
    };
    let (r, g, b, a) = match digits.len() {
        3 => (single(0)?, single(1)?, single(2)?, 255),
        4 => (single(0)?, single(1)?, single(2)?, single(3)?),
        6 => (pair(0)?, pair(2)?, pair(4)?, 255),
        8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
        _ => return None,
    };
    Some(Rgba {
        rgb: Rgb::new(r as f64, g as f64, b as f64),
        alpha: a as f64 / 255.0,
    })
}

/// WCAG contrast ratio, 1 to 21, whichever colour is lighter.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The lowest contrast of `colour` against any of `backgrounds`.
pub fn worst_contrast(colour: Rgb, backgrounds: &[Rgb]) -> f64 {
    backgrounds
        .iter()
        .map(|bg| contrast(colour, *bg))
        .fold(f64::INFINITY, f64::min)
}

/// Moves the lightness of `colour` (hue and saturation stay) until it has at least `min`
/// contrast against every colour of `backgrounds`. It tries `first` and then the other way.
/// A colour that already passes is returned unchanged. When no lightness is enough, the best
/// extreme (pure white or black) comes back.
pub fn nudge(colour: Rgb, backgrounds: &[Rgb], min: f64, first: Dir) -> Rgb {
    if worst_contrast(colour, backgrounds) >= min {
        return colour;
    }
    let start = colour.lightness();
    for dir in [first, first.opposite()] {
        let mut step = 0;
        loop {
            step += 1;
            let delta = 0.004 * step as f64;
            let l = match dir {
                Dir::Lighter => start + delta,
                Dir::Darker => start - delta,
            };
            if !(0.0..=1.0).contains(&l) {
                break;
            }
            let candidate = colour.with_lightness(l);
            if worst_contrast(candidate, backgrounds) >= min {
                return candidate;
            }
        }
    }
    if worst_contrast(WHITE, backgrounds) >= worst_contrast(BLACK, backgrounds) {
        WHITE
    } else {
        BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_hex_form() {
        assert_eq!(parse("#ff0080").unwrap().rgb, Rgb::new(255.0, 0.0, 128.0));
        assert_eq!(parse("#f08").unwrap().rgb, Rgb::new(255.0, 0.0, 136.0));
        let see_through = parse("#11223380").unwrap();
        assert!((see_through.alpha - 128.0 / 255.0).abs() < 1e-9);
        let short = parse("#1238").unwrap();
        assert!((short.alpha - 136.0 / 255.0).abs() < 1e-9);
        assert_eq!(parse("  #FFFFFF ").unwrap().rgb, WHITE);
    }

    #[test]
    fn refuses_what_is_not_a_hex_colour() {
        for text in [
            "",
            "red",
            "#12",
            "#12345",
            "#gg0000",
            "ff0000",
            "#abc\u{e9}",
        ] {
            assert!(parse(text).is_none(), "{text}");
        }
    }

    #[test]
    fn contrast_of_black_and_white_is_twenty_one() {
        assert!((contrast(BLACK, WHITE) - 21.0).abs() < 1e-6);
        assert!((contrast(WHITE, WHITE) - 1.0).abs() < 1e-9);
        assert!((contrast(WHITE, BLACK) - contrast(BLACK, WHITE)).abs() < 1e-9);
    }

    #[test]
    fn a_see_through_colour_is_laid_over_its_base() {
        let half = Rgba {
            rgb: WHITE,
            alpha: 0.5,
        };
        assert_eq!(half.over(BLACK).hex(), "#808080");
    }

    #[test]
    fn hex_round_trips_through_hsl() {
        for text in ["#c62f52", "#11b7d4", "#808080", "#000000", "#ffffff"] {
            let colour = parse(text).unwrap().rgb;
            let (h, s, l) = colour.to_hsl();
            assert_eq!(Rgb::from_hsl(h, s, l).hex(), text);
        }
    }

    #[test]
    fn nudge_leaves_a_passing_colour_alone() {
        let text = parse("#e0e0e0").unwrap().rgb;
        let bg = parse("#101010").unwrap().rgb;
        assert_eq!(nudge(text, &[bg], 4.5, Dir::Lighter), text);
    }

    #[test]
    fn nudge_lightens_a_dull_text_until_it_reads() {
        let text = parse("#40454c").unwrap().rgb;
        let bg = parse("#111418").unwrap().rgb;
        let fixed = nudge(text, &[bg], 4.5, Dir::Lighter);
        assert!(contrast(fixed, bg) >= 4.5);
        assert!(fixed.luminance() > text.luminance());
        // The hue stays close: it is the same colour, only lighter.
        assert!((fixed.to_hsl().0 - text.to_hsl().0).abs() < 8.0);
    }

    #[test]
    fn nudge_goes_the_other_way_when_the_first_way_is_a_dead_end() {
        // Mid grey text on a white background cannot get lighter and pass; it has to darken.
        let text = parse("#b0b0b0").unwrap().rgb;
        let fixed = nudge(text, &[WHITE], 4.5, Dir::Lighter);
        assert!(contrast(fixed, WHITE) >= 4.5);
        assert!(fixed.luminance() < text.luminance());
    }

    #[test]
    fn nudge_meets_the_worst_background_of_several() {
        let backgrounds = [
            parse("#101214").unwrap().rgb,
            parse("#2a2f36").unwrap().rgb,
            parse("#454c57").unwrap().rgb,
        ];
        let fixed = nudge(
            parse("#707070").unwrap().rgb,
            &backgrounds,
            4.5,
            Dir::Lighter,
        );
        assert!(worst_contrast(fixed, &backgrounds) >= 4.5);
    }

    #[test]
    fn nudge_falls_back_to_the_best_extreme() {
        // Nothing can reach 30:1, so the better of white and black comes back.
        let bg = parse("#808080").unwrap().rgb;
        let fixed = nudge(parse("#777777").unwrap().rgb, &[bg], 30.0, Dir::Lighter);
        assert!(fixed == WHITE || fixed == BLACK);
    }
}
