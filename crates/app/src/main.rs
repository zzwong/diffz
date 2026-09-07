//! Wiring for processes and explicit command-line input. Nothing writes remotely during startup.
use anyhow::{Context, Result, bail};
use diffz_adapters::{
    process::{read_bounded, resolve_program},
    service::{Services, default_state_dir},
};
use diffz_core::provider::{Cancellation, OpenRequest, WorkbenchServices};
use std::{path::PathBuf, sync::Arc};
const HELP: &str = r#"diffz: review diffs and pull requests on your desktop

Usage:
  diffz --pr owner/repo#123
  diffz --mr group/project!123
  diffz --patch /path/to/change.patch
  diffz --git /repo --base main --head HEAD
  diffz --staged /repo
  diffz --worktree /repo
  diffz --fixture F01

Options:
  --pr OWNER/REPO#N         Open a pull request on GitHub, fetched by the installed gh
  --mr URL                  Open a merge request on GitLab, fetched by the installed glab
  --patch FILE              Show a unified diff file
  --git REPO                Diff --base (default main) against --head (default HEAD)
  --staged REPO             Compare the index with HEAD
  --worktree REPO           Show changes that are unstaged or untracked
  --fixture ID              Load an offline fixture bundled with the binary
  --resume SNAPSHOT_ID      Bring back a saved review, drafts included
  --state-dir PATH          Store state in a different directory (SQLite)
  --font FAMILY             Family used for monospace source text
  --theme REF               "current" (Omarchy), a theme name, or a path to one theme's colors.toml or its folder
  --allow-github-writes     Let reviews be published to GitHub after a preview
  --allow-gitlab-writes     Let comments and approvals go to GitLab after a preview
  --inspect                 Print the snapshot currently loaded as JSON and stop
  --doctor                  Check which external tools exist, then exit
  --probe FILE              Write a report on native text measurement; requires --probe-output
  --probe-output PATH       Destination for the JSON report written by --probe
  --version                 Show the version, then exit
  --help                    Show this help

