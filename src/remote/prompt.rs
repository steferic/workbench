//! Reading the question an agent is blocked on, off its own screen.
//!
//! The phone used to offer a guessed pair of buttons — Approve sent Enter,
//! Deny sent Esc — with the hook's prose above them ("Claude needs your
//! permission to use Bash"). Two things were wrong with that. Enter takes
//! *whichever* option is highlighted, which the phone could not see, and the
//! hook fires for plain idleness too, so the buttons often answered a question
//! that was not being asked.
//!
//! Both providers render the same shape, and it is worth reading rather than
//! guessing. Claude:
//!
//! ```text
//!  Bash command
//!
//!    rm -f /tmp/scratch
//!    Remove /tmp/scratch file
//!
//!  Do you want to proceed?
//!  ❯ 1. Yes
//!    2. Yes, and always allow access to tmp/ from this project
//!    3. No
//! ```
//!
//! Codex:
//!
//! ```text
//!   Would you like to run the following command?
//!   Reason: Allow deleting the explicitly requested /tmp/scratch file?
//!   $ rm -f /tmp/scratch
//! › 1. Yes, proceed (y)
//!   2. Yes, and don't ask again for commands that start with `rm -f …` (p)
//!   3. No, and tell Codex what to do differently (esc)
//! ```
//!
//! So: a numbered list at the foot of the screen, and the block above it says
//! what you are agreeing to. Both were verified to answer to the bare digit —
//! no Enter, and no dependence on where the highlight sits.

use serde::Serialize;

/// A question on an agent's screen, with the choices it will accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prompt {
    /// What is being asked, as the agent laid it out: the command, the reason,
    /// the question. Rendered monospace, because some of it is a command.
    pub lines: Vec<String>,
    pub options: Vec<PromptOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptOption {
    /// The keystroke that picks it.
    pub key: String,
    pub label: String,
    /// The choice the agent has highlighted — Enter would take this one.
    pub selected: bool,
}

/// How far above the options we are willing to read for context.
const MAX_BODY_LINES: usize = 14;
/// A prompt sits at the foot of the screen. More content than this below the
/// last option means we are looking at a numbered list in the agent's output,
/// not a question.
const MAX_LINES_BELOW: usize = 3;
const MAX_LINE: usize = 200;

/// Find the question on a screen, if there is one.
pub fn parse(screen: &str) -> Option<Prompt> {
    let lines: Vec<&str> = screen.lines().map(str::trim_end).collect();

    // Work up from the bottom: the last numbered line starts the run.
    let last = lines.iter().rposition(|line| option(line).is_some())?;
    if lines[last + 1..].iter().filter(|l| !l.is_empty()).count() > MAX_LINES_BELOW {
        return None;
    }

    let mut options = Vec::new();
    let mut first = last;
    let mut expected = option(lines[last])?.0;
    loop {
        let Some((number, label, selected)) = option(lines[first]) else {
            break;
        };
        // The run has to count down to 1 without gaps; anything else is prose
        // that happens to start with a digit.
        if number != expected {
            break;
        }
        options.push(PromptOption {
            key: number.to_string(),
            label: truncate(&label),
            selected,
        });
        first -= 1;
        expected -= 1;
        if expected == 0 || first == 0 {
            break;
        }
    }
    options.reverse();
    if options.len() < 2 || options[0].key != "1" {
        return None;
    }

    // A choice has to be attached to something. Nothing above the run means
    // we found a list the agent was writing, not a question it stopped on.
    let lines = body(&lines[..=first]);
    if lines.is_empty() {
        return None;
    }

    Some(Prompt { lines, options })
}

/// `❯ 2. Yes, and always allow…` → `(2, "Yes, and always allow…", true)`.
fn option(line: &str) -> Option<(u32, String, bool)> {
    let trimmed = line.trim_start();
    let (selected, rest) = match trimmed.strip_prefix(['❯', '›', '>', '▶']) {
        Some(rest) => (true, rest.trim_start()),
        None => (false, trimmed),
    };
    let (number, label) = rest.split_once('.')?;
    let number: u32 = number.parse().ok()?;
    if !(1..=9).contains(&number) {
        return None;
    }
    let label = label.trim();
    // `1.` alone, or `1.5` in prose, is not a choice.
    if label.is_empty() || label.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((number, label.to_string(), selected))
}

