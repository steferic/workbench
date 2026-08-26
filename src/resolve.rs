//! Turning what someone typed into exactly one agent.
//!
//! Three callers ask this question holding three different records: the
//! control socket's JSON (`workbench wait`), a single workspace roster, and
//! the machine-wide directory that spans every workspace (`workbench ask`).
//! They have to answer it identically — the same word must not address
//! different agents depending on which verb you typed it after — so each
//! record projects onto a [`Candidate`] and the ladder lives here once.
//!
//! The ladder runs from the most specific address to the vaguest: id, alias,
//! id prefix, provider name. Each rung is narrowed to the caller's own
//! project before being called ambiguous, which is what keeps a bare `codex`
//! usable on a machine running four of them.

use crate::comms::{ENV_SESSION, ENV_WORKSPACE};

/// One agent, reduced to the fields an address can name.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub alias: Option<&'a str>,
    pub provider: &'a str,
    /// The workspace this agent belongs to — the unit `Scope` narrows to.
    pub project_id: &'a str,
    /// Its human name, for error messages.
    pub project: &'a str,
}

/// Where to look when what the caller typed could mean several agents.
#[derive(Debug, Default, Clone)]
pub struct Scope {
    /// The project to prefer, as a workspace id. Taken from the caller's own
    /// pane when it has one, or named outright with `--project`.
    pub project_id: Option<String>,
    /// The caller's own short id, so an agent asking for "codex" never
    /// resolves to itself.
    pub exclude: Option<String>,
}

impl Scope {
    /// What the environment already knows: an agent running in a workbench
    /// pane is told which session and workspace it is.
    pub fn from_env() -> Self {
        Self {
            project_id: std::env::var(ENV_WORKSPACE).ok(),
            exclude: std::env::var(ENV_SESSION).ok(),
        }
    }

    fn is_self(&self, id: &str) -> bool {
        self.exclude
            .as_deref()
            .map(|me| me.eq_ignore_ascii_case(id))
            .unwrap_or(false)
    }
}

/// How far a vague address is allowed to travel.
///
/// The distinction exists because the two things a caller does with a
/// resolved agent are not equally forgiving. Waiting on the wrong one wastes
/// a few seconds; consulting the wrong one spends a model turn in a repo that
/// has nothing to do with the question, and answers it with the wrong
/// context. So the verbs that spend a peer's time make a name mean "here",
/// and require a full id to cross a project boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// A vague address may resolve into another project when this one holds
    /// no candidate at all. What `workbench wait` has always done.
    Anywhere,
    /// Only an id or an alias crosses a project boundary. A prefix or a
    /// provider name resolves within the caller's project or not at all —
    /// and the error says which agents it declined to reach for.
    ExplicitAcrossProjects,
}

/// Resolve `target` to exactly one index into `candidates`.
///
/// Ambiguity that survives narrowing is an error naming the candidates, never
/// a guess: addressing the wrong agent silently is worse than a question.
pub fn pick(
    candidates: &[Candidate],
    target: &str,
    scope: &Scope,
    reach: Reach,
) -> Result<usize, String> {
    let wanted = target.trim().to_lowercase();
    if wanted.is_empty() {
        return Err("name an agent".to_string());
    }

    // Addressing yourself is always a mistake, and reporting it as "no match"
    // hides it — the id is right there in the listing the caller just read.
    if scope.is_self(&wanted) {
        return Err(format!("`{target}` is you"));
    }

    let pool: Vec<usize> = (0..candidates.len())
        .filter(|&i| !scope.is_self(candidates[i].id))
        .collect();

    // Vague matches that exist only in other projects. Held back rather than
    // reported at once, so a later rung of the ladder still gets its chance;
    // if none of them lands, this is a far better error than "no match".
    let mut unreachable: Vec<usize> = Vec::new();

    for (kind, matches_it) in strategies(&wanted) {
        let found: Vec<usize> = pool
            .iter()
            .copied()
            .filter(|&i| matches_it(&candidates[i]))
            .collect();

        let vague = matches!(kind, "prefix" | "provider");
        let found = match (vague && reach == Reach::ExplicitAcrossProjects, scope.project_id.as_deref()) {
            (true, Some(home)) => {
                let local: Vec<usize> = found
                    .iter()
                    .copied()
                    .filter(|&i| candidates[i].project_id == home)
                    .collect();
                if local.is_empty() {
                    unreachable.extend(found);
                }
                local
            }
            _ => narrow(candidates, found, scope),
        };

        match found.len() {
            0 => continue,
            1 => return Ok(found[0]),
            _ => return Err(ambiguous(candidates, &found, target, kind)),
        }
    }

    unreachable.sort_unstable();
    unreachable.dedup();
    if !unreachable.is_empty() {
        return Err(out_of_reach(candidates, &unreachable, target));
    }
    Err(format!("no agent matches `{target}`"))
}

