//! Terminal output: colors and banner.

use std::io::IsTerminal;

const BANNER: &str = include_str!("../../../assets/cli-banner.txt");
/// Column where the banner's `>` starts; the `<` is everything before it.
const CHEVRON_SPLIT: usize = 5;
/// Column where the banner's `markoo` starts.
const NAME_START: usize = 9;
const PINK: (u8, u8, u8) = (0xF0, 0x41, 0x8E);
const GREEN: (u8, u8, u8) = (0x22, 0xA5, 0x5A);

/// Applies colors, or nothing when they are not wanted.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    enabled: bool,
}

impl Style {
    /// Colors only on a terminal, unless `NO_COLOR` is set.
    pub fn detect() -> Self {
        Self {
            enabled: std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
        }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    pub fn green(self, text: &str) -> String {
        self.paint("32", text)
    }

    pub fn red(self, text: &str) -> String {
        self.paint("31", text)
    }

    pub fn yellow(self, text: &str) -> String {
        self.paint("33", text)
    }

    pub fn cyan(self, text: &str) -> String {
        self.paint("36", text)
    }

    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    fn rgb(self, (r, g, b): (u8, u8, u8), text: &str) -> String {
        self.paint(&format!("38;2;{r};{g};{b}"), text)
    }

    /// The banner, with `<` in pink and `>` in green when colors are on.
    pub fn banner(self) -> String {
        let mut out = String::new();
        for line in BANNER.lines() {
            let chars: Vec<char> = line.chars().collect();
            let part = |from: usize, to: usize| -> String {
                chars
                    .iter()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            };
            out.push_str(&self.rgb(PINK, &part(0, CHEVRON_SPLIT)));
            out.push_str(&self.rgb(GREEN, &part(CHEVRON_SPLIT, NAME_START)));
            out.push_str(&part(NAME_START, chars.len()));
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_banner_is_the_asset() {
        let style = Style { enabled: false };
        assert_eq!(style.banner().trim_end(), BANNER.trim_end());
    }

    #[test]
    fn colored_banner_keeps_the_text() {
        let style = Style { enabled: true };
        let stripped = style
            .banner()
            .replace("\x1b[38;2;240;65;142m", "")
            .replace("\x1b[38;2;34;165;90m", "")
            .replace("\x1b[0m", "");
        assert_eq!(stripped.trim_end(), BANNER.trim_end());
    }
}
