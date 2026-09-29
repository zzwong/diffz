//! Wiring for processes and explicit command-line input. Nothing writes remotely during startup.
use anyhow::{Context, Result, bail};
use diffz_adapters::{
    github::GithubTarget,
    gitlab::GitlabTarget,
    process::{read_bounded, resolve_program},
    service::{Services, default_state_dir, detect_source},
};
use diffz_core::{
    domain::ProviderId,
    provider::{Cancellation, OpenRequest, WorkbenchServices},
};
#[cfg(any(test, all(unix, feature = "desktop")))]
use std::path::Path;
use std::{
    io::{ErrorKind, Write},
    path::PathBuf,
    sync::Arc,
};
#[cfg(all(target_os = "linux", feature = "desktop"))]
mod code_font;

const HELP: &str = r#"diffz: review diffs and pull requests on your desktop

Usage:
  diffz owner/repo#123
  diffz group/project!123
  diffz https://github.com/owner/repo/pull/123
  diffz https://github.com/owner/repo/compare/v1.0...v2.0
  diffz /path/to/change.patch
  diffz --pr owner/repo#123
  diffz --mr group/project!123
  diffz --compare https://github.com/owner/repo/compare/v1.0...v2.0
  diffz --patch /path/to/change.patch
  diffz --git /repo --base main --head HEAD
  diffz --staged /repo
  diffz --worktree /repo
  diffz --fixture F01

Options:
  --pr OWNER/REPO#N         Open a pull request on GitHub, fetched by the installed gh
  --mr URL                  Open a merge request on GitLab, fetched by the installed glab
  --compare URL             Open a GitHub or GitLab compare of two refs, read-only, fetched by gh or glab
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
  --foreground              Run the window in this process and wait until it closes
  --json                    Report the handoff or launch as one JSON line on stdout
  --inspect                 Print the snapshot currently loaded as JSON and stop
  --doctor                  Check which external tools exist, then exit
  --probe FILE              Write a report on native text measurement; requires --probe-output
  --probe-output PATH       Destination for the JSON report written by --probe
  --version                 Show the version, then exit
  --help                    Show this help

A lone argument opens the patch file at that path when one exists; otherwise it is a
pull or merge request address, in any form --pr or --mr accepts.

diffz returns at once. A window already running on the same state directory takes the
request and comes to the front; otherwise a new window starts in the background. The
exit status says whether the request was taken. --help, --version, --doctor, --inspect,
and --probe never hand off or detach.

