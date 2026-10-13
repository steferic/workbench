//! Turning a message into the words a reader would say it contains.
//!
//! Agents write code as often as prose, and a cloud of `let`, `fn` and file
//! paths says nothing about how anyone talks. So fenced blocks and inline code
//! go first, then any chunk that looks like a path, an identifier or a URL.
//! What is left is lower-cased and split on punctuation.
//!
//! Stop words are counted like any other word and only dropped when a cloud is
//! drawn (see `is_stop_word`), so a better list never needs a rescan.

/// The prose words in an agent's reply.
pub fn agent_words(text: &str) -> Vec<String> {
    words(&without_code(text))
}

/// The words a person typed, without what tools and hooks wrapped around them.
///
/// A user turn in a transcript carries more than the user's own text: system
/// reminders, pasted blocks, slash-command echoes and Workbench's own consults
/// all arrive as XML-ish blocks or bracketed markers, and none of it was typed.
pub fn typed_words(text: &str) -> Vec<String> {
    let text = without_tag_blocks(text);
    let trimmed = text.trim_start();
    if trimmed.starts_with("[workbench ")
        || trimmed.starts_with("# AGENTS.md instructions")
        || trimmed.starts_with("Caveat: The messages below")
    {
        return Vec::new();
    }
    let text = without_markers(&text);
    let words = words(&without_code(&text));
    // A paste is stored as if typed — Codex never marks one, and Claude Code
    // keeps the pasted text in full. Nobody types 800 words in one message,
    // and a word a person types more than three times in one message is
    // almost always a pasted list ("12 people found this helpful", over and
    // over); without these, one pasted page of reviews outweighed months of
    // typing.
    if words.len() > MAX_TYPED_WORDS {
        return Vec::new();
    }
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let keep: Vec<bool> = words
        .iter()
        .map(|word| {
            let n = seen.entry(word.as_str()).or_default();
            *n += 1;
            *n <= MAX_REPEATS
        })
        .collect();
    words
        .into_iter()
        .zip(keep)
        .filter_map(|(word, keep)| keep.then_some(word))
        .collect()
}

const MAX_TYPED_WORDS: usize = 800;
const MAX_REPEATS: usize = 3;

fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in text.split(|c: char| c.is_whitespace() || matches!(c, '—' | '–' | '…')) {
        if looks_like_code(chunk) {
            continue;
        }
        for part in chunk.split([',', ';', '!', '?', '(', ')', '"', '“', '”', '*', '|']) {
            if let Some(word) = normalize(part) {
                out.push(word);
            }
        }
    }
    out
}

fn normalize(part: &str) -> Option<String> {
    let part = part.replace('’', "'");
    let part = part.trim_matches(|c: char| !c.is_alphanumeric());
    let part = part
        .strip_suffix("'s")
        .or_else(|| part.strip_suffix("'S"))
        .unwrap_or(part);
    let count = part.chars().count();
    if !(2..=24).contains(&count) {
        return None;
    }
    if !part
        .chars()
        .all(|c| c.is_alphabetic() || c == '\'' || c == '-')
    {
        return None;
    }
    // `AgentView`, `iPhone` and the like are names in code, not words. An
    // all-caps acronym like `TUI` is kept.
    let rest_has_upper = part.chars().skip(1).any(char::is_uppercase);
    if rest_has_upper && part.chars().any(char::is_lowercase) {
        return None;
    }
    Some(part.to_lowercase())
}

/// Paths, URLs, `a::b`, `a_b`, `x=1`, `file.rs`, `<tag>`, `@mention`.
fn looks_like_code(chunk: &str) -> bool {
    if chunk.contains([
        '/', '\\', '_', '=', '<', '>', '{', '}', '[', ']', '@', '#', '$', '`', '~', '^', '+',
    ]) || chunk.contains("::")
        || chunk.contains("->")
    {
        return true;
    }
    // A dot between two letters: `main.rs`, `e.g`, `foo.bar()`.
    let chars: Vec<char> = chunk.chars().collect();
    chars
        .windows(3)
        .any(|w| w[0].is_alphanumeric() && w[1] == '.' && w[2].is_alphanumeric())
}

/// Drop fenced blocks and inline code spans.
fn without_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut inline = false;
        for c in line.chars() {
            if c == '`' {
                inline = !inline;
                out.push(' ');
            } else if !inline {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

/// Remove `<tag …>…</tag>` blocks. An opening tag with no close is removed
/// on its own, so a stray `<br>` does not swallow the rest of the message.
fn without_tag_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        let opens = !name.is_empty()
            && name.starts_with(|c: char| c.is_ascii_alphabetic())
            && after[name_len..].starts_with(['>', ' ', '\n']);
        let Some(open_end) = opens.then(|| after.find('>')).flatten() else {
            out.push_str(&rest[..start + 1]);
            rest = after;
            continue;
        };
        out.push_str(&rest[..start]);
        let body = &after[open_end + 1..];
        let close = format!("</{name}>");
        rest = match body.find(&close) {
            Some(end) => &body[end + close.len()..],
            None => body,
        };
    }
    out.push_str(rest);
    out
}

