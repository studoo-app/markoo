//! The questions asked by `markoo pair add` without arguments.

use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};
use markoo_core::config::{NewPair, expand_home, is_valid_name};
use markoo_core::kb;

/// Asks for the settings of a new pair.
pub fn ask_new_pair(input: &mut impl BufRead, output: &mut impl Write) -> Result<NewPair> {
    let name = ask_until(input, output, "Name of the pair", None, |answer| {
        is_valid_name(answer)
            .then_some(())
            .ok_or("use lowercase letters, digits and hyphens")
    })?;
    let kb = ask_folder(input, output, "Knowledge base folder")?;
    let mut kb_type = None;
    if let Some(tool) = kb::detect_unsupported(&expand_home(&kb)) {
        writeln!(
            output,
            "This folder is in a {tool} knowledge base. Its adapter is not available yet: \
             links would be copied without conversion."
        )?;
        let accepted = ask(
            input,
            output,
            "Sync it as plain Markdown anyway? [y/N]",
            Some("n"),
        )?;
        if !accepted.eq_ignore_ascii_case("y") {
            bail!("cancelled");
        }
        kb_type = Some("markdown".to_owned());
    }
    let repo = ask_folder(input, output, "Repository folder")?;
    let direction = ask_until(
        input,
        output,
        "Direction (both, kb-to-repo, repo-to-kb)",
        Some("both"),
        |answer| {
            ["both", "kb-to-repo", "repo-to-kb"]
                .contains(&answer)
                .then_some(())
                .ok_or("choose both, kb-to-repo or repo-to-kb")
        },
    )?;
    let target_changes = if direction == "both" {
        None
    } else {
        Some(ask_until(
            input,
            output,
            "If the target is edited (protect: report it, overwrite: reset it)",
            Some("protect"),
            |answer| {
                ["protect", "overwrite"]
                    .contains(&answer)
                    .then_some(())
                    .ok_or("choose protect or overwrite")
            },
        )?)
    };
    Ok(NewPair {
        name,
        kb,
        kb_type,
        repo,
        direction: Some(direction),
        target_changes,
    })
}

fn ask_folder(input: &mut impl BufRead, output: &mut impl Write, label: &str) -> Result<String> {
    ask_until(input, output, label, None, |answer| {
        expand_home(answer)
            .is_dir()
            .then_some(())
            .ok_or("this folder does not exist")
    })
}

fn ask_until(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: Option<&str>,
    check: impl Fn(&str) -> Result<(), &'static str>,
) -> Result<String> {
    loop {
        let answer = ask(input, output, label, default)?;
        match check(&answer) {
            Ok(()) => return Ok(answer),
            Err(message) => writeln!(output, "  {message}")?,
        }
    }
}

fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    default: Option<&str>,
) -> Result<String> {
    match default {
        Some(default) => write!(output, "{label} [{default}]: ")?,
        None => write!(output, "{label}: ")?,
    }
    output.flush()?;
    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .context("cannot read the answer")?;
    if read == 0 {
        bail!("no more input");
    }
    let answer = line.trim();
    Ok(if answer.is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        answer.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(text: &str) -> Result<NewPair> {
        ask_new_pair(&mut text.as_bytes(), &mut Vec::new())
    }

    #[test]
    fn invalid_answers_are_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!(
            "Bad Name\nnotes\n/nope\n{folder}\n{folder}\nsideways\n\n"
        ))
        .unwrap();
        assert_eq!(
            (pair.name.as_str(), pair.direction.as_deref()),
            ("notes", Some("both"))
        );
    }

    #[test]
    fn one_way_asks_about_target_changes() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!("notes\n{folder}\n{folder}\nkb-to-repo\n\n")).unwrap();
        assert_eq!(pair.target_changes.as_deref(), Some("protect"));
    }

    #[test]
    fn end_of_input_stops_the_wizard() {
        assert!(answers("notes\n").is_err());
    }
}
