//! The Obsidian adapter.
//!
//! The conversion follows one rule. Each Obsidian construct has a canonical
//! rendering `R`, and each `R` has a *default* Obsidian form. When the original
//! is that default form, the canonical text is `R` alone, so the repo stays
//! clean. Otherwise the original `W` is kept next to it, as `R<!--markoo:W-->`.
//! On the way back, a comment is honored only if the text before it still is
//! the rendering of `W`; if someone edited it on the repo side, the edit wins
//! and the comment is dropped. Everything else is left as is, byte for byte.

use std::ops::Range;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde_json::{Value, json};

use super::{ConvertCtx, Dialect, DialectError, KbAdapter, KbOptions, NameIndex, read_optional};
use crate::fs::Fs;

const MARKER_OPEN: &str = "<!--markoo:";
const MARKER_CLOSE: &str = "-->";
const CALLOUT_MARKER: &str = "callout ";
const FRONTMATTER_MARKER: &str = "frontmatter ";

/// (Obsidian type, GitHub alert), the first type of each alert being its default.
const CALLOUTS: &[(&str, &str)] = &[
    ("note", "NOTE"),
    ("info", "NOTE"),
    ("tip", "TIP"),
    ("hint", "TIP"),
    ("important", "IMPORTANT"),
    ("warning", "WARNING"),
    ("attention", "WARNING"),
    ("caution", "CAUTION"),
    ("danger", "CAUTION"),
    ("error", "CAUTION"),
];

/// Finds the vault containing `folder`: the closest parent with a `.obsidian` folder.
pub fn find_vault_root(fs: &impl Fs, folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|dir| fs.has_folder(&dir.join(".obsidian")))
        .map(Path::to_path_buf)
}

/// The vaults Obsidian knows about, read from its `obsidian.json`.
pub fn known_vaults() -> Vec<PathBuf> {
    let Some(dirs) = BaseDirs::new() else {
        return Vec::new();
    };
    let settings = dirs.config_dir().join("obsidian").join("obsidian.json");
    let Ok(text) = std::fs::read_to_string(settings) else {
        return Vec::new();
    };
    parse_known_vaults(&text)
}

fn parse_known_vaults(text: &str) -> Vec<PathBuf> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut vaults: Vec<PathBuf> = value
        .get("vaults")
        .and_then(Value::as_object)
        .map(|vaults| {
            vaults
                .values()
                .filter_map(|vault| vault.get("path")?.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default();
    vaults.sort();
    vaults
}

/// Settings read from `.obsidian/app.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultSettings {
    /// `useMarkdownLinks`: new links are Markdown links rather than wikilinks.
    pub use_markdown_links: bool,
    /// `attachmentFolderPath`: where new attachments go.
    pub attachment_folder: Option<String>,
}

impl VaultSettings {
    /// Reads the settings of a vault; missing or invalid settings mean defaults.
    pub fn read(fs: &impl Fs, vault_root: &Path) -> Self {
        read_optional(fs, &vault_root.join(".obsidian").join("app.json"))
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .map(|value| Self {
                use_markdown_links: value
                    .get("useMarkdownLinks")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                attachment_folder: value
                    .get("attachmentFolderPath")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
            .unwrap_or_default()
    }
}

/// The Obsidian adapter of one pair.
#[derive(Debug, Clone, Default)]
pub struct ObsidianAdapter {
    /// Conversion options.
    pub options: KbOptions,
    /// Vault settings.
    pub settings: VaultSettings,
}

impl Dialect for ObsidianAdapter {
    fn id(&self) -> &'static str {
        "obsidian"
    }

    fn to_canonical(&self, text: &str, ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        let (front, body) = self.frontmatter_to_canonical(text);
        Ok(front + &self.body_to_canonical(&body, ctx))
    }

    fn from_canonical(&self, text: &str, ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        let (front, body) = frontmatter_from_canonical(text);
        Ok(front + &self.body_from_canonical(&body, ctx))
    }
}

impl KbAdapter for ObsidianAdapter {
    fn excluded(&self) -> &'static [&'static str] {
        &["**/.obsidian/**", "**/.trash/**"]
    }

    fn excluded_folders(&self) -> &'static [&'static str] {
        &[".obsidian", ".trash"]
    }

    fn referenced_assets(&self, canonical: &str, ctx: &ConvertCtx<'_>) -> Vec<String> {
        let mut names = Vec::new();
        for link in inline_links(canonical) {
            if let Some(name) = asset_name_of_href(split_fragment(&link.href).0, ctx) {
                names.push(name);
            }
        }
        for found in markers(canonical, &code_ranges(canonical)) {
            let payload = found.payload;
            if let Some(wiki) = Wikilink::parse(payload)
                && wiki.embed
                && ctx.index.find_attachment(wiki.target, ctx).is_some()
            {
                names.extend(asset_basename(wiki.target).map(str::to_owned));
            }
        }
        names.sort();
        names.dedup();
        names
    }
}

