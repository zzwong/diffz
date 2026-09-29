//! Places release tags on a compare's range; shared by the GitHub and GitLab compare loaders.
use crate::Result;
use std::collections::{HashMap, HashSet};

/// The timeline keeps the newest this many tags; older ones fold into its first step.
pub const MAX_RELEASES: usize = 30;
/// Tag listings stop after this many pages of 100.
pub const TAG_PAGES: usize = 10;
/// Commit pages and step compares run on at most this many `gh` or `glab` processes at once.
/// They start only after the tag, release, and compare reads, which overlap one another.
pub const WIDTH: usize = 4;

/// One step of the timeline: `from` (the previous tag's commit, or the base) to `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// `None` for the untagged commits after the last tag.
    pub tag: Option<String>,
    pub from: String,
    pub to: String,
}

/// Cuts the first-parent path from `head` back through `range`, the compare's commits as
/// `(commit, first parent)`, at each tag on it. A tag off that path, such as one on a release
/// branch, is left out even when the branch was merged, because a step to it would not be
/// linear. Where tags share a commit, the first with a release wins, else the first listed.
/// Returns the steps and the tags folded into the first step to keep [`MAX_RELEASES`].
pub fn steps(
    range: &[(String, Option<String>)],
    base: &str,
    head: &str,
    tags: &[(String, String)],
    released: &HashSet<String>,
) -> (Vec<Step>, Vec<String>) {
    let parents: HashMap<&str, Option<&str>> = range
        .iter()
        .map(|(sha, parent)| (sha.as_str(), parent.as_deref()))
        .collect();
    let mut path: Vec<String> = vec![];
    let mut at_commit = Some(head).filter(|h| parents.contains_key(h));
    while let Some(sha) = at_commit {
        path.push(sha.to_owned());
        at_commit = parents[sha].filter(|p| parents.contains_key(p) && path.len() <= range.len());
    }
    path.reverse();
    let range = path;
    let position: HashMap<&str, usize> = range
        .iter()
        .enumerate()
        .map(|(i, sha)| (sha.as_str(), i))
        .collect();
    let mut at: HashMap<usize, &str> = HashMap::new();
    for (name, sha) in tags {
        let Some(&i) = position.get(sha.as_str()) else {
            continue;
        };
        match at.get(&i) {
            Some(kept) if released.contains(*kept) || !released.contains(name) => {}
            _ => {
                at.insert(i, name);
            }
        }
    }
    let mut placed: Vec<(usize, &str)> = at.into_iter().collect();
    placed.sort_unstable();
    let folded: Vec<String> = placed
        .drain(..placed.len().saturating_sub(MAX_RELEASES))
        .map(|(_, name)| name.to_owned())
        .collect();
    let mut steps = Vec::with_capacity(placed.len() + 1);
    let mut from = base.to_owned();
    for (i, name) in placed {
        steps.push(Step {
            tag: Some(name.to_owned()),
            from: std::mem::replace(&mut from, range[i].clone()),
            to: range[i].clone(),
        });
    }
    if let Some(head) = range
        .last()
        .filter(|head| !steps.is_empty() && **head != from)
    {
        steps.push(Step {
            tag: None,
            from,
            to: head.clone(),
        });
    }
    (steps, folded)
}

/// The commits each step adds: those its `to` reaches through `range`, the compare's commits
/// as `(commit, parents)`, that no earlier step reached. Merged branches join the step that
/// merged them, so a blamed commit off the first-parent path still finds its release.
pub fn members(range: &[(String, Vec<String>)], steps: &[Step]) -> Vec<Vec<String>> {
    let parents: HashMap<&str, &[String]> = range
        .iter()
        .map(|(sha, parents)| (sha.as_str(), parents.as_slice()))
        .collect();
    let mut seen: HashSet<&str> = HashSet::new();
    steps
        .iter()
        .map(|step| {
            let mut added = vec![];
            let mut next = vec![step.to.as_str()];
            while let Some(sha) = next.pop() {
                let Some(up) = parents.get(sha) else { continue };
                if seen.insert(sha) {
                    added.push(sha.to_owned());
                    next.extend(up.iter().map(String::as_str));
                }
            }
            added
        })
        .collect()
}

pub fn folded_warning(folded: &[String]) -> Option<String> {
    let (first, last) = (folded.first()?, folded.last()?);
    Some(format!(
        "This range holds {} release tags; the timeline keeps the newest {MAX_RELEASES} and folds the oldest {} ({first} to {last}) into its first step.",
        folded.len() + MAX_RELEASES,
        folded.len()
    ))
}

