#![cfg(unix)]
use diffz_adapters::{github::GithubReader, gitlab::GitlabReader};
use diffz_core::domain::{ProviderId, RemoteTarget, RepositoryKey};
use std::os::unix::fs::PermissionsExt;
#[test]
fn source_reads_pin_revisions_and_encode_paths_without_live_cli() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("mock-provider");
    std::fs::write(
        &program,
        r#"#!/usr/bin/env python3
import sys, pathlib, json
pathlib.Path(__file__).with_suffix('.args').write_text(json.dumps(sys.argv[1:]))
sys.stdout.buffer.write(b'HTTP/1.1 200 OK\r\n\r\nfirst\nsecond\n')
"#,
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut target = RemoteTarget {
        provider: ProviderId::GITHUB,
        repository: RepositoryKey {
            host: "example.com".into(),
            id: 1,
            owner: "group".into(),
            name: "repo".into(),
        },
        account: "me".into(),
        pr: 1,
        target_tip: "a".repeat(40),
        comparison_base: "b".repeat(40),
        head: "c".repeat(40),
        open: true,
        draft: false,
        pending_review: false,
    };
    let bytes = GithubReader::new(program.clone())
        .source(&target, "dir/a b.rs", &target.head)
        .unwrap();
    assert_eq!(bytes, b"first\nsecond\n");
    let args = std::fs::read_to_string(program.with_extension("args")).unwrap();
    assert!(args.contains("a%20b.rs?ref="));
    assert!(args.contains(&target.head));
    target.provider = ProviderId::GITLAB;
    target.repository.owner = "group/sub".into();
    let bytes = GitlabReader::new(program.clone())
        .source(&target, "dir/a b.rs", &target.comparison_base)
        .unwrap();
    assert_eq!(bytes, b"first\nsecond\n");
    let args = std::fs::read_to_string(program.with_extension("args")).unwrap();
    assert!(args.contains("projects/group%2Fsub%2Frepo/repository/files/dir%2Fa%20b.rs/raw?ref="));
    assert!(args.contains(&target.comparison_base));
}