impl ObsidianAdapter {
    fn body_to_canonical(&self, body: &str, ctx: &ConvertCtx<'_>) -> String {
        let protected = protected_ranges(body);
        let mut edits = Vec::new();
        let links = inline_links(body);
        if self.options.convert_links {
            for link in &links {
                if overlaps(&link.range, &protected) || followed_by_marker(body, &link.range) {
                    continue;
                }
                let original = &body[link.range.clone()];
                let rendered = self.render_markdown_link(link, ctx);
                if (rendered != original
                    || self
                        .default_form(&rendered, ctx)
                        .is_some_and(|form| form != rendered))
                    && let Some(marked) = marked(&rendered, original)
                {
                    edits.push((link.range.clone(), marked));
                }
            }
            for found in wikilinks(body) {
                if overlaps(&found.range, &protected)
                    || links
                        .iter()
                        .any(|link| ranges_touch(&link.range, &found.range))
                {
                    continue;
                }
                let original = &body[found.range.clone()];
                let Some(wiki) = Wikilink::parse(original) else {
                    continue;
                };
                let Some(rendered) = self.render_wikilink(&wiki, ctx) else {
                    ctx.index
                        .warn(ctx.path, format!("link target not found: {original}"));
                    continue;
                };
                let replacement = if self.default_form(&rendered, ctx).as_deref() == Some(original)
                {
                    Some(rendered)
                } else {
                    marked(&rendered, original)
                };
                if let Some(replacement) = replacement {
                    edits.push((found.range.clone(), replacement));
                }
            }
        }
        if self.options.convert_callouts {
            edits.extend(callouts_to_canonical(body, &protected));
        }
        apply_edits(body, edits)
    }

    fn body_from_canonical(&self, body: &str, ctx: &ConvertCtx<'_>) -> String {
        let code = code_ranges(body);
        let mut edits = Vec::new();
        let mut covered = Vec::new();
        for found in markers(body, &code) {
            let payload = found.payload;
            if payload.starts_with(CALLOUT_MARKER) || payload.starts_with(FRONTMATTER_MARKER) {
                continue;
            }
            let rendered = self.render_original(payload, ctx);
            let start = found.range.start;
            match rendered {
                Some(rendered) if !rendered.is_empty() && body[..start].ends_with(&rendered) => {
                    let whole = start - rendered.len()..found.range.end;
                    edits.push((whole.clone(), payload.to_owned()));
                    covered.push(whole);
                }
                // The text before the comment was edited: the edit wins.
                _ => edits.push((found.range.clone(), String::new())),
            }
        }
        if self.options.convert_links {
            let protected = protected_ranges(body);
            for link in inline_links(body) {
                if overlaps(&link.range, &protected) || overlaps(&link.range, &covered) {
                    continue;
                }
                // A comment after this link was stale, or it would be covered.
                let original = &body[link.range.clone()];
                if let Some(form) = self.default_form(original, ctx) {
                    edits.push((link.range.clone(), form));
                }
            }
        }
        if self.options.convert_callouts {
            edits.extend(callouts_from_canonical(body, &code));
        }
        apply_edits(body, edits)
    }