Run with no arguments and diffz shows the fixture named F01. Repositories are only read:
diffz will not check out, stage, or alter anything in them. All network access
happens inside gh or glab; writes additionally need an --allow-*-writes flag.
"#;
#[derive(Debug)]
struct Options {
    request: OpenRequest,
    state: Option<PathBuf>,
    writes: bool,
    gitlab_writes: bool,
    inspect: bool,
    font: Option<String>,
    theme: Option<String>,
    probe: Option<PathBuf>,
    probe_output: Option<PathBuf>,
}
fn parse(args: impl IntoIterator<Item = String>) -> Result<Options> {
    let mut it = args.into_iter();
    let mut source = None;
    let mut state = None;
    let mut writes = false;
    let mut gitlab_writes = false;
    let mut inspect = false;
    let mut font = None;
    let mut theme = None;
    let mut base = "main".to_string();
    let mut head = "HEAD".to_string();
    let mut probe = None;
    let mut probe_output = None;
    while let Some(arg) = it.next() {
        let mut next = || {
            it.next()
                .with_context(|| format!("missing value after {arg}"))
        };
        match arg.as_str() {
            "--fixture" => set_source(&mut source, OpenRequest::Fixture(next()?))?,
            "--patch" => set_source(&mut source, OpenRequest::Patch(PathBuf::from(next()?)))?,
            "--pr" => set_source(&mut source, OpenRequest::GitHub(next()?))?,
            "--git" => set_source(
                &mut source,
                OpenRequest::LocalGit {
                    root: PathBuf::from(next()?),
                    base: String::new(),
                    head: String::new(),
                },
            )?,
            "--staged" => set_source(&mut source, OpenRequest::LocalIndex(PathBuf::from(next()?)))?,
            "--worktree" => set_source(
                &mut source,
                OpenRequest::LocalWorktree(PathBuf::from(next()?)),
            )?,
            "--resume" => set_source(
                &mut source,
                OpenRequest::Resume(diffz_core::domain::SnapshotId(next()?)),
            )?,
            "--base" => base = next()?,
            "--head" => head = next()?,
            "--state-dir" => state = Some(PathBuf::from(next()?)),
            "--allow-github-writes" => writes = true,
            "--allow-gitlab-writes" => gitlab_writes = true,
            "--mr" => set_source(&mut source, OpenRequest::GitLab(next()?))?,
            "--inspect" => inspect = true,
            "--font" => font = Some(next()?),
            "--theme" => theme = Some(next()?),
            "--probe" => probe = Some(PathBuf::from(next()?)),
            "--probe-output" => probe_output = Some(PathBuf::from(next()?)),
            _ => bail!("unknown argument {arg:?}; use --help"),
        }
    }
    let mut request = source.unwrap_or(OpenRequest::Fixture("F01".into()));
    if let OpenRequest::LocalGit {
        base: b, head: h, ..
    } = &mut request
    {
        *b = base;
        *h = head;
    }
    if probe.is_some() != probe_output.is_some() {
        bail!("pass --probe together with --probe-output")
    }
    Ok(Options {
        request,
        state,
        writes,
        gitlab_writes,
        inspect,
        font,
        theme,
        probe,
        probe_output,
    })
}
fn set_source(source: &mut Option<OpenRequest>, request: OpenRequest) -> Result<()> {
    if source.is_some() {
        bail!("pass a single source per launch")
    };
    *source = Some(request);
    Ok(())
}
fn early_exit(args: &[String]) -> Option<String> {
    if args.iter().any(|s| s == "--help" || s == "-h") {
        Some(HELP.to_string())
    } else if args.iter().any(|s| s == "--version" || s == "-V") {
        Some(format!("diffz {}", env!("CARGO_PKG_VERSION")))
    } else {
        None
    }
}
fn main() {
    diffz_core::timing::mark("main");
    if let Err(e) = run() {
        eprintln!("diffz: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(text) = early_exit(&args) {
        println!("{text}");
        return Ok(());
    }
    if args == ["--doctor"] {
        println!(
            "OS={} ARCH={} desktop_feature={}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(feature = "desktop")
        );
        for tool in ["git", "gh", "cargo", "rustc"] {
            println!(
                "{tool}: {}",
                resolve_program(tool)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|e| format!("missing ({e})"))
            );
        }
        println!("This report implies no native runtime and no credential capability.");
        return Ok(());
    }
    let options = parse(args)?;
    #[cfg(not(feature = "desktop"))]
    let _ = &options.font;
    let _ = &options.theme;
    if let (Some(source), Some(output)) = (options.probe, options.probe_output) {
        let source = String::from_utf8(read_bounded(&source, 1024 * 1024)?)
            .context("probe source is not UTF-8")?;
        #[cfg(feature = "desktop")]
        {
            diffz_ui::probe::launch_probe(source, options.font, output);
            return Ok(());
        }
        #[cfg(not(feature = "desktop"))]
        {
            let _ = (source, output);
            bail!("native probe requires --features desktop")
        }
    }
    let state = match options.state {
        Some(path) => path,
        None => default_state_dir()?,
    };
    let services: Arc<dyn WorkbenchServices> = Arc::new(Services::new_with_providers(
        &state,
        options.writes,
        options.gitlab_writes,
    )?);
    diffz_core::timing::mark("services");
    if options.inspect {
        let opened = services.open(options.request, Cancellation::default())?;
        println!("{}", serde_json::to_string_pretty(&opened.snapshot)?);
        return Ok(());
    }
    #[cfg(feature = "desktop")]
    {
        diffz_ui::launch(
            services,
            diffz_ui::LaunchOptions {
                initial: options.request,
                font_family: options.font,
                theme: options.theme,
            },
        );
        Ok(())
    }
    #[cfg(not(feature = "desktop"))]
    {
        bail!(
            "this build lacks the desktop feature; run --inspect, or compile with --features desktop"
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_offline() {
        assert!(matches!(
            parse(Vec::<String>::new()).unwrap().request,
            OpenRequest::Fixture(_)
        ));
    }
    #[test]
    fn no_implicit_write_permission() {
        assert!(!parse(["--pr".into(), "o/r#1".into()]).unwrap().writes);
    }
    #[test]
    fn conflicting_sources_are_rejected() {
        assert!(parse(["--pr", "o/r#1", "--fixture", "F01"].map(str::to_string)).is_err());
    }
    #[test]
    fn unknown_flags_are_rejected() {
        assert!(parse(["--chat-claude".into()]).is_err());
    }
    #[test]
    fn version_is_handled_before_parse() {
        let expected = format!("diffz {}", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            early_exit(&["--version".to_string()]).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            early_exit(&["-V".to_string()]).as_deref(),
            Some(expected.as_str())
        );
        assert!(early_exit(&[]).is_none());
    }
}