/// The block of screen that explains the question.
///
/// Read upward until something says the agent's own output has started: a rule
/// drawn across the pane, or a line beginning with one of the bullets both
/// providers use to mark a turn.
fn body(above: &[&str]) -> Vec<String> {
    let mut collected: Vec<String> = Vec::new();
    for line in above.iter().rev() {
        if is_rule(line) || is_turn_marker(line) {
            break;
        }
        if collected.len() == MAX_BODY_LINES {
            break;
        }
        collected.push(truncate(line.trim_end()));
    }
    collected.reverse();

    // Blank lines are the provider's spacing, not ours: keep single ones as
    // paragraph breaks, drop runs and edges.
    let mut out: Vec<String> = Vec::new();
    for line in collected {
        if line.trim().is_empty() {
            if out.is_empty() || out.last().map(|l| l.trim().is_empty()) == Some(true) {
                continue;
            }
        }
        out.push(line);
    }
    while out.last().map(|l| l.trim().is_empty()) == Some(true) {
        out.pop();
    }
    out
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= 8 && trimmed.chars().all(|c| matches!(c, '─' | '━' | '-' | '═'))
}

/// The glyphs Claude and Codex print at the head of a turn or a tool call.
fn is_turn_marker(line: &str) -> bool {
    line.trim_start()
        .starts_with(['•', '⏺', '✻', '✗', '■', '❯', '›', '⎿'])
}

fn truncate(line: &str) -> String {
    if line.chars().count() <= MAX_LINE {
        return line.to_string();
    }
    line.chars().take(MAX_LINE - 1).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from Claude Code 2.1.220 by driving it in a pty until it
    /// blocked. Kept verbatim, indentation included — that is the input.
    const CLAUDE: &str = "\
❯ Run the shell command: rm -f /tmp/zzz. Nothing else.

  Running 1 shell command…
  ⎿  $ rm -f /tmp/zzz

────────────────────────────────────────────────────────────
 Bash command

   rm -f /tmp/zzz
   Remove /tmp/zzz file

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and always allow access to tmp/ from this project
   3. No

 Esc to cancel · Tab to amend · ctrl+e to explain
";

    /// Captured the same way from Codex 0.146.0.
    const CODEX: &str = "\
› Run the shell command: rm -f /tmp/zzz. Nothing else.

• You have 1 usage limit reset available. Run /usage to use one.

• Running rm -f /tmp/zzz

  Would you like to run the following command?

  Environment: local

  Reason: Allow deleting the explicitly requested /tmp/zzz file?

  $ rm -f /tmp/zzz

› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `rm -f /tmp/zzz` (p)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel
";

    #[test]
    fn claude_asking_to_run_a_command_is_read_whole() {
        let prompt = parse(CLAUDE).expect("a prompt is on screen");

        assert_eq!(
            prompt.lines,
            vec![
                " Bash command",
                "",
                "   rm -f /tmp/zzz",
                "   Remove /tmp/zzz file",
                "",
                " Do you want to proceed?",
            ],
            "the command being approved has to reach the phone, not just \"needs permission\""
        );
        let keys: Vec<&str> = prompt.options.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(keys, vec!["1", "2", "3"]);
        assert_eq!(prompt.options[0].label, "Yes");
        assert_eq!(prompt.options[2].label, "No");
        assert!(prompt.options[0].selected, "Enter would take this one");
        assert!(!prompt.options[1].selected);
    }

    #[test]
    fn codex_asking_the_same_thing_parses_the_same_way() {
        let prompt = parse(CODEX).expect("a prompt is on screen");

        assert_eq!(
            prompt.lines[0],
            "  Would you like to run the following command?"
        );
        assert!(
            prompt.lines.iter().any(|l| l.contains("$ rm -f /tmp/zzz")),
            "{:?}",
            prompt.lines
        );
        // The turn marker above it is where the question starts.
        assert!(!prompt.lines.iter().any(|l| l.contains("usage limit")));
        assert_eq!(prompt.options.len(), 3);
        assert!(prompt.options[0].label.starts_with("Yes, proceed"));
    }

    #[test]
    fn an_ordinary_screen_has_no_prompt_on_it() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("⏺ Done. Tests pass.\n\n❯ \n"), None);
        // One choice is not a choice.
        assert_eq!(parse(" Do you want to proceed?\n ❯ 1. Yes\n"), None);
    }

    /// The failure this guards is a phone showing buttons for something the
    /// agent wrote in passing.
    #[test]
    fn a_numbered_list_in_the_middle_of_output_is_not_a_question() {
        let screen = "\
⏺ Here is the plan:

  1. Read the config
  2. Migrate the rows
  3. Verify the counts

  I'll start with the config now, then move on to the migration and finally
  check that the row counts line up on both sides.

❯
";
        assert_eq!(parse(screen), None);
    }

    #[test]
    fn prose_that_starts_with_a_digit_is_not_an_option_run() {
        // Version numbers and decimals must not be mistaken for choices.
        assert!(option("2.1.220 is installed").is_none());
        assert!(option("  1.").is_none());
        assert_eq!(option(" ❯ 1. Yes").unwrap().0, 1);
    }
}