    /// The canonical rendering of a piece of Obsidian text kept in a comment.
    fn render_original(&self, original: &str, ctx: &ConvertCtx<'_>) -> Option<String> {
        if let Some(wiki) = Wikilink::parse(original) {
            return self.render_wikilink(&wiki, ctx);
        }
        let links = inline_links(original);
        match links.as_slice() {
            [link] if link.range == (0..original.len()) => {
                Some(self.render_markdown_link(link, ctx))
            }
            _ => None,
        }
    }

    /// Canonical form of a Markdown link written in the vault. Links to an
    /// attachment go to the assets folder, and links written from the vault
    /// root, which only Obsidian understands, become relative.
    fn render_markdown_link(&self, link: &InlineLink, ctx: &ConvertCtx<'_>) -> String {
        let (path, fragment) = split_fragment(&link.href);
        if asset_name_of_href(path, ctx).is_some() {
            return link.source.clone();
        }
        let Some(found) = ctx.index.vault_path_of_href(path, ctx) else {
            return link.source.clone();
        };
        if ctx.index.attachments.contains(&found.path) {
            let Some(name) = asset_basename(&found.path)
                .filter(|name| ctx.index.attachment_name_is_unique(name))
            else {
                return link.source.clone();
            };
            let mut href =
                encode_href(&relative_to(ctx.path.as_str(), &ctx.index.asset_path(name)));
            if let Some(fragment) = fragment {
                href.push('#');
                href.push_str(fragment);
            }
            let bang = if link.is_image { "!" } else { "" };
            return format!("{bang}[{}]({href})", link.text);
        }
        if !found.from_root {
            return link.source.clone();
        }
        let text = if link.text.is_empty() {
            stem(&found.path)
        } else {
            &link.text
        };
        let Some(inside) = ctx.index.tracked_relative(&found.path) else {
            // Outside the pair: GitHub could not follow the link.
            return text.to_owned();
        };
        let mut href = encode_href(&relative_to(ctx.path.as_str(), &inside));
        if let Some(fragment) = fragment.filter(|fragment| !fragment.starts_with('^')) {
            href.push('#');
            href.push_str(&github_slug(&decode_href(fragment)));
        }
        // An embedded note cannot display as an image: it stays a plain link.
        format!("[{text}]({href})")
    }

    fn render_wikilink(&self, wiki: &Wikilink<'_>, ctx: &ConvertCtx<'_>) -> Option<String> {
        if wiki.embed
            && let Some(path) = ctx.index.find_attachment(wiki.target, ctx)
        {
            let name = asset_basename(&path)?;
            if !ctx.index.attachment_name_is_unique(name) {
                return None;
            }
            let href = encode_href(&relative_to(ctx.path.as_str(), &ctx.index.asset_path(name)));
            return Some(match wiki.alias {
                Some(size) if !size.is_empty() && size.chars().all(|c| c.is_ascii_digit()) => {
                    format!("<img src=\"{href}\" width=\"{size}\">")
                }
                Some(alt) => format!("![{alt}]({href})"),
                None => format!("![{}]({href})", stem(name)),
            });
        }
        let note = ctx.index.find_note(wiki.target, ctx)?;
        let text = wiki
            .alias
            .map_or_else(|| stem(&note).to_owned(), str::to_owned);
        let Some(inside) = ctx.index.tracked_relative(&note) else {
            // Outside the pair: GitHub could not follow the link.
            return Some(text);
        };
        let mut href = encode_href(&relative_to(ctx.path.as_str(), &inside));
        if let Some(anchor) = wiki.anchor.filter(|anchor| !anchor.starts_with('^')) {
            href.push('#');
            href.push_str(&github_slug(anchor));
        }
        Some(format!("[{text}]({href})"))
    }

