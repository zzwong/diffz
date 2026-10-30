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
    ffi::OsString,
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
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options> {
    parse_with_detect(args, detect)
}
/// Keep argument parsing independent of positional-source filesystem detection.
fn parse_with_detect(
    args: impl IntoIterator<Item = OsString>,
    detect: impl FnOnce(PathBuf) -> Result<OpenRequest>,
) -> Result<Options> {
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
        // Only option names are text. Filesystem operands stay in their native encoding.
        let flag = arg.to_str().unwrap_or("");
        let mut next = || {
            it.next()
                .with_context(|| format!("missing value after {flag}"))
        };
        match flag {
            "--fixture" => set_source(
                &mut source,
                OpenRequest::Fixture(text_argument(next()?, flag)?),
            )?,
            "--patch" => set_source(&mut source, OpenRequest::Patch(PathBuf::from(next()?)))?,
            "--pr" => set_source(
                &mut source,
                OpenRequest::Remote {
                    provider: ProviderId::GITHUB,
                    address: text_argument(next()?, flag)?,
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
                OpenRequest::Resume(diffz_core::domain::SnapshotId(text_argument(
                    next()?,
                    flag,
                )?)),
            )?,
            "--base" => base = text_argument(next()?, flag)?,
            "--head" => head = text_argument(next()?, flag)?,
            "--state-dir" => state = Some(PathBuf::from(next()?)),
            "--allow-github-writes" => writes = true,
            "--allow-gitlab-writes" => gitlab_writes = true,
            "--mr" => set_source(
                &mut source,
                OpenRequest::Remote {
                    provider: ProviderId::GITLAB,
                    address: text_argument(next()?, flag)?,
                },
            )?,
            "--compare" => {
                set_source(&mut source, compare_request(text_argument(next()?, flag)?)?)?
            }
            "--inspect" => inspect = true,
            "--foreground" => foreground = true,
            "--json" => json = true,
            "--font" => font = Some(text_argument(next()?, flag)?),
            // Theme selection is a textual UI contract, even when it names a path.
            "--theme" => theme = Some(text_argument(next()?, flag)?),
            "--probe" => probe = Some(PathBuf::from(next()?)),
            "--probe-output" => probe_output = Some(PathBuf::from(next()?)),
            _ if !arg.as_encoded_bytes().starts_with(b"-") => {
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
    // Fail before launching or handing off a source that the JSON report cannot encode.
    if json {
        json_source(request.as_ref())?;
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
/// Textual operands cannot be repaired lossily: that could select a different ref or source.
fn text_argument(value: OsString, flag: &str) -> Result<String> {
    value
        .into_string()
        .map_err(|_| anyhow::anyhow!("value after {flag} must be valid UTF-8"))
}
/// A lone argument is a patch when that path is a file, and otherwise the address of whichever
/// review provider's parser accepts it, passed on unchanged.
fn detect(path: PathBuf) -> Result<OpenRequest> {
    // CLI arguments are already delimited: whitespace belongs to the filename.
    // Native paths are checked before the Open panel's free-text normalization.
    if path.is_file() {
        return Ok(OpenRequest::Patch(std::path::absolute(path)?));
    }
    if let Some(input) = path.to_str()
        && let Some(detected) = detect_source(input)
        && matches!(detected.request, OpenRequest::Remote { .. })
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
fn early_exit(args: &[OsString]) -> Option<String> {
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
fn error_line(error: &anyhow::Error) -> serde_json::Value {
    serde_json::json!({ "status": "error", "message": format!("{error:#}") })
}
fn json_source(request: Option<&OpenRequest>) -> Result<serde_json::Value> {
    serde_json::to_value(request).context("source cannot be represented in JSON; use a UTF-8 path")
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
fn launch(
    args: Vec<std::ffi::OsString>,
    state: &Path,
    activation_token: Option<String>,
) -> Result<()> {
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
    diffz_adapters::process::spawn_detached(&exe, &args, log, activation_token)?;
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
fn hand_off(options: &Options, state: &Path, activation_token: Option<String>) -> Result<()> {
    use diffz_adapters::handoff::{Delivery, deliver};
    let request = handed_request(options).and_then(|request| {
        // A Unicode relative path can become non-UTF-8 in a native working directory.
        if options.json {
            json_source(request.as_ref())?;
        }
        Ok(request)
    });
    let request = match request {
        Ok(request) => request,
        Err(error) if options.json => {
            return report(&mut std::io::stdout().lock(), Err(error), None);
        }
        Err(error) => return Err(error),
    };
    let result = deliver(
        state,
        request.as_ref(),
        writes(options),
        activation_token,
        |token| {
            launch_args(options, state, request.as_ref())
                .and_then(|args| launch(args, state, token))
                .map_err(|e| format!("{e:#}").into())
        },
    )
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
/// Prints the outcome of a handoff as one JSON line. Sources are checked before dispatch;
/// output I/O never changes the outcome, so a taken request stays taken when stdout is gone.
#[cfg(any(test, all(unix, feature = "desktop")))]
fn report(out: &mut impl Write, result: Result<&str>, request: Option<&OpenRequest>) -> Result<()> {
    // Keep this fallible even when called without the parser's preflight.
    let result = result.and_then(|status| json_source(request).map(|source| (status, source)));
    let line = match &result {
        Ok((status, source)) => serde_json::json!({ "status": status, "source": source }),
        Err(e) => error_line(e),
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
    let (tx, rx) = smol::channel::bounded(1);
    std::thread::spawn(move || {
        while let Ok(mut incoming) = listener.accept() {
            let handoff = diffz_ui::Handoff {
                request: incoming.request.take(),
                activation_token: incoming.activation_token.take(),
                accepted_at: incoming.accepted_at,
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
    // SAFETY: this is the first operation in main, before profiling, GPUI or
    // our worker threads. Never move process-environment mutation into serve.
    let activation_token = unsafe { capture_activation_token() };
    diffz_core::timing::mark("main");
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    start_profile_heap_samples();
    if let Err(e) = run(activation_token) {
        eprintln!("diffz: {e:#}");
        std::process::exit(1);
    }
}

/// glibc's allocator totals, sampled only for an explicit Linux profile run.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn start_profile_heap_samples() {
    #[repr(C)]
    struct Mallinfo2 {
        arena: usize,
        ordblks: usize,
        smblks: usize,
        hblks: usize,
        hblkhd: usize,
        usmblks: usize,
        fsmblks: usize,
        uordblks: usize,
        fordblks: usize,
        keepcost: usize,
    }
    unsafe extern "C" {
        fn mallinfo2() -> Mallinfo2;
    }
    let Some(path) = std::env::var_os("DIFFZ_PROFILE_HEAP") else {
        return;
    };
    std::thread::spawn(move || {
        let Ok(mut file) = std::fs::File::create(path) else {
            return;
        };
        loop {
            // SAFETY: mallinfo2 reads glibc's process-wide allocator counters.
            let info = unsafe { mallinfo2() };
            let row = serde_json::json!({
                "arena": info.arena,
                "hblkhd": info.hblkhd,
                "uordblks": info.uordblks,
                "fordblks": info.fordblks,
            });
            if writeln!(file, "{row}").is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}
/// Must run at the single-threaded process entry point.
unsafe fn capture_activation_token() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let value = std::env::var_os("XDG_ACTIVATION_TOKEN");
        // SAFETY: caller guarantees no threads or environment readers exist yet.
        unsafe { std::env::remove_var("XDG_ACTIVATION_TOKEN") };
        captured_activation_token(value)
    }
    #[cfg(not(target_os = "linux"))]
    None
}
#[cfg(target_os = "linux")]
fn captured_activation_token(value: Option<std::ffi::OsString>) -> Option<String> {
    value?
        .into_string()
        .ok()
        .and_then(diffz_adapters::handoff::admit_activation_token)
}
fn run(activation_token: Option<String>) -> Result<()> {
    #[cfg(feature = "desktop")]
    let mut activation_token = activation_token;
    #[cfg(not(feature = "desktop"))]
    let _ = activation_token;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
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
            let _ = print(&error_line(e).to_string());
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
            diffz_ui::probe::launch_probe(
                source,
                desktop_font(options.font)?,
                output,
                activation_token,
            );
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
        return hand_off(&options, &state, activation_token);
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
            &mut activation_token,
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
        diffz_core::timing::mark("snapshot opened");
        // The window reads releases after the compare shows; here the output waits for them.
        let snapshot = services.releases(&opened.snapshot, Cancellation::default())?;
        diffz_core::timing::mark("releases");
        return print(&serde_json::to_string_pretty(&snapshot)?);
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
                initial_activation_token: activation_token,
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
    /// New argument-boundary fixtures never inspect paths or open sources.
    fn parse_pure(args: impl IntoIterator<Item = OsString>) -> Result<Options> {
        parse_with_detect(args, |_| panic!("unexpected positional detection"))
    }
    #[cfg(unix)]
    fn non_unicode_argument() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"native-\xff-name".to_vec())
    }
    #[cfg(windows)]
    fn non_unicode_argument() -> OsString {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[b'n' as u16, 0xd800, b'e' as u16])
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn native_source_path_operands_are_lossless() {
        let path = non_unicode_argument();
        for flag in ["--patch", "--git", "--staged", "--worktree"] {
            let options = parse_pure([flag.into(), path.clone()]).unwrap();
            let expected = match flag {
                "--patch" => OpenRequest::Patch(PathBuf::from(&path)),
                "--git" => OpenRequest::LocalGit {
                    root: PathBuf::from(&path),
                    base: "main".into(),
                    head: "HEAD".into(),
                },
                "--staged" => OpenRequest::LocalIndex(PathBuf::from(&path)),
                "--worktree" => OpenRequest::LocalWorktree(PathBuf::from(&path)),
                _ => unreachable!(),
            };
            assert_eq!(options.request, Some(expected), "{flag}");
        }
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn native_state_and_probe_paths_are_lossless() {
        let mut state = non_unicode_argument();
        state.push("-state");
        let mut source = non_unicode_argument();
        source.push("-source");
        let mut output = non_unicode_argument();
        output.push("-output");
        let options = parse_pure([
            "--state-dir".into(),
            state.clone(),
            "--probe".into(),
            source.clone(),
            "--probe-output".into(),
            output.clone(),
            "--json".into(),
        ])
        .unwrap();
        assert_eq!(options.state, Some(PathBuf::from(state)));
        assert_eq!(options.probe, Some(PathBuf::from(source)));
        assert_eq!(options.probe_output, Some(PathBuf::from(output)));
        assert!(options.json);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn native_positional_path_reaches_detection_losslessly() {
        let path = non_unicode_argument();
        let options = parse_with_detect([path.clone(), "--inspect".into()], |received| {
            assert_eq!(received.as_os_str(), path.as_os_str());
            Ok(OpenRequest::Patch(received))
        })
        .unwrap();
        assert_eq!(options.request, Some(OpenRequest::Patch(path.into())));
        assert!(options.inspect);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn json_rejects_unrepresentable_source_paths_before_dispatch() {
        for flag in ["--patch", "--git", "--staged", "--worktree"] {
            let error =
                parse_pure([flag.into(), non_unicode_argument(), "--json".into()]).unwrap_err();
            assert_eq!(
                error.to_string(),
                "source cannot be represented in JSON; use a UTF-8 path"
            );
        }
        let error = parse_with_detect([non_unicode_argument(), "--json".into()], |path| {
            Ok(OpenRequest::Patch(path))
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "source cannot be represented in JSON; use a UTF-8 path"
        );
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn json_report_serialization_failure_is_a_structured_error() {
        let request = OpenRequest::Patch(non_unicode_argument().into());
        let mut out = Vec::new();
        let error = report(&mut out, Ok("launched"), Some(&request)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "source cannot be represented in JSON; use a UTF-8 path"
        );
        let line: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(line["status"], "error");
        assert!(
            line["message"]
                .as_str()
                .unwrap()
                .contains(&error.to_string())
        );
        assert!(line.get("source").is_none());
        assert_eq!(out.iter().filter(|&&byte| byte == b'\n').count(), 1);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn non_unicode_text_values_are_rejected_with_the_flag() {
        for flag in [
            "--fixture",
            "--pr",
            "--mr",
            "--compare",
            "--resume",
            "--base",
            "--head",
            "--font",
            "--theme",
        ] {
            let error = parse_pure([flag.into(), non_unicode_argument()]).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("value after {flag} must be valid UTF-8")
            );
        }
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn non_unicode_option_names_are_not_positional_paths() {
        let mut flag = OsString::from("--");
        flag.push(non_unicode_argument());
        let error = parse_pure([flag.clone()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("unknown argument {flag:?}; use --help")
        );
    }
    #[test]
    fn unicode_paths_and_text_remain_unchanged() {
        let path = "répertoire/変更";
        let options = parse_pure(
            [
                "--git",
                path,
                "--base",
                "版本一",
                "--head",
                "révision",
                "--font",
                "字体",
                "--theme",
                "thèmes/夜/colors.toml",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert_eq!(
            options.request,
            Some(OpenRequest::LocalGit {
                root: path.into(),
                base: "版本一".into(),
                head: "révision".into(),
            })
        );
        assert_eq!(options.font.as_deref(), Some("字体"));
        assert_eq!(options.theme.as_deref(), Some("thèmes/夜/colors.toml"));
    }
    #[test]
    fn unknown_unicode_flags_keep_the_existing_diagnostic() {
        let error = parse_pure(["--未知".into()]).unwrap_err();
        assert_eq!(error.to_string(), "unknown argument \"--未知\"; use --help");
    }
    #[test]
    fn all_value_flags_keep_the_missing_value_diagnostic() {
        for flag in [
            "--fixture",
            "--patch",
            "--pr",
            "--git",
            "--staged",
            "--worktree",
            "--resume",
            "--base",
            "--head",
            "--state-dir",
            "--mr",
            "--compare",
            "--font",
            "--theme",
            "--probe",
            "--probe-output",
        ] {
            let error = parse_pure([flag.into()]).unwrap_err();
            assert_eq!(error.to_string(), format!("missing value after {flag}"));
        }
    }
    #[test]
    fn flag_shaped_path_operands_are_still_values() {
        let options = parse_pure(["--patch", "--json"].map(OsString::from)).unwrap();
        assert_eq!(options.request, Some(OpenRequest::Patch("--json".into())));
        assert!(!options.json);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn early_exits_do_not_decode_other_os_arguments() {
        for flag in ["--help", "-h"] {
            assert_eq!(
                early_exit(&[non_unicode_argument(), flag.into()]).as_deref(),
                Some(HELP)
            );
        }
        let version = format!("diffz {}", env!("CARGO_PKG_VERSION"));
        for flag in ["--version", "-V"] {
            assert_eq!(
                early_exit(&[non_unicode_argument(), flag.into()]).as_deref(),
                Some(version.as_str())
            );
        }
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn non_unicode_text_errors_have_structured_json_diagnostics() {
        let mut bad_flag = OsString::from("--");
        bad_flag.push(non_unicode_argument());
        for args in [
            vec!["--font".into(), non_unicode_argument(), "--json".into()],
            vec![bad_flag, "--json".into()],
        ] {
            let error = parse_pure(args).unwrap_err();
            let expected = error.to_string();
            let mut out = Vec::new();
            let returned = report(&mut out, Err(error), None).unwrap_err();
            assert_eq!(returned.to_string(), expected);
            let line: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(line["status"], "error");
            assert_eq!(line["message"], expected);
            assert_eq!(out.iter().filter(|&&byte| byte == b'\n').count(), 1);
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn startup_capture_admission_preserves_only_bounded_utf8() {
        use std::os::unix::ffi::OsStringExt;
        for value in [
            None,
            Some("".into()),
            Some("x".repeat(4097).into()),
            Some(std::ffi::OsString::from_vec(vec![0xff])),
        ] {
            assert!(captured_activation_token(value).is_none());
        }
        for value in [" ", "synthetic \n opaque ☃"] {
            assert!(captured_activation_token(Some(value.into())).as_deref() == Some(value));
        }
        assert!(captured_activation_token(Some("x".repeat(4096).into())).is_some());
    }
    #[cfg(all(unix, feature = "desktop"))]
    #[test]
    fn bounded_bridge_moves_tokens_and_starts_age_only_at_accept() {
        use diffz_adapters::handoff::{Listener, Writes, socket_path};
        use std::{
            io::Write as _,
            os::unix::net::UnixStream,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        let rx = serve(Listener::bind(dir.path(), Writes::default()).unwrap());
        let connect = |token: &str| {
            let mut stream = UnixStream::connect(socket_path(dir.path())).unwrap();
            let frame = serde_json::json!({"version":1,"request":null,"activation_token":token});
            writeln!(stream, "{frame}").unwrap();
            stream
        };
        let _first = connect("first-synthetic");
        let active = rx.recv_blocking().unwrap();
        assert!(active.activation_token.as_deref() == Some("first-synthetic"));
        let _second = connect("second-synthetic");
        let _third = connect("third-synthetic");
        let _fourth = connect("fourth-synthetic");
        let deadline = Instant::now() + Duration::from_secs(2);
        while rx.len() != 1 {
            assert!(
                Instant::now() < deadline,
                "bridge did not queue its one message"
            );
            std::thread::yield_now();
        }
        assert_eq!(rx.capacity(), Some(1));
        // One active plus one queued; the listener's next send blocks, so a
        // fourth connection cannot receive an accept timestamp yet.
        std::thread::sleep(Duration::from_millis(25));
        let released = Instant::now();
        let second = rx.recv_blocking().unwrap();
        let third = rx.recv_blocking().unwrap();
        let fourth = rx.recv_blocking().unwrap();
        assert!(second.activation_token.as_deref() == Some("second-synthetic"));
        assert!(third.activation_token.as_deref() == Some("third-synthetic"));
        assert!(fourth.activation_token.as_deref() == Some("fourth-synthetic"));
        assert!(fourth.accepted_at >= released);
        // Move-out before reply means the reply closure owns no token copy.
        let mut active = active;
        assert!(active.activation_token.take().is_some());
        (active.reply)(Ok(()));
        drop((second, third, fourth));
        // Wake the otherwise idle accept loop after closing the UI receiver.
        drop(rx);
        let _last = connect("discarded-synthetic");
    }

    #[cfg(all(unix, feature = "desktop"))]
    #[test]
    fn receiver_drop_releases_a_listener_blocked_on_enqueue() {
        use diffz_adapters::handoff::{Listener, Writes, socket_path};
        use std::{
            io::{Read as _, Write as _},
            os::unix::net::UnixStream,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        let rx = serve(Listener::bind(dir.path(), Writes::default()).unwrap());
        let connect = || {
            let mut stream = UnixStream::connect(socket_path(dir.path())).unwrap();
            stream
                .write_all(b"{\"version\":1,\"request\":null,\"activation_token\":\"synthetic\"}\n")
                .unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
        };
        let _queued = connect();
        let mut blocked = connect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while rx.len() != 1 {
            assert!(Instant::now() < deadline, "first enqueue did not finish");
            std::thread::yield_now();
        }
        drop(rx);
        let mut bytes = Vec::new();
        assert!(blocked.read_to_end(&mut bytes).is_ok());
        assert!(bytes.is_empty());
    }
    #[test]
    fn no_arguments_name_no_source() {
        assert!(parse(Vec::<OsString>::new()).unwrap().request.is_none());
    }
    #[test]
    fn a_fixture_is_only_opened_when_named() {
        assert_eq!(
            parse(["--fixture", "F01"].map(OsString::from))
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
            parse([OsString::from(&path), "--inspect".into()]).unwrap().request,
            Some(OpenRequest::Patch(p)) if p == std::path::Path::new(&path)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn positional_patch_keeps_trailing_whitespace_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let trimmed = dir.path().join("review 雪.patch");
        let exact = dir.path().join("review 雪.patch ");
        std::fs::write(&trimmed, "").unwrap();
        std::fs::write(&exact, "").unwrap();
        for args in [
            vec![exact.as_os_str().to_owned()],
            vec!["--patch".into(), exact.as_os_str().to_owned()],
        ] {
            assert_eq!(
                parse(args).unwrap().request,
                Some(OpenRequest::Patch(exact.clone()))
            );
        }
        std::fs::remove_file(&trimmed).unwrap();
        assert_eq!(
            parse([exact.as_os_str().to_owned()]).unwrap().request,
            Some(OpenRequest::Patch(exact))
        );
    }
    #[cfg(unix)]
    #[test]
    fn missing_literal_patch_does_not_open_trimmed_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let trimmed = dir.path().join("review.patch");
        std::fs::write(&trimmed, "").unwrap();
        let missing = dir.path().join("review.patch ");
        assert!(parse([missing.as_os_str().to_owned()]).is_err());
        assert_eq!(
            parse(["--patch".into(), missing.as_os_str().to_owned()])
                .unwrap()
                .request,
            Some(OpenRequest::Patch(missing))
        );
    }
    #[cfg(unix)]
    #[test]
    fn positional_patch_keeps_leading_relative_whitespace() {
        // Own a unique cwd-relative fixture without changing the process-wide cwd.
        let dir = tempfile::Builder::new()
            .prefix(" diffz-literal-")
            .tempdir_in(".")
            .unwrap();
        let relative = PathBuf::from(dir.path().file_name().unwrap()).join("review.patch");
        std::fs::write(&relative, "").unwrap();
        let input = relative.to_str().unwrap().to_string();
        assert!(input.starts_with(' '));
        assert_eq!(
            parse([OsString::from(input)]).unwrap().request,
            Some(OpenRequest::Patch(std::path::absolute(relative).unwrap()))
        );
    }
    #[cfg(unix)]
    #[test]
    fn existing_review_address_shaped_file_takes_precedence() {
        let dir = tempfile::Builder::new()
            .prefix("diffz-literal-")
            .tempdir_in(".")
            .unwrap();
        let relative = PathBuf::from(dir.path().file_name().unwrap()).join("repo#123");
        let input = relative.to_str().unwrap().to_string();
        assert!(matches!(
            GithubTarget::parse(&input),
            Ok(GithubTarget::Pr(_))
        ));
        std::fs::write(&relative, "").unwrap();
        assert_eq!(
            parse([OsString::from(input)]).unwrap().request,
            Some(OpenRequest::Patch(std::path::absolute(relative).unwrap()))
        );
    }
    #[cfg(unix)]
    #[test]
    fn native_literal_patch_uses_real_detector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(non_unicode_argument());
        let created = match std::fs::write(&path, "") {
            Ok(()) => true,
            // macOS filesystems can reject a non-UTF-8 filename at creation.
            // Darwin EILSEQ is 92; other creation errors must still fail the test.
            #[cfg(target_os = "macos")]
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(92), "{error}");
                assert!(!path.exists());
                assert!(parse([path.as_os_str().to_owned()]).is_err());
                false
            }
            #[cfg(not(target_os = "macos"))]
            Err(error) => panic!("cannot create native filename fixture: {error}"),
        };
        if created {
            assert_eq!(
                parse([path.as_os_str().to_owned()]).unwrap().request,
                Some(OpenRequest::Patch(std::path::absolute(&path).unwrap()))
            );
        }
        assert_eq!(
            parse(["--patch".into(), path.as_os_str().to_owned()])
                .unwrap()
                .request,
            Some(OpenRequest::Patch(path.clone()))
        );
        let error = parse([
            "--patch".into(),
            path.as_os_str().to_owned(),
            "--json".into(),
        ])
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "source cannot be represented in JSON; use a UTF-8 path"
        );
        assert!(parse([path.as_os_str().to_owned(), "--json".into()]).is_err());
        if created {
            std::fs::remove_file(&path).unwrap();
        }
        assert!(parse([path.as_os_str().to_owned()]).is_err());
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
            let options = parse([OsString::from(address)]).unwrap();
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
        let error = parse(["no/such/change.patch".into()])
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
            let error = parse(args.into_iter().map(OsString::from)).unwrap_err();
            assert_eq!(error.to_string(), "pass a single source per launch");
        }
    }
    #[test]
    fn launch_flags_are_parsed() {
        let options = parse(["--foreground", "--json"].map(OsString::from)).unwrap();
        assert!(options.foreground && options.json && options.request.is_none());
        let options = parse(Vec::<OsString>::new()).unwrap();
        assert!(!options.foreground && !options.json);
    }
    #[test]
    fn launched_windows_get_absolute_paths_and_run_in_the_foreground() {
        let options =
            parse(["--git", "repo", "--base", "v1", "--allow-gitlab-writes"].map(OsString::from))
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
        let options = parse(["group/project!4".into()]).unwrap();
        let args = launch_args(&options, Path::new("/state"), options.request.as_ref()).unwrap();
        assert_eq!(
            args[3..],
            ["--mr", "group/project!4"].map(std::ffi::OsString::from)
        );
        let compare = "https://gitlab.com/g/p/-/compare/v1...v2";
        let options = parse([compare.into()]).unwrap();
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
            let error = parse(args.into_iter().map(OsString::from)).unwrap_err();
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
                    file.map(OsString::from)
                } else {
                    Some(OsString::from(arg))
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
        assert!(parse(["--pr", "o/r#1", "--fixture", "F01"].map(OsString::from)).is_err());
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
            let request = parse(["--compare", url].map(OsString::from))
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
                parse(["--compare", url].map(OsString::from)).is_err(),
                "{url}"
            );
        }
        assert!(parse(["--compare".into()]).is_err());
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
        assert_eq!(
            line,
            serde_json::json!({ "status": "handed_off", "source": { "Fixture": "F01" } })
        );
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
            early_exit(&["--version".into()]).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            early_exit(&["-V".into()]).as_deref(),
            Some(expected.as_str())
        );
        assert!(early_exit(&[]).is_none());
    }
}
