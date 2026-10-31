//! Line-based 3-way merge.

/// Merges `kb` and `repo` against their common `base`, line by line.
///
/// Returns the merged text, or `None` when both sides changed the same lines
/// differently (a conflict). Identical changes on both sides are not conflicts.
pub fn three_way(base: &str, kb: &str, repo: &str) -> Option<String> {
    if kb == repo {
        return Some(kb.to_owned());
    }
    diffy::merge(base, kb, repo).ok()
}

/// Whether a text still has conflict markers. `=======` alone is not checked:
/// it is also a valid Markdown heading underline.
pub fn has_markers(text: &str) -> bool {
    text.lines()
        .any(|line| line.starts_with("<<<<<<<") || line.starts_with(">>>>>>>"))
}

/// Merges like [`three_way`] but always returns text: conflicting blocks are
/// surrounded by `<<<<<<<` / `=======` / `>>>>>>>` markers, for manual editing.
pub fn with_markers(base: &str, kb: &str, repo: &str) -> String {
    match diffy::merge(base, kb, repo) {
        Ok(merged) | Err(merged) => merged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "L1\nL2\nL3\nL4\n";

    #[test]
    fn changes_on_different_lines_are_combined() {
        let merged = three_way(BASE, "L1\nL2'\nL3\nL4\n", "L1\nL2\nL3\nL4'\n");
        assert_eq!(merged.as_deref(), Some("L1\nL2'\nL3\nL4'\n"));
    }

    #[test]
    fn different_changes_on_the_same_line_conflict() {
        assert_eq!(
            three_way(BASE, "L1\nKB\nL3\nL4\n", "L1\nREPO\nL3\nL4\n"),
            None
        );
    }

    #[test]
    fn identical_changes_do_not_conflict() {
        let same = "L1\nX\nL3\nL4\n";
        assert_eq!(three_way(BASE, same, same).as_deref(), Some(same));
    }

    #[test]
    fn conflicting_blocks_get_markers() {
        let draft = with_markers(BASE, "L1\nKB\nL3\nL4\n", "L1\nREPO\nL3\nL4\n");
        assert!(draft.contains("<<<<<<<") && draft.contains("KB") && draft.contains("REPO"));
    }

    #[test]
    fn different_files_without_base_conflict() {
        assert_eq!(three_way("", "a\n", "b\n"), None);
    }
}