    /// The form a clean canonical link takes in the vault, if it has one.
    /// It must convert back to exactly the same link, or it stays as it is.
    fn default_form(&self, canonical: &str, ctx: &ConvertCtx<'_>) -> Option<String> {
        if !self.options.convert_links || self.settings.use_markdown_links {
            return None;
        }
        let links = inline_links(canonical);
        let [link] = links.as_slice() else {
            return None;
        };
        if link.range != (0..canonical.len()) || !link.is_simple_text() {
            return None;
        }
        let form = if link.is_image {
            let name = asset_name_of_href(&link.href, ctx)?;
            if link.text == stem(&name) {
                format!("![[{name}]]")
            } else {
                format!("![[{name}|{}]]", link.text)
            }
        } else {
            let inside = resolve_relative(ctx.path.as_str(), &decode_href(&link.href))?;
            if !ctx.index.tracked.contains(&inside) {
                return None;
            }
            let vault_path = ctx.index.vault_relative(&inside);
            let name = stem(&vault_path);
            let target = if ctx.index.note_name_is_unique(name) {
                name.to_owned()
            } else {
                vault_path.trim_end_matches(".md").to_owned()
            };
            if link.text == name {
                format!("[[{target}]]")
            } else {
                format!("[[{target}|{}]]", link.text)
            }
        };
        let wiki = Wikilink::parse(&form)?;
        (self.render_wikilink(&wiki, ctx).as_deref() == Some(canonical)).then_some(form)
    }

    fn frontmatter_to_canonical(&self, text: &str) -> (String, String) {
        let Some(front) = Frontmatter::parse(text) else {
            return (String::new(), text.to_owned());
        };
        let drop = |key: &str| {
            self.options.frontmatter_drop_all
                || self.options.frontmatter_drop.iter().any(|k| k == key)
        };
        let mut kept = String::new();
        let mut dropped = Vec::new();
        for (index, chunk) in front.chunks.iter().enumerate() {
            if chunk.key.as_deref().is_some_and(drop) {
                dropped.push(json!([index, chunk.raw]));
            } else {
                kept.push_str(chunk.raw);
            }
        }
        if dropped.is_empty() {
            return (String::new(), text.to_owned());
        }
        let payload = Value::Array(dropped).to_string().replace('>', "\\u003e");
        let fence = if kept.is_empty() {
            String::new()
        } else {
            format!("---\n{kept}---\n")
        };
        let head = format!("{fence}{MARKER_OPEN}{FRONTMATTER_MARKER}{payload}{MARKER_CLOSE}\n");
        (head, front.body.to_owned())
    }
}

/// A frontmatter split into top-level keys, preserving their exact text.
struct Frontmatter<'a> {
    chunks: Vec<Chunk<'a>>,
    body: &'a str,
}

struct Chunk<'a> {
    key: Option<String>,
    raw: &'a str,
}

impl<'a> Frontmatter<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let rest = text.strip_prefix("---\n")?;
        let (inner, body) = if let Some(body) = rest.strip_prefix("---\n") {
            ("", body)
        } else {
            let end = rest.find("\n---\n")?;
            (&rest[..=end], &rest[end + 5..])
        };
        let mut chunks: Vec<Chunk<'a>> = Vec::new();
        let mut start = 0;
        let mut offset = 0;
        for line in inner.split_inclusive('\n') {
            if offset > start && top_level_key(line).is_some() {
                chunks.push(Chunk::of(&inner[start..offset]));
                start = offset;
            }
            offset += line.len();
        }
        if start < inner.len() {
            chunks.push(Chunk::of(&inner[start..]));
        }
        Some(Self { chunks, body })
    }
}

impl<'a> Chunk<'a> {
    fn of(raw: &'a str) -> Self {
        Self {
            key: raw.lines().find_map(top_level_key).map(str::to_owned),
            raw,
        }
    }
}

