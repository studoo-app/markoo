//! Knowledge base adapters and Markdown dialects.
//!
//! Every side of a pair speaks a [`Dialect`]. The engine compares and merges
//! the **canonical form** (GitHub Flavored Markdown with relative links) and
//! only ever talks to these traits, never to a specific tool.
//!
//! M1 ships the identity adapter (`markdown`) and an identity `gfm` dialect.
//! Detection, layout and link resolution (see the knowledge bases design doc)
//! arrive with the Obsidian adapter in M3.

use serde::{Deserialize, Serialize};

/// Errors raised while converting between a dialect and the canonical form.
#[derive(Debug, thiserror::Error)]
#[error("{dialect}: {message}")]
pub struct DialectError {
    /// Dialect that failed.
    pub dialect: &'static str,
    /// Description of the failure.
    pub message: String,
}

/// Conversion between a Markdown dialect and the canonical form.
///
/// Implementations must round-trip losslessly in both directions:
/// `from_canonical(to_canonical(y)) == y` for any file `y` of the dialect.
pub trait Dialect {
    /// Identifier of the dialect (e.g. `"gfm"`, `"obsidian"`).
    fn id(&self) -> &'static str;
    /// Converts a file of this dialect to canonical form.
    fn to_canonical(&self, text: &str) -> Result<String, DialectError>;
    /// Converts canonical text to this dialect.
    // The name mirrors `to_canonical` and the design docs; the method needs
    // `&self` because dialects carry options.
    #[expect(clippy::wrong_self_convention)]
    fn from_canonical(&self, text: &str) -> Result<String, DialectError>;
}

/// A knowledge base type.
pub trait KbAdapter: Dialect {
    /// Glob patterns the adapter always ignores (tool settings, trash…),
    /// on top of the pair's own filters.
    fn excluded(&self) -> &'static [&'static str];
}

/// Supported values of the `kb_type` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KbType {
    /// A plain Markdown folder, without any conversion.
    #[default]
    Markdown,
}

impl KbType {
    /// Returns the adapter for this KB type.
    pub fn adapter(self) -> &'static dyn KbAdapter {
        match self {
            KbType::Markdown => &MarkdownAdapter,
        }
    }
}

/// Plain Markdown folder: the KB already is in canonical form.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownAdapter;

impl Dialect for MarkdownAdapter {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn to_canonical(&self, text: &str) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }

    fn from_canonical(&self, text: &str) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }
}

impl KbAdapter for MarkdownAdapter {
    fn excluded(&self) -> &'static [&'static str] {
        &[]
    }
}

/// GitHub Flavored Markdown, the repo side's dialect. It is the canonical
/// form itself, so the conversion is the identity.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gfm;

impl Dialect for Gfm {
    fn id(&self) -> &'static str {
        "gfm"
    }

    fn to_canonical(&self, text: &str) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }

    fn from_canonical(&self, text: &str) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Title\n\nSee [ADR](./adr/001.md).\n";

    #[test]
    fn markdown_adapter_round_trips() {
        let canonical = MarkdownAdapter.to_canonical(SAMPLE).unwrap();
        assert_eq!(MarkdownAdapter.from_canonical(&canonical).unwrap(), SAMPLE);
    }

    #[test]
    fn gfm_round_trips() {
        let canonical = Gfm.to_canonical(SAMPLE).unwrap();
        assert_eq!(Gfm.from_canonical(&canonical).unwrap(), SAMPLE);
    }

    #[test]
    fn default_kb_type_is_markdown() {
        assert_eq!(KbType::default().adapter().id(), "markdown");
    }
}