Run with no arguments and diffz shows the Open panel. Repositories are only read:
diffz will not check out, stage, or alter anything in them. All network access
happens inside gh or glab; writes additionally need an --allow-*-writes flag.
"#;
#[derive(Debug)]
struct Options {
    /// `None` when the command line named no source.
    request: Option<OpenRequest>,
    state: Option<PathBuf>,
    writes: bool,
    gitlab_writes: bool,
    inspect: bool,
    foreground: bool,
    json: bool,
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
    let mut positional = false;
    let mut foreground = false;
    let mut json = false;
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
            "--pr" => set_source(
                &mut source,
                OpenRequest::Remote {
                    provider: ProviderId::GITHUB,
                    address: next()?,
                },
            )?,
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
            "--mr" => set_source(
                &mut source,
                OpenRequest::Remote {
                    provider: ProviderId::GITLAB,
                    address: next()?,
                },
            )?,
            "--compare" => set_source(&mut source, compare_request(next()?)?)?,
            "--inspect" => inspect = true,
            "--foreground" => foreground = true,
            "--json" => json = true,
            "--font" => font = Some(next()?),
            "--theme" => theme = Some(next()?),
            "--probe" => probe = Some(PathBuf::from(next()?)),
            "--probe-output" => probe_output = Some(PathBuf::from(next()?)),
            _ if !arg.starts_with('-') => {
                positional = true;
                set_source(&mut source, OpenRequest::Patch(PathBuf::from(arg)))?
            }
            _ => bail!("unknown argument {arg:?}; use --help"),
        }
    }
    let mut request = match source {
        Some(OpenRequest::Patch(path)) if positional => Some(detect(path)?),
        source => source,
    };
    if let Some(OpenRequest::LocalGit {
        base: b, head: h, ..
    }) = &mut request
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
        foreground,
        json,
        font,
        theme,
        probe,
        probe_output,
    })
}
/// A lone argument is a patch when that path is a file, and otherwise the address of whichever
/// review provider's parser accepts it, passed on unchanged.
fn detect(path: PathBuf) -> Result<OpenRequest> {
    if let Some(input) = path.to_str()
        && let Some(detected) = detect_source(input)
    {
        return Ok(detected.request);
    }
    bail!(
        "{:?} is neither a file nor a review address; pass a patch file, owner/repo#N or a \
         GitHub pull request URL, group/project!N or a GitLab merge request URL, or a GitHub \
         or GitLab compare URL",
        path
    )
}
/// Each provider's own parser decides whether a URL is a compare of its own.
fn compare_request(address: String) -> Result<OpenRequest> {
    let (github, gitlab) = (GithubTarget::parse(&address), GitlabTarget::parse(&address));
    let provider = match (github, gitlab) {
        (Ok(GithubTarget::Compare(_)), _) => ProviderId::GITHUB,
        (_, Ok(GitlabTarget::Compare(_))) => ProviderId::GITLAB,
        (Ok(_), _) | (_, Ok(_)) => {
            bail!("{address:?} is a pull or merge request; use --pr or --mr")
        }
        (Err(github), Err(gitlab)) => {
            bail!("not a GitHub compare ({github}) or a GitLab compare ({gitlab})")
        }
    };
    Ok(OpenRequest::Remote { provider, address })
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
/// Writes `text` and a newline. A closed reader, as in `diffz --inspect | head`, ends output without an error.
fn print_to(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}
fn print(text: &str) -> Result<()> {
    print_to(&mut std::io::stdout().lock(), text).context("failed writing to stdout")
}
#[cfg(feature = "desktop")]
fn desktop_font(requested: Option<String>) -> Result<Option<String>> {
    #[cfg(target_os = "linux")]
    {
        let family = code_font::resolve(requested.as_deref()).map_err(anyhow::Error::msg)?;
        if let Some(requested) = requested.as_deref()
            && !requested.eq_ignore_ascii_case(&family)
        {
            eprintln!("diffz: code font {requested:?} resolved to {family:?}");
        }
        Ok(Some(family))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(requested)
    }
}
/// Makes paths in `request` independent of this process's working directory.
#[cfg(any(test, all(unix, feature = "desktop")))]
fn absolute(request: OpenRequest) -> Result<OpenRequest> {
    Ok(match request {
        OpenRequest::Patch(path) => OpenRequest::Patch(std::path::absolute(path)?),
        OpenRequest::LocalGit { root, base, head } => OpenRequest::LocalGit {
            root: std::path::absolute(root)?,
            base,
            head,
        },
        OpenRequest::LocalIndex(root) => OpenRequest::LocalIndex(std::path::absolute(root)?),
        OpenRequest::LocalWorktree(root) => OpenRequest::LocalWorktree(std::path::absolute(root)?),
        request => request,
    })
}
/// The arguments that start a foreground window on `request`, from any working directory.
#[cfg(any(test, all(unix, feature = "desktop")))]
fn launch_args(
    options: &Options,
    state: &Path,
    request: Option<&OpenRequest>,
) -> Result<Vec<std::ffi::OsString>> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--foreground".into(),
        "--state-dir".into(),
        std::path::absolute(state)?.into(),
    ];
    match request {
        None => {}
        Some(OpenRequest::Fixture(id)) => args.extend(["--fixture".into(), id.into()]),
        Some(OpenRequest::Patch(path)) => args.extend(["--patch".into(), path.into()]),
        Some(OpenRequest::Remote { provider, address }) => {
            let flag = if compare_request(address.clone()).is_ok() {
                "--compare"
            } else if *provider == ProviderId::GITHUB {
                "--pr"
            } else if *provider == ProviderId::GITLAB {
                "--mr"
            } else {
                bail!("no command-line flag opens a {provider} address")
            };
            args.extend([flag.into(), address.into()]);
        }
        Some(OpenRequest::LocalGit { root, base, head }) => args.extend([
            "--git".into(),
            root.into(),
            "--base".into(),
            base.into(),
            "--head".into(),
            head.into(),
        ]),
        Some(OpenRequest::LocalIndex(root)) => args.extend(["--staged".into(), root.into()]),
        Some(OpenRequest::LocalWorktree(root)) => args.extend(["--worktree".into(), root.into()]),
        Some(OpenRequest::Resume(id)) => args.extend(["--resume".into(), (&id.0).into()]),
    }
    if options.writes {
        args.push("--allow-github-writes".into());
    }
    if options.gitlab_writes {
        args.push("--allow-gitlab-writes".into());
    }
    if let Some(font) = &options.font {
        args.extend(["--font".into(), font.into()]);
    }
    if let Some(theme) = &options.theme {
        // A theme may be a relative path to its folder or colors.toml.
        let theme = Path::new(theme);
        let theme = match theme.exists() {
            true => std::path::absolute(theme)?,
            false => theme.to_path_buf(),
        };
        args.extend(["--theme".into(), theme.into()]);
    }
    Ok(args)
}
/// The `.app` bundle holding `exe`, which macOS should start through LaunchServices.
#[cfg(all(target_os = "macos", any(test, feature = "desktop")))]
fn app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then_some(bundle)
}
/// Starts a window in the background: through LaunchServices from an app bundle, so the Dock
/// shows the app, and otherwise as this executable in a session of its own. Either way its
/// stderr goes to the log in `state`.
#[cfg(all(unix, feature = "desktop"))]
fn launch(args: Vec<std::ffi::OsString>, state: &Path) -> Result<()> {
    use diffz_adapters::handoff::log_file;
    let exe = std::env::current_exe()?.canonicalize()?;
    let log = log_file(state)?;
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle(&exe) {
        use diffz_adapters::{
            handoff::log_path,
            process::{ProcessRequest, Runner, stderr_excerpt},
        };
        // -n: an app already running on another state directory must not swallow the request.
        // open appends the app's stderr to the log that log_file prepared.
        let open = ProcessRequest::new("/usr/bin/open".into()).args(
            [
                "-n".into(),
                "-a".into(),
                bundle.as_os_str().to_owned(),
                "--stderr".into(),
                log_path(state).into(),
                "--args".into(),
            ]
            .into_iter()
            .chain(args),
        );
        let output = Runner::run(open, Cancellation::default())?;
        if !output.status.success() {
            bail!(
                "LaunchServices could not start {}: {}",
                bundle.display(),
                stderr_excerpt(&output.stderr).unwrap_or_default()
            )
        }
        return Ok(());
    }
    diffz_adapters::process::spawn_detached(&exe, &args, log)?;
    Ok(())
}
#[cfg(all(unix, feature = "desktop"))]
fn writes(options: &Options) -> diffz_adapters::handoff::Writes {
    diffz_adapters::handoff::Writes {
        github: options.writes,
        gitlab: options.gitlab_writes,
    }
}
/// Gives the request to the window running on `state`, or starts one in the background. Either
/// way this returns once a window has the request, without waiting for it to close.
#[cfg(all(unix, feature = "desktop"))]
fn hand_off(options: &Options, state: &Path) -> Result<()> {
    use diffz_adapters::handoff::{Delivery, deliver};
    let request = handed_request(options)?;
    let result = deliver(state, request.as_ref(), writes(options), || {
        launch_args(options, state, request.as_ref())
            .and_then(|args| launch(args, state))
            .map_err(|e| format!("{e:#}").into())
    })
    .map(|delivery| match delivery {
        Delivery::HandedOff => "handed_off",
        Delivery::Launched => "launched",
    })
    .map_err(anyhow::Error::from);
    if matches!(result, Ok("handed_off")) && (options.font.is_some() || options.theme.is_some()) {
        eprintln!("diffz: the running window keeps the font and theme it started with");
    }
    match options.json {
        true => report(&mut std::io::stdout().lock(), result, request.as_ref()),
        false => result.map(|_| ()),
    }
}
/// The request a later invocation hands over; `None` when the command line named no source.
#[cfg(all(unix, feature = "desktop"))]
fn handed_request(options: &Options) -> Result<Option<OpenRequest>> {
    options.request.clone().map(absolute).transpose()
}
/// Prints the outcome of a handoff as one JSON line. The outcome alone sets the exit status: a
/// request the window took stays taken when stdout is gone.
#[cfg(any(test, all(unix, feature = "desktop")))]
fn report(out: &mut impl Write, result: Result<&str>, request: Option<&OpenRequest>) -> Result<()> {
    let line = match &result {
        Ok(status) => serde_json::json!({ "status": status, "source": request }),
        Err(e) => serde_json::json!({ "status": "error", "message": format!("{e:#}") }),
    };
    if let Err(e) = print_to(out, &line.to_string()) {
        eprintln!("diffz: could not write the status line: {e}");
    }
    result.map(|_| ())
}
/// Forwards requests from later invocations to the window, one at a time.
#[cfg(all(unix, feature = "desktop"))]
fn serve(
    listener: diffz_adapters::handoff::Listener,
) -> smol::channel::Receiver<diffz_ui::Handoff> {
    let (tx, rx) = smol::channel::unbounded();
    std::thread::spawn(move || {
        while let Ok(mut incoming) = listener.accept() {
            let handoff = diffz_ui::Handoff {
                request: incoming.request.take(),
                reply: Box::new(move |result| incoming.reply(result)),
            };
            if tx.send_blocking(handoff).is_err() {
                break;
            }
        }
    });
    rx
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
        return print(&text);
    }
    if args == ["--doctor"] {
        let mut report = format!(
            "OS={} ARCH={} desktop_feature={}\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(feature = "desktop")
        );
        for tool in ["git", "gh", "cargo", "rustc"] {
            report.push_str(&format!(
                "{tool}: {}\n",
                resolve_program(tool)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|e| format!("missing ({e})"))
            ));
        }
        let registry = diffz_core::registry::Registry::installed();
        report.push_str(&format!(
            "extension contract: {}.{} wasm_feature={}\n",
            diffz_core::extension::CONTRACT.0,
            diffz_core::extension::CONTRACT.1,
            cfg!(feature = "wasm")
        ));
        for ext in registry.extensions() {
            report.push_str(&format!(
                "extension {} {}: {} languages, {} themes, {} annotators ({})\n",
                ext.id,
                ext.version,
                ext.languages.len(),
                ext.themes.len(),
                ext.annotators.len(),
                ext.dir.display()
            ));
        }
        for problem in registry
            .problems()
            .iter()
            .chain(&registry.check_extensions())
        {
            report.push_str(&format!("extension problem: {problem}\n"));
        }
        report.push_str("This report implies no native runtime and no credential capability.");
        return print(&report);
    }
    let json = args.iter().any(|a| a == "--json");
    let mut options = parse(args).inspect_err(|e| {
        // An agent reading --json output gets unusable input reported the same way.
        if json {
            let line = serde_json::json!({ "status": "error", "message": format!("{e:#}") });
            let _ = print(&line.to_string());
        }
    })?;
    #[cfg(not(feature = "desktop"))]
    let _ = &options.font;
    #[cfg(not(all(unix, feature = "desktop")))]
    let _ = (options.foreground, options.json);
    let _ = &options.theme;
    if let (Some(source), Some(output)) = (options.probe.take(), options.probe_output.take()) {
        let source = String::from_utf8(read_bounded(&source, 1024 * 1024)?)
            .context("probe source is not UTF-8")?;
        #[cfg(feature = "desktop")]
        {
            diffz_ui::probe::launch_probe(source, desktop_font(options.font)?, output);
            return Ok(());
        }
        #[cfg(not(feature = "desktop"))]
        {
            let _ = (source, output);
            bail!("native probe requires --features desktop")
        }
    }
    let state = match options.state.clone() {
        Some(path) => path,
        None => default_state_dir()?,
    };
    // A launch from the Dock or Finder has launchd as its parent and is the window itself.
    // Linux desktop launchers start diffz like a terminal does, so it hands off or detaches.
    #[cfg(all(unix, feature = "desktop"))]
    if !options.inspect && !options.foreground && std::os::unix::process::parent_id() != 1 {
        return hand_off(&options, &state);
    }
    let open = || Services::new_with_providers(&state, options.writes, options.gitlab_writes);
    // Another window can take the state first when two start at once; it gets the request.
    #[cfg(all(unix, feature = "desktop"))]
    let services = match options.inspect {
        true => open()?,
        false => match diffz_adapters::handoff::open_or_hand_off(
            &state,
            handed_request(&options)?.as_ref(),
            writes(&options),
            open,
        )? {
            Some(services) => services,
            None => {
                eprintln!("diffz: handed the request to the diffz already running");
                return Ok(());
            }
        },
    };
    #[cfg(not(all(unix, feature = "desktop")))]
    let services = open()?;
    let services: Arc<dyn WorkbenchServices> = Arc::new(services);
    diffz_core::timing::mark("services");
    if options.inspect {
        let request = options
            .request
            .context("--inspect needs a source; use --help")?;
        let opened = services.open(request, Cancellation::default())?;
        return print(&serde_json::to_string_pretty(&opened.snapshot)?);
    }
    #[cfg(feature = "desktop")]
    let registry = diffz_core::registry::Registry::installed();
    #[cfg(feature = "desktop")]
    for problem in registry.problems() {
        eprintln!("diffz: extension skipped: {problem}");
    }
    #[cfg(all(unix, feature = "desktop"))]
    let handoffs = match diffz_adapters::handoff::Listener::bind(&state, writes(&options)) {
        Ok(listener) => Some(serve(listener)),
        Err(e) => {
            eprintln!("diffz: later invocations cannot hand requests to this window: {e}");
            None
        }
    };
    #[cfg(all(not(unix), feature = "desktop"))]
    let handoffs = None;
    #[cfg(feature = "desktop")]
    {
        if diffz_ui::launch(
            services,
            diffz_ui::LaunchOptions {
                initial: options.request,
                registry: std::sync::Arc::new(registry),
                font_family: desktop_font(options.font)?,
                theme: options.theme,
                handoffs,
            },
        ) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("no native window could be created"))
        }
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
    fn no_arguments_name_no_source() {
        assert!(parse(Vec::<String>::new()).unwrap().request.is_none());
    }
    #[test]
    fn a_fixture_is_only_opened_when_named() {
        assert_eq!(
            parse(["--fixture", "F01"].map(str::to_string))
                .unwrap()
                .request,
            Some(OpenRequest::Fixture("F01".into()))
        );
    }
    /// An existing patch file whose path contains a space.
    fn patch_file() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review files");
        std::fs::create_dir(&path).unwrap();
        let path = path.join("change.patch");
        std::fs::write(&path, "").unwrap();
        (dir, path.to_str().unwrap().to_string())
    }
    #[test]
    fn positional_patch_preserves_the_file_path() {
        let (_dir, path) = patch_file();
        assert!(matches!(
            parse([path.clone(), "--inspect".into()]).unwrap().request,
            Some(OpenRequest::Patch(p)) if p == std::path::Path::new(&path)
        ));
    }
    #[test]
    fn positional_review_addresses_pass_through_unchanged() {
        for (address, provider) in [
            ("owner/repo#123", ProviderId::GITHUB),
            ("https://github.com/owner/repo/pull/123", ProviderId::GITHUB),
            ("group/project!123", ProviderId::GITLAB),
            (
                "https://gitlab.com/group/sub/project/-/merge_requests/123/diffs",
                ProviderId::GITLAB,
            ),
            (
                "https://github.com/owner/repo/compare/v1.0...v2.0",
                ProviderId::GITHUB,
            ),
            (
                "https://gitlab.com/group/sub/project/-/compare/v1.0...v2.0",
                ProviderId::GITLAB,
            ),
        ] {
            let options = parse([address.to_string()]).unwrap();
            assert_eq!(
                options.request,
                Some(OpenRequest::Remote {
                    provider,
                    address: address.into()
                })
            );
        }
    }
    #[test]
    fn unrecognized_positional_lists_what_is_accepted() {
        let error = parse(["no/such/change.patch".to_string()])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("neither a file nor a review address"),
            "{error}"
        );
        assert!(error.contains("owner/repo#N") && error.contains("group/project!N"));
    }
    #[test]
    fn positional_address_conflicts_with_other_sources() {
        for args in [
            vec!["owner/repo#1", "--fixture", "F01"],
            vec!["--mr", "group/project!1", "owner/repo#1"],
        ] {
            let error = parse(args.into_iter().map(str::to_string)).unwrap_err();
            assert_eq!(error.to_string(), "pass a single source per launch");
        }
    }
    #[test]
    fn launch_flags_are_parsed() {
        let options = parse(["--foreground", "--json"].map(str::to_string)).unwrap();
        assert!(options.foreground && options.json && options.request.is_none());
        let options = parse(Vec::<String>::new()).unwrap();
        assert!(!options.foreground && !options.json);
    }
    #[test]
    fn launched_windows_get_absolute_paths_and_run_in_the_foreground() {
        let options =
            parse(["--git", "repo", "--base", "v1", "--allow-gitlab-writes"].map(str::to_string))
                .unwrap();
        let request = absolute(options.request.clone().unwrap()).unwrap();
        let args = launch_args(&options, Path::new("state"), Some(&request)).unwrap();
        let cwd = std::env::current_dir().unwrap();
        let expected: Vec<std::ffi::OsString> = vec![
            "--foreground".into(),
            "--state-dir".into(),
            cwd.join("state").into(),
            "--git".into(),
            cwd.join("repo").into(),
            "--base".into(),
            "v1".into(),
            "--head".into(),
            "HEAD".into(),
            "--allow-gitlab-writes".into(),
        ];
        assert_eq!(args, expected);
        let options = parse(["group/project!4".to_string()]).unwrap();
        let args = launch_args(&options, Path::new("/state"), options.request.as_ref()).unwrap();
        assert_eq!(
            args[3..],
            ["--mr", "group/project!4"].map(std::ffi::OsString::from)
        );
        let compare = "https://gitlab.com/g/p/-/compare/v1...v2";
        let options = parse([compare.to_string()]).unwrap();
        let args = launch_args(&options, Path::new("/state"), options.request.as_ref()).unwrap();
        assert_eq!(
            args[3..],
            ["--compare", compare].map(std::ffi::OsString::from)
        );
        let args = launch_args(&options, Path::new("/state"), None).unwrap();
        assert_eq!(args.len(), 3);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn app_bundles_are_found_from_their_executable() {
        assert_eq!(
            app_bundle(Path::new("/Applications/Diffz.app/Contents/MacOS/diffz")),
            Some(Path::new("/Applications/Diffz.app"))
        );
        assert_eq!(app_bundle(Path::new("/repo/target/debug/diffz")), None);
        assert_eq!(app_bundle(Path::new("/opt/MacOS/diffz")), None);
    }
    #[test]
    fn positional_patch_conflicts_with_other_sources() {
        for args in [
            vec!["one.patch", "two.patch"],
            vec!["one.patch", "--fixture", "F01"],
            vec!["--fixture", "F01", "one.patch"],
        ] {
            let error = parse(args.into_iter().map(str::to_string)).unwrap_err();
            assert_eq!(error.to_string(), "pass a single source per launch");
        }
    }
    #[test]
    fn desktop_entry_launches_with_and_without_a_file() {
        let desktop = include_str!("../../../packaging/linux/io.github.zzwong.Diffz.desktop");
        let exec = desktop
            .lines()
            .find_map(|line| line.strip_prefix("Exec="))
            .unwrap();
        // This entry uses unquoted tokens; the file field expands to one argument,
        // even when its path contains spaces, and disappears for a menu launch.
        let (_dir, path) = patch_file();
        for file in [None, Some(path.as_str())] {
            let args = exec.split_whitespace().skip(1).filter_map(|arg| {
                if arg == "%f" {
                    file.map(str::to_string)
                } else {
                    Some(arg.to_string())
                }
            });
            let request = parse(args).unwrap().request;
            match file {
                Some(path) => {
                    assert!(
                        matches!(request, Some(OpenRequest::Patch(p)) if p == std::path::Path::new(path))
                    )
                }
                None => assert!(request.is_none()),
            }
        }
    }
    #[test]
    fn explicit_patch_requires_a_value() {
        assert!(parse(["--patch".into()]).is_err());
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
    fn compare_url_picks_its_provider() {
        for (url, expected) in [
            ("https://github.com/o/r/compare/v1...v2", ProviderId::GITHUB),
            (
                "https://gitlab.com/g/p/-/compare/v1...v2",
                ProviderId::GITLAB,
            ),
            (
                "https://gitlab.com/g/p/-/compare?from=v1&to=v2",
                ProviderId::GITLAB,
            ),
        ] {
            let request = parse(["--compare", url].map(str::to_string))
                .unwrap()
                .request;
            assert_eq!(
                request,
                Some(OpenRequest::Remote {
                    provider: expected,
                    address: url.into()
                })
            );
        }
    }
    #[test]
    fn compare_rejects_other_addresses() {
        for url in [
            "https://github.com/o/r/pull/1",
            "https://gitlab.com/g/p/-/merge_requests/1",
            "o/r#1",
            "https://example.com/",
        ] {
            assert!(
                parse(["--compare", url].map(str::to_string)).is_err(),
                "{url}"
            );
        }
        assert!(parse(["--compare".to_string()]).is_err());
    }
    #[test]
    fn unknown_flags_are_rejected() {
        assert!(parse(["--chat-claude".into()]).is_err());
    }
    struct FailingWriter(ErrorKind);
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn an_unwritable_status_line_keeps_the_exit_status() {
        let mut out = FailingWriter(ErrorKind::StorageFull);
        assert!(report(&mut out, Ok("launched"), None).is_ok());
        let refused = report(&mut out, Err(anyhow::anyhow!("refused")), None).unwrap_err();
        assert_eq!(refused.to_string(), "refused");
        let mut out = Vec::new();
        let request = OpenRequest::Fixture("F01".into());
        report(&mut out, Ok("handed_off"), Some(&request)).unwrap();
        let line: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(line["status"], "handed_off");
        assert!(line["source"].is_object());
    }
    #[test]
    fn closed_stdout_ends_output_cleanly() {
        assert!(print_to(&mut FailingWriter(ErrorKind::BrokenPipe), "{}").is_ok());
    }
    #[test]
    fn other_stdout_errors_are_reported() {
        let error = print_to(&mut FailingWriter(ErrorKind::StorageFull), "{}").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::StorageFull);
    }
    #[test]
    fn printed_text_ends_with_a_newline() {
        let mut out = Vec::new();
        print_to(&mut out, "{}").unwrap();
        assert_eq!(out, b"{}\n");
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
