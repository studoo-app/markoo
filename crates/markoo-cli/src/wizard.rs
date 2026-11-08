//! The questions asked by `markoo pair add` without arguments.

use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};
use markoo_core::config::{NewPair, expand_home, is_valid_label};
use markoo_core::kb::{self, KbType};

/// Asks for the settings of a new pair, starting with its project: one of
/// `projects`, the existing ones, or a new one named here. `taken` lists the
/// project and name of the existing pairs: a name is used once per project.
pub fn ask_new_pair(
    input: &mut impl BufRead,
    output: &mut impl Write,
    projects: &[&str],
    taken: &[(&str, &str)],
) -> Result<NewPair> {
    let project = ask_project(input, output, projects)?;
    let name = ask_until(input, output, "Name of the pair", None, |answer| {
        if !is_valid_label(answer) {
            return Err(INVALID_LABEL);
        }
        let used = taken.iter().any(|(other_project, other_name)| {
            other_project.to_lowercase() == project.to_lowercase()
                && other_name.to_lowercase() == answer.to_lowercase()
        });
        if used {
            return Err("this project already has a pair with this name");
        }
        Ok(())
    })?;
    let known = kb::known_kbs();
    if !known.is_empty() {
        writeln!(output, "Knowledge bases found:")?;
        for folder in &known {
            writeln!(output, "  {}", folder.display())?;
        }
    }
    let kb = ask_folder(input, output, "Knowledge base folder")?;
    let detected = KbType::detect(&expand_home(&kb));
    writeln!(output, "  Type: {}", detected.id())?;
    let kb_type = Some(detected.id().to_owned());
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
        id: None,
        name,
        project: Some(project),
        kb,
        kb_type,
        repo,
        direction: Some(direction),
        target_changes,
        settings: Vec::new(),
    })
}

/// What is wrong with a project or pair name refused by `is_valid_label`.
const INVALID_LABEL: &str = "use 1 to 64 characters on one line, without /";

/// The first pair names its project; the next ones pick an existing project
/// by number, or name a new one.
fn ask_project(
    input: &mut impl BufRead,
    output: &mut impl Write,
    projects: &[&str],
) -> Result<String> {
    if projects.is_empty() {
        return ask_until(input, output, "Project name", None, |answer| {
            is_valid_label(answer).then_some(()).ok_or(INVALID_LABEL)
        });
    }
    writeln!(output, "Projects:")?;
    for (number, project) in projects.iter().enumerate() {
        writeln!(output, "  {}. {project}", number + 1)?;
    }
    loop {
        let answer = ask(input, output, "Project (number, or a new name)", None)?;
        // A number outside the list is a new name: "2026" is a valid project.
        let listed = answer
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| projects.get(index));
        if let Some(project) = listed {
            return Ok((*project).to_owned());
        } else if is_valid_label(&answer) {
            return Ok(answer);
        } else {
            writeln!(output, "  {INVALID_LABEL}")?;
        }
    }
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
        ask_new_pair(&mut text.as_bytes(), &mut Vec::new(), &[], &[])
    }

    fn answers_with(projects: &[&str], text: &str) -> Result<NewPair> {
        ask_new_pair(&mut text.as_bytes(), &mut Vec::new(), projects, &[])
    }

    #[test]
    fn the_first_pair_names_its_project() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!("\nMarkoo\nnotes\n{folder}\n{folder}\n\n")).unwrap();
        assert_eq!(pair.project.as_deref(), Some("Markoo"));
    }

    #[test]
    fn an_existing_project_is_picked_by_number() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers_with(
            &["Markoo", "Site web"],
            &format!("2\nnotes\n{folder}\n{folder}\n\n"),
        )
        .unwrap();
        assert_eq!(pair.project.as_deref(), Some("Site web"));
    }

    #[test]
    fn a_number_outside_the_list_names_a_new_project() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair =
            answers_with(&["Markoo"], &format!("2026\nnotes\n{folder}\n{folder}\n\n")).unwrap();
        assert_eq!(pair.project.as_deref(), Some("2026"));
    }

    #[test]
    fn a_new_project_can_be_typed_instead_of_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers_with(
            &["Markoo"],
            &format!("Clara\nnotes\n{folder}\n{folder}\n\n"),
        )
        .unwrap();
        assert_eq!(pair.project.as_deref(), Some("Clara"));
    }

    #[test]
    fn invalid_answers_are_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!(
            "Markoo\na/b\nnotes\n/nope\n{folder}\n{folder}\nsideways\n\n"
        ))
        .unwrap();
        assert_eq!(
            (pair.name.as_str(), pair.direction.as_deref()),
            ("notes", Some("both"))
        );
    }

    #[test]
    fn a_pair_name_is_free_text() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!(
            "Markoo\nDocumentation technique\n{folder}\n{folder}\n\n"
        ))
        .unwrap();
        assert_eq!(
            (pair.name.as_str(), pair.id),
            ("Documentation technique", None)
        );
    }

    #[test]
    fn a_name_used_in_the_project_is_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let taken = [("Markoo", "Docs"), ("Clara", "Notes")];
        let mut output = Vec::new();
        let pair = ask_new_pair(
            &mut format!("1\ndocs\nNotes\n{folder}\n{folder}\n\n").as_bytes(),
            &mut output,
            &["Markoo", "Clara"],
            &taken,
        )
        .unwrap();
        assert_eq!(pair.name, "Notes");
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("already has a pair with this name")
        );
    }

    #[test]
    fn one_way_asks_about_target_changes() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display();
        let pair = answers(&format!(
            "Markoo\nnotes\n{folder}\n{folder}\nkb-to-repo\n\n"
        ))
        .unwrap();
        assert_eq!(pair.target_changes.as_deref(), Some("protect"));
    }

    #[test]
    fn end_of_input_stops_the_wizard() {
        assert!(answers("Markoo\nnotes\n").is_err());
    }
}