fn top_level_key(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '-', '#', '\n']) {
        return None;
    }
    line.split_once(':').map(|(key, _)| key.trim())
}

/// Puts back the frontmatter keys kept in a comment. Returns the restored
/// frontmatter and the rest of the text.
fn frontmatter_from_canonical(text: &str) -> (String, String) {
    let (front, body) = match Frontmatter::parse(text) {
        Some(front) => {
            let chunks: Vec<&str> = front.chunks.iter().map(|chunk| chunk.raw).collect();
            (Some(chunks), front.body)
        }
        None => (None, text),
    };
    let dropped = body
        .strip_prefix(MARKER_OPEN)
        .and_then(|rest| rest.strip_prefix(FRONTMATTER_MARKER))
        .and_then(|rest| rest.split_once(&format!("{MARKER_CLOSE}\n")))
        .and_then(|(payload, rest)| Some((parse_dropped(payload)?, rest)));
    let Some((dropped, rest)) = dropped else {
        return (String::new(), text.to_owned());
    };
    let mut chunks: Vec<&str> = front.unwrap_or_default();
    for (index, raw) in &dropped {
        chunks.insert((*index).min(chunks.len()), raw);
    }
    (format!("---\n{}---\n", chunks.concat()), rest.to_owned())
}

fn parse_dropped(payload: &str) -> Option<Vec<(usize, String)>> {
    serde_json::from_str::<Value>(payload)
        .ok()?
        .as_array()?
        .iter()
        .map(|item| {
            let index = usize::try_from(item.get(0)?.as_u64()?).ok()?;
            Some((index, item.get(1)?.as_str()?.to_owned()))
        })
        .collect()
}

fn callouts_to_canonical(body: &str, protected: &[Range<usize>]) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    for (range, kind) in callout_lines(body) {
        if overlaps(&range, protected) {
            continue;
        }
        let Some((_, alert)) = CALLOUTS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(kind))
        else {
            continue;
        };
        let default = default_callout(alert);
        let line = format!("> [!{alert}]");
        let text = if kind == default {
            line
        } else {
            format!("{MARKER_OPEN}{CALLOUT_MARKER}{kind}{MARKER_CLOSE}\n{line}")
        };
        edits.push((range, text));
    }
    edits
}

fn callouts_from_canonical(body: &str, code: &[Range<usize>]) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    for (range, kind) in callout_lines(body) {
        if overlaps(&range, code) || kind != kind.to_uppercase() {
            continue;
        }
        if !CALLOUTS.iter().any(|(_, alert)| *alert == kind) {
            continue;
        }
        // The original type, on the line above, if it still matches the alert.
        let before = body[..range.start].strip_suffix('\n').unwrap_or_default();
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let original = before[line_start..]
            .strip_prefix(MARKER_OPEN)
            .and_then(|rest| rest.strip_prefix(CALLOUT_MARKER))
            .and_then(|rest| rest.strip_suffix(MARKER_CLOSE));
        match original {
            Some(original) => {
                let matches = CALLOUTS
                    .iter()
                    .any(|(name, alert)| name.eq_ignore_ascii_case(original) && *alert == kind);
                let shown = if matches {
                    original
                } else {
                    default_callout(kind)
                };
                edits.push((line_start..range.end, format!("> [!{shown}]")));
            }
            None => edits.push((range, format!("> [!{}]", default_callout(kind)))),
        }
    }
    edits
}

fn default_callout(alert: &str) -> &'static str {
    CALLOUTS
        .iter()
        .find(|(_, candidate)| *candidate == alert)
        .map_or("note", |(name, _)| *name)
}

/// Lines that are exactly `> [!kind]`, with the range of the line (no newline).
fn callout_lines(body: &str) -> Vec<(Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        let kind = content
            .strip_prefix("> [!")
            .and_then(|rest| rest.strip_suffix(']'))
            .filter(|kind| !kind.is_empty() && kind.chars().all(|c| c.is_ascii_alphabetic()));
        if let Some(kind) = kind {
            found.push((offset..offset + content.len(), kind));
        }
        offset += line.len();
    }
    found
}