type Test<'a> = Box<dyn Fn(&Candidate) -> bool + 'a>;

/// The ladder, most specific first.
fn strategies(wanted: &str) -> [(&'static str, Test<'_>); 4] {
    [
        ("id", Box::new(move |c: &Candidate| c.id.to_lowercase() == wanted)),
        (
            "alias",
            Box::new(move |c: &Candidate| {
                c.alias
                    .map(|alias| !alias.is_empty() && alias.to_lowercase() == wanted)
                    .unwrap_or(false)
            }),
        ),
        (
            "prefix",
            Box::new(move |c: &Candidate| c.id.to_lowercase().starts_with(wanted)),
        ),
        (
            "provider",
            Box::new(move |c: &Candidate| c.provider.to_lowercase() == wanted),
        ),
    ]
}

/// Prefer the caller's own project when a name means several agents.
///
/// This is what makes a bare provider name usable at all. Several Claudes run
/// at once across projects, so `claude` is almost always ambiguous — but from
/// inside a pane it nearly always means "the Claude working on this with me",
/// and that one is a single filter away.
///
/// Only ever narrows an ambiguity; if the caller's project holds none of the
/// candidates, the wider set stands and the error names them all.
fn narrow(candidates: &[Candidate], matches: Vec<usize>, scope: &Scope) -> Vec<usize> {
    if matches.len() <= 1 {
        return matches;
    }
    let Some(home) = scope.project_id.as_deref() else {
        return matches;
    };
    let local: Vec<usize> = matches
        .iter()
        .copied()
        .filter(|&i| candidates[i].project_id == home)
        .collect();
    if local.is_empty() { matches } else { local }
}

/// `abc12345 (backend, workbench)` — enough to retype as an unambiguous
/// address, which is the only reason an error lists candidates at all.
fn describe(candidate: &Candidate) -> String {
    match candidate.alias {
        Some(alias) if !alias.is_empty() => {
            format!("{} ({alias}, {})", candidate.id, candidate.project)
        }
        _ => format!("{} ({})", candidate.id, candidate.project),
    }
}

/// How many candidates an error names before it stops helping. On a machine
/// running twenty agents, `claude` matches most of them, and a message that
/// lists all twenty is one the reader skips entirely — the point of naming
/// candidates is that one of them can be retyped as an address.
const MAX_NAMED: usize = 4;

fn list(candidates: &[Candidate], found: &[usize]) -> String {
    let shown = found.len().min(MAX_NAMED);
    let named = found[..shown]
        .iter()
        .map(|&i| describe(&candidates[i]))
        .collect::<Vec<_>>()
        .join(", ");
    match found.len() - shown {
        0 => named,
        more => format!("{named} (+{more} more — see `workbench agents --all`)"),
    }
}

