# Glossary

> 🇫🇷 [Version française](../fr/glossaire.md)

| Term | Definition |
|------|------------|
| **Knowledge base (KB)** | A folder of Markdown notes managed by a tool (e.g. Obsidian) or a plain Markdown folder: one of the two sides of a pair. Its type (`kb_type`) decides which adapter is used. In French, never shortened to "base", which means the merge ancestor. See [Knowledge bases](09-knowledge-bases.md). |
| **Vault** | An Obsidian knowledge base: root folder containing `.obsidian/`. |
| **`kb_type`** | Type of a KB in `markoo.toml`: `markdown` or `obsidian` in v1; `logseq`, `foam`, `dendron` after v1. Auto-detected when missing. |
| **Adapter** | Implementation of the `KbAdapter` trait for a KB type: detection, layout (notes, attachments, exclusions), dialect and name resolution. See [Adapter model](09-knowledge-bases.md#adapter-model). |
| **Dialect** | A side's own Markdown variant (Obsidian wikilinks and callouts, `gfm` on the repo side…), converted to and from the canonical form by `to_canonical` / `from_canonical`. |
| **Repo** | The Git repository side of a pair: a folder in a local clone. |
| **Pair** | A KB folder and a repo folder associated and synced together. |
| **Base** | A file's content at the last successful sync, stored in `state.db`. Serves as the common ancestor for merging; stored in canonical form. |
| **Canonical form** | Standard GitHub Flavored Markdown with relative links, plus invisible `<!--markoo:…-->` comments for what GFM cannot represent. Every adapter converts its dialect to and from this form; comparison, merging and storage of the base are done on it. |
| **Transformation** | A pair of functions `to_canonical` / `from_canonical` that converts a Markdown construct (links, callouts…) from a dialect to the canonical form and back. |
| **3-way merge** | Merging two modified versions using their common ancestor (the base). |
| **Hunk** | A block of changed lines in a diff. |
| **Conflict** | Hunks from both sides that touch the same lines of the base with a different result. |
| **Plan** | The list of actions a sync would perform, computed before any write. |
| **Dry-run** | Computing and displaying the plan without applying anything. |
| **Trigger** | What starts a sync: interval, Git hook, API, MCP, CLI or app. |
| **Daemon** | Markoo's background process: scheduler, API and MCP. |
| **MCP** | *Model Context Protocol*: a protocol that lets an AI assistant (Claude) call Markoo's tools. |
| **Wikilink** | Obsidian link `[[Note name]]`. |
| **Embed** | Obsidian inclusion `![[file]]` (image or note). |
| **Callout** | Obsidian box `> [!type]`, equivalent to GitHub *alerts*. |
