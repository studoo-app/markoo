# Test fixtures

Small knowledge bases and a repo used by the test suite. Tests copy them to a
temporary directory before syncing; never modify them in place.

| Folder | Content |
|--------|---------|
| `markdown/kb/` | Plain Markdown KB (`kb_type = "markdown"`, identity dialect) |
| `obsidian/kb/` | Obsidian vault (`kb_type = "obsidian"`): wikilinks, embed, callout, frontmatter |
| `repo/docs/` | Repo side in GitHub Flavored Markdown, matching `obsidian/kb/` once converted |