/// A parsed `[[target#anchor|alias]]` or `![[…]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wikilink<'a> {
    embed: bool,
    target: &'a str,
    anchor: Option<&'a str>,
    alias: Option<&'a str>,
}

impl<'a> Wikilink<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let (embed, rest) = match text.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let inner = rest.strip_prefix("[[")?.strip_suffix("]]")?;
        if inner.is_empty() || inner.contains(['[', ']', '\n']) {
            return None;
        }
        // In tables, the separator has to be written `\|`.
        let (link, alias) = match inner.split_once('|') {
            Some((link, alias)) => (link.strip_suffix('\\').unwrap_or(link), Some(alias)),
            None => (inner, None),
        };
        let (target, anchor) = match link.split_once('#') {
            Some((target, anchor)) => (target, Some(anchor)),
            None => (link, None),
        };
        if target.is_empty() {
            return None;
        }
        Some(Self {
            embed,
            target,
            anchor,
            alias,
        })
    }
}

struct FoundWikilink {
    range: Range<usize>,
}

fn wikilinks(body: &str) -> Vec<FoundWikilink> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = body[from..].find("[[").map(|i| i + from) {
        let Some(close) = body[open..].find("]]").map(|i| i + open + 2) else {
            break;
        };
        let escaped = open > 0 && body.as_bytes()[open - 1] == b'\\';
        let embed = open > 0 && body.as_bytes()[open - 1] == b'!';
        let start = if embed { open - 1 } else { open };
        if !escaped && Wikilink::parse(&body[start..close]).is_some() {
            found.push(FoundWikilink {
                range: start..close,
            });
            from = close;
        } else {
            from = open + 2;
        }
    }
    found
}

/// A `[text](href)` or `![text](href)` link, as written.
#[derive(Debug, Clone)]
struct InlineLink {
    range: Range<usize>,
    is_image: bool,
    text: String,
    href: String,
    source: String,
}

impl InlineLink {
    /// Whether the text can go into a wikilink alias.
    fn is_simple_text(&self) -> bool {
        !self.text.contains(['[', ']', '|', '\n'])
    }
}

fn inline_links(text: &str) -> Vec<InlineLink> {
    let parser = Parser::new_ext(text, parser_options());
    let mut links = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        let (is_image, link_type, title) = match &event {
            Event::Start(Tag::Link {
                link_type, title, ..
            }) => (false, *link_type, title),
            Event::Start(Tag::Image {
                link_type, title, ..
            }) => (true, *link_type, title),
            _ => continue,
        };
        if link_type != LinkType::Inline || !title.is_empty() {
            continue;
        }
        let source = &text[range.clone()];
        let inner = if is_image {
            source.strip_prefix("![")
        } else {
            source.strip_prefix('[')
        };
        let Some((label, href)) = inner
            .and_then(|inner| inner.strip_suffix(')'))
            .and_then(|inner| inner.rsplit_once("]("))
        else {
            continue;
        };
        if href.is_empty() || href.contains([' ', '<', '>', '"']) {
            continue;
        }
        links.push(InlineLink {
            range,
            is_image,
            text: label.to_owned(),
            href: href.to_owned(),
            source: source.to_owned(),
        });
    }
    links
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