/// `[Image #1]`, `[Pasted text #2 +40 lines]`, `[Request interrupted by user]`.
fn without_markers(text: &str) -> String {
    const MARKERS: [&str; 4] = ["[Image", "[Pasted text", "[Request interrupted", "[Shell"];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = MARKERS.iter().filter_map(|m| rest.find(m)).min() {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find(']') {
            Some(end) => &rest[start + end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Words too common to say anything about the speaker.
pub fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.binary_search(&word).is_ok()
}

/// Sorted, for the binary search above. Standard English function words plus
/// their contractions, both with and without the apostrophe, since people
/// type `dont` as often as `don't`.
const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "ain't",
    "all",
    "also",
    "am",
    "an",
    "and",
    "any",
    "are",
    "aren't",
    "arent",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "can't",
    "cannot",
    "cant",
    "could",
    "couldn't",
    "couldnt",
    "did",
    "didn't",
    "didnt",
    "do",
    "does",
    "doesn't",
    "doesnt",
    "doing",
    "don't",
    "dont",
    "down",
    "during",
    "each",
    "either",
    "else",
    "etc",
    "even",
    "ever",
    "every",
    "few",
    "for",
    "from",
    "further",
    "get",
    "gets",
    "got",
    "had",
    "hadn't",
    "has",
    "hasn't",
    "have",
    "haven't",
    "having",
    "he",
    "he'd",
    "he'll",
    "he's",
    "her",
    "here",
    "here's",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "how's",
    "i",
    "i'd",
    "i'll",
    "i'm",
    "i've",
    "id",
    "if",
    "im",
    "in",
    "into",
    "is",
    "isn't",
    "isnt",
    "it",
    "it'll",
    "it's",
    "its",
    "itself",
    "ive",
    "just",
    "let",
    "let's",
    "lets",
    "like",
    "me",
    "might",
    "more",
    "most",
    "much",
    "must",
    "mustn't",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "ok",
    "okay",
    "on",
    "once",
    "one",
    "only",
    "or",
    "other",
    "ought",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "shall",
    "shan't",
    "she",
    "she'd",
    "she'll",
    "she's",
    "should",
    "shouldn't",
    "so",
    "some",
    "such",
    "than",
    "that",
    "that'll",
    "that's",
    "thats",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "there's",
    "theres",
    "these",
    "they",
    "they'd",
    "they'll",
    "they're",
    "they've",
    "theyre",
    "this",
    "those",
    "though",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "upon",
    "us",
    "very",
    "via",
    "was",
    "wasn't",
    "we",
    "we'd",
    "we'll",
    "we're",
    "we've",
    "were",
    "weren't",
    "what",
    "what's",
    "whats",
    "when",
    "when's",
    "where",
    "where's",
    "whether",
    "which",
    "while",
    "who",
    "who's",
    "whom",
    "whose",
    "why",
    "why's",
    "will",
    "with",
    "won't",
    "wont",
    "would",
    "wouldn't",
    "wouldnt",
    "yes",
    "yet",
    "you",
    "you'd",
    "you'll",
    "you're",
    "you've",
    "your",
    "youre",
    "yours",
    "yourself",
    "yourselves",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stop_list_is_sorted_for_its_binary_search() {
        let mut sorted = STOP_WORDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, STOP_WORDS);
        assert!(is_stop_word("the") && is_stop_word("don't") && !is_stop_word("agent"));
    }

    #[test]
    fn code_paths_and_identifiers_are_not_words() {
        let text = "Fixed `parse()` in src/remote/prompt.rs — the AgentView now\n\
                    ```rust\nlet x = 1;\n```\n\
                    reads **memory_mb** from https://x.y and main.rs, e.g. Tests pass!";
        assert_eq!(
            agent_words(text),
            vec!["fixed", "in", "the", "now", "reads", "from", "and", "tests", "pass"]
        );
    }

    #[test]
    fn contractions_and_possessives_read_as_people_write_them() {
        assert_eq!(
            agent_words("Don’t break the agent's TUI; it's long-lived."),
            vec!["don't", "break", "the", "agent", "tui", "it", "long-lived"]
        );
    }

    #[test]
    fn what_wraps_a_typed_message_is_not_counted_as_typed() {
        let text = "<system-reminder>\nIgnore me\n</system-reminder>[Image #1] commit \
                    and push <pasted_content id=\"a1\">pasted words</pasted_content> it";
        assert_eq!(typed_words(text), vec!["commit", "and", "push", "it"]);
        assert!(typed_words("[workbench consult 12 from agent ab] hello").is_empty());
        assert!(typed_words("<command-name>/clear</command-name>").is_empty());
        // A comparison is not a tag.
        assert_eq!(typed_words("if a < b go"), vec!["if", "go"]);
    }

    #[test]
    fn a_paste_does_not_outweigh_what_was_typed() {
        let reviews = "Great station. 12 people found this helpful. ".repeat(40);
        let words = typed_words(&reviews);
        assert_eq!(words.iter().filter(|w| *w == "helpful").count(), 3);
        assert!(typed_words(&"word ".repeat(801)).is_empty());
    }
}
