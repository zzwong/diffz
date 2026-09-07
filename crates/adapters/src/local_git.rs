//! Git input is read-only. It does not check out files, alter the index, run hooks, textconv, or diff drivers.
use crate::{
    Result,
    process::{ProcessRequest, Runner},
};
use diffz_core::{domain::*, patch::*, provider::Cancellation};
use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Debug, Clone)]
pub enum LocalMode {
    Compare { base: String, head: String },
    Staged,
    WorkingTree,
}
pub struct LocalGit {
    program: PathBuf,
}
impl LocalGit {
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }
    fn run(&self, root: &Path, args: Vec<OsString>, cancel: Cancellation) -> Result<Vec<u8>> {
        let mut r = ProcessRequest::new(self.program.clone()).args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.pager=cat",
            "-C",
        ]);
        r.args.push(root.as_os_str().into());
        r.args.extend(args);
        r.deadline = Duration::from_secs(60);
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_EXTERNAL_DIFF",
            "GIT_DIFF_OPTS",
        ] {
            r.env_remove.push(key.into());
        }
        // Drop command-level config injection while retaining the user's normal global credentials.
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().starts_with("GIT_CONFIG_KEY_")
                || k.to_string_lossy().starts_with("GIT_CONFIG_VALUE_")
                || k == "GIT_CONFIG_COUNT"
            {
                r.env_remove.push(k);
            }
        }
        let out = Runner::run(r, cancel)?;
        if !out.status.success() {
            return Err(format!(
                "Git could not read the repository (exit {:?}); the worktree was left untouched",
                out.status.code()
            )
            .into());
        }
        Ok(out.stdout)
    }
    fn resolve(&self, root: &Path, value: &str, cancel: Cancellation) -> Result<String> {
        if value.is_empty() || value.starts_with('-') || value.contains('\0') {
            return Err("invalid revision expression".into());
        }
        let raw = self.run(
            root,
            vec![
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{value}^{{commit}}").into(),
            ],
            cancel,
        )?;
        object_id(&raw)
    }
    fn patch(
        &self,
        root: &Path,
        extra: Vec<OsString>,
        cancel: Cancellation,
    ) -> Result<PatchReport> {
        let mut args: Vec<OsString> = [
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--full-index",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--find-renames",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        args.extend(extra);
        Ok(parse_patch(
            &self.run(root, args, cancel)?,
            ParseLimits::default(),
        )?)
    }
    pub fn snapshot(&self, path: &Path, mode: LocalMode, cancel: Cancellation) -> Result<Snapshot> {
        let inside = path.canonicalize()?;
        let raw_root = self.run(
            &inside,
            vec!["rev-parse".into(), "--show-toplevel".into()],
            cancel.clone(),
        )?;
        let raw_root = raw_root.strip_suffix(b"\n").unwrap_or(&raw_root);
        #[cfg(unix)]
        let root = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(OsString::from_vec(raw_root.to_vec())).canonicalize()?
        };
        #[cfg(not(unix))]
        let root = PathBuf::from(
            std::str::from_utf8(raw_root).map_err(|_| "repository root is not UTF-8")?,
        )
        .canonicalize()?;
        if self.program.starts_with(&root) {
            return Err("the Git executable cannot live inside the repository under review".into());
        }
        match mode {
            LocalMode::Compare { base, head } => {
                let base_oid = self.resolve(&root, &base, cancel.clone())?;
                let head_oid = self.resolve(&root, &head, cancel.clone())?;
                let common = object_id(&self.run(
                    &root,
                    vec![
                        "merge-base".into(),
                        base_oid.clone().into(),
                        head_oid.clone().into(),
                    ],
                    cancel.clone(),
                )?)?;
                let patch = self.patch(
                    &root,
                    vec![common.clone().into(), head_oid.clone().into(), "--".into()],
                    cancel,
                )?;
                let mut s = Snapshot::with_origin(
                    format!("{} · {base}…{head}", root.display()),
                    patch,
                    None,
                    vec![],
                    format!("git:{root:?}:{common}:{head_oid}"),
                );
                // The root and fixed object IDs distinguish sessions with matching patch text.
                s.title
                    .push_str(&format!(" · {} → {}", &common[..12], &head_oid[..12]));
                Ok(s)
            }
            LocalMode::Staged | LocalMode::WorkingTree => {
                let staged = matches!(mode, LocalMode::Staged);
                for _ in 0..3 {
                    let mut a = self.patch(
                        &root,
                        if staged {
                            vec!["--cached".into(), "--".into()]
                        } else {
                            vec!["--".into()]
                        },
                        cancel.clone(),
                    )?;
                    if !staged {
                        self.untracked(&root, &mut a, cancel.clone())?;
                    }
                    let mut b = self.patch(
                        &root,
                        if staged {
                            vec!["--cached".into(), "--".into()]
                        } else {
                            vec!["--".into()]
                        },
                        cancel.clone(),
                    )?;
                    if !staged {
                        self.untracked(&root, &mut b, cancel.clone())?;
                    }
                    if serde_json::to_vec(&a)? == serde_json::to_vec(&b)? {
                        return Ok(Snapshot::with_origin(
                            format!(
                                "{} · {} (double-captured)",
                                root.display(),
                                if staged {
                                    "staged"
                                } else {
                                    "working tree + untracked"
                                }
                            ),
                            a,
                            None,
                            vec![],
                            format!(
                                "git:{root:?}:{}",
                                if staged { "staged" } else { "worktree" }
                            ),
                        ));
                    }
                }
                Err(
                    "the local source changed during capture; the previous session was preserved"
                        .into(),
                )
            }
        }
    }
    fn untracked(&self, root: &Path, patch: &mut PatchReport, cancel: Cancellation) -> Result<()> {
        let names = self.run(
            root,
            vec![
                "ls-files".into(),
                "--others".into(),
                "--exclude-standard".into(),
                "-z".into(),
            ],
            cancel,
        )?;
        let paths: Vec<_> = names.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
        if paths.len() > 200 {
            return Err(
                "over 200 files are untracked; use staged mode or narrow the set first".into(),
            );
        }
        let mut total_bytes = 0usize;
        let mut total_rows = patch
            .files
            .iter()
            .flat_map(|f| &f.hunks)
            .map(|h| h.rows.len())
            .sum::<usize>();
        for raw in paths {
            let path = RepoPath::new(raw.to_vec())?;
            #[cfg(unix)]
            let os = {
                use std::os::unix::ffi::OsStringExt;
                OsString::from_vec(raw.to_vec())
            };
            #[cfg(not(unix))]
            let os = OsString::from(path.utf8()?);
            let full = root.join(os);
            let meta = full.symlink_metadata()?;
            if !meta.file_type().is_file() {
                return Err(format!(
                    "untracked item {} is not a regular file; stage or exclude it before snapshotting",
                    path.display()
                )
                .into());
            }
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let file = options.open(&full)?;
            let mut bytes = vec![];
            file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err("an untracked file exceeds 32 MiB; it was kept out explicitly".into());
            }
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or("untracked byte count overflow")?;
            if total_bytes > 32 * 1024 * 1024 {
                return Err("all untracked content is over the 32 MiB cap".into());
            }
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 != 0 {
                    "100755"
                } else {
                    "100644"
                }
            };
            #[cfg(not(unix))]
            let mode = "100644";
            let doc = if bytes.contains(&0) {
                None
            } else {
                match SourceDocument::from_utf8(bytes.clone()) {
                    Ok(d) => Some(d),
                    Err(SourceError::InvalidUtf8) => None,
                    Err(e) => return Err(e.to_string().into()),
                }
            };
            let mut f = FileChange {
                id: FileId(digest(&[b"file-v1", b"", path.bytes()])),
                old_path: None,
                new_path: Some(path),
                kind: ChangeKind::Added,
                content: if doc.is_some() {
                    ContentKind::Text
                } else {
                    ContentKind::Binary
                },
                old_mode: None,
                new_mode: Some(mode.into()),
                old_oid: None,
                new_oid: None,
                hunks: vec![],
                metadata: vec!["untracked file; the worktree was left unchanged".into()],
            };
            if let Some(d) = doc {
                total_rows += d.lines().len();
                if total_rows > 500_000 {
                    return Err("the local patch total is above 500,000 rows".into());
                }
                if d.lines().iter().any(|l| l.content.len() > 1024 * 1024) {
                    return Err("an untracked source line is over 1 MiB".into());
                }
                if !d.lines().is_empty() {
                    f.hunks.push(Hunk {
                        old_start: 0,
                        old_count: 0,
                        new_start: 1,
                        new_count: d.lines().len() as u32,
                        section: String::new(),
                        rows: d
                            .lines()
                            .iter()
                            .enumerate()
                            .map(|(i, l)| PatchRow {
                                old_line: None,
                                new_line: Some(i as u32 + 1),
                                text: d.line_text(i as u32 + 1).unwrap_or_default().into(),
                                ending: l.ending,
                                kind: RowKind::Added,
                            })
                            .collect(),
                    });
                }
            }
            patch.files.push(f);
        }
        Ok(())
    }
}
fn object_id(bytes: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| "Git object ID was not ASCII")?
        .trim();
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Git returned an object ID with an invalid form".into());
    }
    Ok(value.to_ascii_lowercase())
}