/// Ranges of code: their content is never touched.
fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut parser = Parser::new_ext(text, parser_options()).into_offset_iter();
    while let Some((event, range)) = parser.next() {
        match event {
            Event::Code(_) => ranges.push(range),
            Event::Start(Tag::CodeBlock(_)) => {
                ranges.push(range);
                for (event, _) in parser.by_ref() {
                    if matches!(event, Event::End(pulldown_cmark::TagEnd::CodeBlock)) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    ranges
}

/// Code, raw HTML and Obsidian `%%` comments: text to leave alone.
fn protected_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = code_ranges(text);
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        if matches!(event, Event::Html(_) | Event::InlineHtml(_)) {
            ranges.push(range);
        }
    }
    let mut from = 0;
    while let Some(open) = text[from..].find("%%").map(|i| i + from) {
        let Some(close) = text[open + 2..].find("%%").map(|i| i + open + 4) else {
            break;
        };
        ranges.push(open..close);
        from = close;
    }
    ranges
}

fn overlaps(range: &Range<usize>, others: &[Range<usize>]) -> bool {
    others.iter().any(|other| ranges_touch(range, other))
}

fn ranges_touch(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

struct Marker<'a> {
    range: Range<usize>,
    payload: &'a str,
}

fn markers<'a>(text: &'a str, code: &[Range<usize>]) -> Vec<Marker<'a>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find(MARKER_OPEN).map(|i| i + from) {
        let start = open + MARKER_OPEN.len();
        let Some(close) = text[start..].find(MARKER_CLOSE).map(|i| i + start) else {
            break;
        };
        let range = open..close + MARKER_CLOSE.len();
        if !overlaps(&range, code) {
            found.push(Marker {
                payload: &text[start..close],
                range: range.clone(),
            });
        }
        from = range.end;
    }
    found
}

fn followed_by_marker(body: &str, range: &Range<usize>) -> bool {
    body[range.end..].starts_with(MARKER_OPEN)
}

/// `rendered` followed by a comment keeping `original`, unless the comment
/// could not hold it.
fn marked(rendered: &str, original: &str) -> Option<String> {
    (!original.contains(MARKER_CLOSE))
        .then(|| format!("{rendered}{MARKER_OPEN}{original}{MARKER_CLOSE}"))
}

fn apply_edits(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(text.len());
    let mut position = 0;
    for (range, replacement) in edits {
        if range.start < position {
            continue;
        }
        out.push_str(&text[position..range.start]);
        out.push_str(&replacement);
        position = range.end;
    }
    out.push_str(&text[position..]);
    out
}

impl NameIndex {
    /// Finds a note like Obsidian does: exact path, then unique name, then the
    /// one closest to the current file (with a warning).
    fn find_note(&self, target: &str, ctx: &ConvertCtx<'_>) -> Option<String> {
        let wanted = target.strip_suffix(".md").unwrap_or(target).to_lowercase();
        let exact = format!("{wanted}.md");
        let suffix = format!("/{wanted}.md");
        let candidates: Vec<&String> = self
            .notes
            .iter()
            .filter(|note| {
                let note = note.to_lowercase();
                note == exact || note.ends_with(&suffix)
            })
            .collect();
        if let Some(found) = candidates.iter().find(|note| note.to_lowercase() == exact) {
            return Some((*found).clone());
        }
        match candidates.as_slice() {
            [] => None,
            [only] => Some((*only).clone()),
            several => {
                let from = self.vault_relative(ctx.path.as_str());
                let closest = several
                    .iter()
                    .max_by_key(|note| (shared_folders(note, &from), std::cmp::Reverse(note.len())))
                    .map(|note| (*note).clone());
                self.warn(ctx.path, format!("ambiguous link target: {target}"));
                closest
            }
        }
    }

    /// Finds an attachment by name or path.
    fn find_attachment(&self, target: &str, _ctx: &ConvertCtx<'_>) -> Option<String> {
        let wanted = target.to_lowercase();
        let suffix = format!("/{wanted}");
        let found = self
            .attachments
            .iter()
            .find(|file| {
                let file = file.to_lowercase();
                file == wanted || file.ends_with(&suffix)
            })
            .cloned();
        found.or_else(|| {
            self.repo_assets
                .iter()
                .find(|name| name.to_lowercase() == wanted)
                .cloned()
        })
    }