/// Runs `f` on every item with at most [`WIDTH`] threads, keeping their order; the first error wins.
pub fn bounded<T: Sync, R: Send>(
    items: &[T],
    f: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut done: Vec<(usize, Result<R>)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..WIDTH.min(items.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(i) else {
                            return done;
                        };
                        let result = f(item);
                        let failed = result.is_err();
                        done.push((i, result));
                        if failed {
                            // Stop the others taking more work; what they hold still finishes.
                            next.store(items.len(), std::sync::atomic::Ordering::Relaxed);
                            return done;
                        }
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    if done.len() != items.len() && done.iter().all(|(_, r)| r.is_ok()) {
        return Err("a release read stopped unexpectedly".into());
    }
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `c1` to `cN`, each the first parent of the next, `c1` on the base.
    fn linear(n: usize) -> Vec<(String, Option<String>)> {
        (1..=n)
            .map(|i| {
                let parent = if i == 1 {
                    "base".into()
                } else {
                    format!("c{}", i - 1)
                };
                (format!("c{i}"), Some(parent))
            })
            .collect()
    }
    fn tags(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, s)| (n.to_string(), s.to_string()))
            .collect()
    }
    fn place(
        range: &[(String, Option<String>)],
        listed: &[(String, String)],
        released: &HashSet<String>,
    ) -> (Vec<Step>, Vec<String>) {
        let head = range.last().map_or("base", |(sha, _)| sha.as_str());
        steps(range, "base", head, listed, released)
    }
    fn names(steps: &[Step]) -> Vec<Option<&str>> {
        steps.iter().map(|s| s.tag.as_deref()).collect()
    }

    #[test]
    fn tags_follow_the_range_and_skip_commits_off_it() {
        // Listed newest first, as both providers do; one sits on an unmerged release branch.
        let listed = tags(&[
            ("v3", "c5"),
            ("v2.1-hotfix", "side"),
            ("v2", "c3"),
            ("v1", "c1"),
            ("v0", "base"),
        ]);
        let (steps, folded) = place(&linear(5), &listed, &HashSet::new());
        assert!(folded.is_empty());
        assert_eq!(names(&steps), [Some("v1"), Some("v2"), Some("v3")]);
        assert_eq!(
            steps
                .iter()
                .map(|s| (s.from.as_str(), s.to.as_str()))
                .collect::<Vec<_>>(),
            [("base", "c1"), ("c1", "c3"), ("c3", "c5")]
        );
    }

    #[test]
    fn tags_on_a_merged_branch_are_off_the_path() {
        // c3 merges the release branch s1, cut from c1, whose tag is reachable from the head.
        let mut range = linear(4);
        range.insert(1, ("s1".into(), Some("c1".into())));
        let listed = tags(&[("v2", "c4"), ("v1.1", "s1"), ("v1", "c1")]);
        let (steps, _) = place(&range, &listed, &HashSet::new());
        assert_eq!(names(&steps), [Some("v1"), Some("v2")]);
        assert_eq!((steps[1].from.as_str(), steps[1].to.as_str()), ("c1", "c4"));
    }

    #[test]
    fn an_untagged_head_ends_the_timeline() {
        let (tail, _) = place(&linear(4), &tags(&[("v1", "c2")]), &HashSet::new());
        assert_eq!(names(&tail), [Some("v1"), None]);
        assert_eq!((tail[1].from.as_str(), tail[1].to.as_str()), ("c2", "c4"));
        // No tag on the range at all means no timeline, not a single untagged step.
        let (none, _) = place(&linear(4), &tags(&[("v0", "base")]), &HashSet::new());
        assert!(none.is_empty());
    }

    #[test]
    fn a_released_tag_wins_a_shared_commit() {
        let listed = tags(&[("v1.0.0", "c2"), ("v1.0", "c2")]);
        let (plain, _) = place(&linear(2), &listed, &HashSet::new());
        assert_eq!(names(&plain), [Some("v1.0.0")]);
        let released = HashSet::from(["v1.0".to_string()]);
        let (with_release, _) = place(&linear(2), &listed, &released);
        assert_eq!(names(&with_release), [Some("v1.0")]);
    }

    #[test]
    fn the_newest_tags_are_kept_and_the_rest_fold_into_the_first_step() {
        let listed: Vec<_> = (1..=40)
            .rev()
            .map(|i| (format!("v{i}"), format!("c{i}")))
            .collect();
        let (steps, folded) = place(&linear(40), &listed, &HashSet::new());
        assert_eq!(steps.len(), MAX_RELEASES);
        assert_eq!(folded.len(), 10);
        assert_eq!((folded[0].as_str(), folded[9].as_str()), ("v1", "v10"));
        // The first kept step starts at the base, so it covers the folded tags' commits.
        assert_eq!(
            (steps[0].tag.as_deref(), steps[0].from.as_str()),
            (Some("v11"), "base")
        );
        assert_eq!(steps.last().unwrap().tag.as_deref(), Some("v40"));
        let warning = folded_warning(&folded).unwrap();
        assert!(
            warning.contains("40 release tags") && warning.contains("(v1 to v10)"),
            "{warning}"
        );
        assert_eq!(folded_warning(&[]), None);
    }

    #[test]
    fn merged_branches_join_the_step_that_merged_them() {
        // c3 merges s1, cut from c1; v1 tags c2 and v2 tags c4.
        let mut range: Vec<(String, Vec<String>)> = linear(4)
            .into_iter()
            .map(|(sha, parent)| (sha, parent.into_iter().collect()))
            .collect();
        range[2].1.push("s1".into());
        range.push(("s1".into(), vec!["c1".into()]));
        let (placed, _) = place(
            &linear(4),
            &tags(&[("v2", "c4"), ("v1", "c2")]),
            &HashSet::new(),
        );
        let mut added = members(&range, &placed);
        for step in &mut added {
            step.sort();
        }
        assert_eq!(added, [vec!["c1", "c2"], vec!["c3", "c4", "s1"]]);
    }

    #[test]
    fn bounded_keeps_order_and_reports_errors() {
        let items: Vec<usize> = (0..17).collect();
        assert_eq!(
            bounded(&items, |i| Ok(i * 2)).unwrap(),
            (0..17).map(|i| i * 2).collect::<Vec<_>>()
        );
        let err = bounded(
            &items,
            |i| {
                if *i == 5 { Err("five".into()) } else { Ok(*i) }
            },
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "five");
        assert!(bounded(&[] as &[usize], |i| Ok(*i)).unwrap().is_empty());
    }
}