fn ambiguous(candidates: &[Candidate], found: &[usize], target: &str, kind: &str) -> String {
    // Which advice actually helps depends on why it is ambiguous. Candidates
    // spread across projects can be narrowed by one; candidates sharing a
    // project cannot, and saying otherwise sends the reader somewhere that
    // will not work. (The scope may be set and still not have narrowed
    // anything — a caller in a project that holds none of these.)
    let mut projects: Vec<&str> = found.iter().map(|&i| candidates[i].project).collect();
    projects.sort_unstable();
    projects.dedup();
    let hint = if projects.len() > 1 {
        " — pass --project, or address one by id or alias"
    } else {
        " — address one by id, or give it an alias"
    };
    format!(
        "`{target}` is ambiguous: it matches {} agents by {kind}: {}{hint}",
        found.len(),
        list(candidates, found)
    )
}

/// The error that teaches the cross-project address. A name means "in this
/// project", so a peer elsewhere is invisible to it — but the caller almost
/// certainly wants to know it exists, and how to reach it.
fn out_of_reach(candidates: &[Candidate], found: &[usize], target: &str) -> String {
    format!(
        "no agent matches `{target}` in this project. In other projects: {} \
         — a bare name only reaches your own project, so address one of those by its full id",
        list(candidates, found)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row {
        id: String,
        alias: Option<String>,
        provider: String,
        project: String,
    }

    fn row(id: &str, provider: &str, project: &str, alias: Option<&str>) -> Row {
        Row {
            id: id.into(),
            alias: alias.map(Into::into),
            provider: provider.into(),
            project: project.into(),
        }
    }

    /// Project ids are derived from the name so tests can name a project once.
    fn candidates(rows: &[Row]) -> Vec<Candidate<'_>> {
        rows.iter()
            .map(|r| Candidate {
                id: &r.id,
                alias: r.alias.as_deref(),
                provider: &r.provider,
                project_id: &r.project,
                project: &r.project,
            })
            .collect()
    }

    fn anywhere() -> Scope {
        Scope::default()
    }

    fn inside(project: &str) -> Scope {
        Scope {
            project_id: Some(project.into()),
            exclude: None,
        }
    }

    fn resolve<'a>(rows: &'a [Row], target: &str, scope: &Scope, reach: Reach) -> Result<&'a str, String> {
        let candidates = candidates(rows);
        pick(&candidates, target, scope, reach).map(|i| {
            // Borrowing back out of the temporary needs the original row.
            rows[i].id.as_str()
        })
    }

    #[test]
    fn an_id_or_a_prefix_resolves() {
        let rows = [
            row("abc12345", "Claude", "workbench", None),
            row("def67890", "Codex", "workbench", None),
        ];
        assert_eq!(resolve(&rows, "abc12345", &anywhere(), Reach::Anywhere).unwrap(), "abc12345");
        assert_eq!(resolve(&rows, "ABC12345", &anywhere(), Reach::Anywhere).unwrap(), "abc12345");
        assert_eq!(resolve(&rows, "def", &anywhere(), Reach::Anywhere).unwrap(), "def67890");
    }

    /// An alias is the only address that survives a restart, so it outranks
    /// every guess below it.
    #[test]
    fn an_alias_resolves_and_beats_a_provider_name() {
        let rows = [
            row("abc12345", "Claude", "workbench", Some("backend")),
            row("def67890", "Claude", "workbench", None),
        ];
        assert_eq!(resolve(&rows, "backend", &anywhere(), Reach::Anywhere).unwrap(), "abc12345");
        assert_eq!(resolve(&rows, "BACKEND", &anywhere(), Reach::Anywhere).unwrap(), "abc12345");
    }

    #[test]
    fn a_provider_name_resolves_within_the_callers_project() {
        let rows = [
            row("abc12345", "Claude", "workbench", None),
            row("def67890", "Claude", "canvas", None),
            row("aaa11111", "Codex", "workbench", None),
        ];
        // From nowhere in particular it is still ambiguous, and says so.
        let err = resolve(&rows, "claude", &anywhere(), Reach::Anywhere).unwrap_err();
        assert!(err.contains("abc12345") && err.contains("def67890"), "{err}");
        assert!(err.contains("--project"), "spread across projects: --project helps: {err}");

        // Two in the SAME project: --project cannot help, so do not suggest it.
        let together = [
            row("abc12345", "Claude", "workbench", None),
            row("aaa11111", "Claude", "workbench", None),
        ];
        let err = resolve(&together, "claude", &inside("workbench"), Reach::Anywhere).unwrap_err();
        assert!(!err.contains("--project"), "should not send them somewhere useless: {err}");
        assert!(err.contains("alias"), "{err}");

        assert_eq!(resolve(&rows, "claude", &inside("workbench"), Reach::Anywhere).unwrap(), "abc12345");
        assert_eq!(resolve(&rows, "claude", &inside("canvas"), Reach::Anywhere).unwrap(), "def67890");
    }

    /// `Anywhere` narrows an ambiguity; it must not hide the only match there is.
    #[test]
    fn anywhere_reaches_an_agent_in_another_project() {
        let rows = [row("def67890", "Codex", "canvas", None)];
        assert_eq!(
            resolve(&rows, "codex", &inside("workbench"), Reach::Anywhere).unwrap(),
            "def67890"
        );
    }

    /// The rule this change exists for. A full id reaches any project…
    #[test]
    fn an_id_crosses_a_project_boundary() {
        let rows = [row("def67890", "Codex", "canvas", Some("parser"))];
        assert_eq!(
            resolve(&rows, "def67890", &inside("workbench"), Reach::ExplicitAcrossProjects).unwrap(),
            "def67890"
        );
        // …and so does an alias, which is just as deliberate an address.
        assert_eq!(
            resolve(&rows, "parser", &inside("workbench"), Reach::ExplicitAcrossProjects).unwrap(),
            "def67890"
        );
    }

    /// …but a bare name does not, and the refusal has to teach the address
    /// that would have worked, or the caller just retries the same word.
    #[test]
    fn a_bare_name_does_not_cross_a_project_boundary() {
        let rows = [row("def67890", "Codex", "canvas", None)];
        let err = resolve(&rows, "codex", &inside("workbench"), Reach::ExplicitAcrossProjects)
            .unwrap_err();
        assert!(err.contains("def67890"), "names the peer it declined to reach: {err}");
        assert!(err.contains("canvas"), "and which project it is in: {err}");
        assert!(err.contains("full id"), "and how to reach it: {err}");
    }

    /// A name still prefers home even when it would resolve elsewhere.
    #[test]
    fn a_bare_name_prefers_home_over_a_peer_elsewhere() {
        let rows = [
            row("abc12345", "Codex", "workbench", None),
            row("def67890", "Codex", "canvas", None),
        ];
        assert_eq!(
            resolve(&rows, "codex", &inside("workbench"), Reach::ExplicitAcrossProjects).unwrap(),
            "abc12345"
        );
    }

    /// An agent asking for "codex" means a peer, never itself — otherwise a
    /// consult is delivered to the caller and `wait` returns instantly.
    #[test]
    fn an_agent_never_resolves_to_itself() {
        let rows = [
            row("abc12345", "Codex", "workbench", None),
            row("def67890", "Codex", "workbench", None),
        ];
        let me = Scope {
            project_id: Some("workbench".into()),
            exclude: Some("abc12345".into()),
        };
        assert_eq!(resolve(&rows, "codex", &me, Reach::Anywhere).unwrap(), "def67890");
        // And saying so outright beats reporting it as "no such agent".
        let err = resolve(&rows, "abc12345", &me, Reach::Anywhere).unwrap_err();
        assert!(err.contains("is you"), "{err}");
    }

    #[test]
    fn an_ambiguous_prefix_is_an_error_that_names_the_candidates() {
        let rows = [
            row("ab111111", "Claude", "workbench", Some("one")),
            row("ab222222", "Claude", "workbench", Some("two")),
        ];
        let err = resolve(&rows, "ab", &inside("workbench"), Reach::Anywhere).unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("ab111111") && err.contains("one"), "{err}");
        assert!(err.contains("ab222222") && err.contains("two"), "{err}");

        assert!(resolve(&rows, "nope", &anywhere(), Reach::Anywhere).is_err());
        assert!(resolve(&rows, "  ", &anywhere(), Reach::Anywhere).is_err());
    }
}