    /// The vault file a link path of the current note points to. A path next
    /// to the note wins, since that is how GitHub reads it; otherwise it is
    /// looked up from the vault root, unless it is explicitly relative.
    fn vault_path_of_href(&self, path: &str, ctx: &ConvertCtx<'_>) -> Option<LinkedFile> {
        let decoded = decode_href(path);
        let exists = |file: &String| self.notes.contains(file) || self.attachments.contains(file);
        // The file may be anywhere in the vault, not only in the pair.
        let from = self.vault_relative(ctx.path.as_str());
        let next_to_note = resolve_relative(&from, &decoded)?;
        if exists(&next_to_note) {
            return Some(LinkedFile {
                path: next_to_note,
                from_root: false,
            });
        }
        if decoded.starts_with("./") || decoded.starts_with("../") {
            return None;
        }
        resolve_relative("", &decoded)
            .filter(exists)
            .map(|path| LinkedFile {
                path,
                from_root: true,
            })
    }

    fn attachment_name_is_unique(&self, name: &str) -> bool {
        self.attachments
            .iter()
            .filter(|file| asset_basename(file) == Some(name))
            .count()
            <= 1
    }

    fn note_name_is_unique(&self, name: &str) -> bool {
        self.notes.iter().filter(|note| stem(note) == name).count() == 1
    }

    /// Path of a note relative to the pair, if it is tracked.
    fn tracked_relative(&self, vault_path: &str) -> Option<String> {
        self.pair_relative(vault_path)
            .filter(|inside| self.tracked.contains(inside))
    }

    /// Path of an attachment in the repo, relative to the pair's root.
    pub(crate) fn asset_path(&self, name: &str) -> String {
        format!("{}/{name}", self.assets_dir)
    }

    /// Where an attachment is in the vault, relative to the vault root.
    pub(crate) fn attachment_in_vault(&self, name: &str) -> Option<String> {
        self.attachments
            .iter()
            .find(|file| asset_basename(file) == Some(name))
            .cloned()
    }
}

/// A file of the vault a link points to.
struct LinkedFile {
    /// Path relative to the vault root.
    path: String,
    /// The link is written from the vault root rather than from the note.
    from_root: bool,
}

/// Splits a link destination into its path and its `#fragment`.
fn split_fragment(href: &str) -> (&str, Option<&str>) {
    match href.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (href, None),
    }
}

/// The attachment name if `href`, from the current note, is in the assets folder.
fn asset_name_of_href(href: &str, ctx: &ConvertCtx<'_>) -> Option<String> {
    let inside = resolve_relative(ctx.path.as_str(), &decode_href(href))?;
    let name = inside
        .strip_prefix(&ctx.index.assets_dir)?
        .strip_prefix('/')?;
    (!name.contains('/') && !name.is_empty()).then(|| name.to_owned())
}

fn asset_basename(path: &str) -> Option<&str> {
    path.rsplit('/').next().filter(|name| !name.is_empty())
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn shared_folders(a: &str, b: &str) -> usize {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

/// `target` as a relative link from the file `from`; both relative to the same root.
fn relative_to(from: &str, target: &str) -> String {
    let from_dirs: Vec<&str> = from.split('/').collect();
    let from_dirs = &from_dirs[..from_dirs.len() - 1];
    let target_parts: Vec<&str> = target.split('/').collect();
    let shared = from_dirs
        .iter()
        .zip(&target_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from_dirs.len() - shared;
    let rest = target_parts[shared..].join("/");
    if ups == 0 {
        format!("./{rest}")
    } else {
        format!("{}{rest}", "../".repeat(ups))
    }
}

/// Resolves a relative link from the file `from`. `None` for URLs, absolute
/// paths, and anything leaving the root.
fn resolve_relative(from: &str, href: &str) -> Option<String> {
    if href.contains("://") || href.starts_with(['/', '#']) || href.starts_with("mailto:") {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in href.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// Encodes what a Markdown link destination cannot contain. Accents stay
/// readable; GitHub handles them.
fn encode_href(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

fn decode_href(href: &str) -> String {
    let bytes = href.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| href.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = hex {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| href.to_owned())
}

/// The anchor GitHub generates for a heading.
fn github_slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
