use crate::domain::{ProviderId, Side, Snapshot, SourcePoint};
fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
pub fn source_link(snapshot: &Snapshot, point: &SourcePoint) -> Result<String, String> {
    if point.snapshot != snapshot.id || point.line == 0 {
        return Err("Line belongs to a different review".into());
    }
    let t = snapshot
        .remote
        .as_ref()
        .ok_or("Hosted line link unavailable for local source")?;
    let file = snapshot
        .file(&point.file)
        .ok_or("File is not in this review")?;
    let (path, revision) = match point.side {
        Side::Left => (file.old_path.as_ref(), &t.comparison_base),
        Side::Right => (file.new_path.as_ref(), &t.head),
    };
    let path = path.ok_or("This side has no source file")?.utf8()?;
    let route = if t.provider == ProviderId::GITLAB {
        "-/blob"
    } else {
        "blob"
    };
    Ok(format!(
        "https://{}/{}/{}/{}/{}/{}#L{}",
        t.repository.host,
        encode(&t.repository.owner),
        encode(&t.repository.name),
        route,
        encode(revision),
        encode(path),
        point.line
    ))
}
#[cfg(test)]
mod tests {
    use super::encode;
    #[test]
    fn encodes_path_separators_and_reserved_characters() {
        assert_eq!(encode("dir/a b#?.rs"), "dir/a%20b%23%3F.rs");
        assert_eq!(encode("group/sub"), "group/sub");
    }
}

#[cfg(test)]
mod link_tests {
    use super::source_link;
    use crate::{domain::*, patch::parse_patch};
    #[test]
    fn links_use_frozen_revision_and_correct_provider_route() {
        let remote = RemoteTarget {
            provider: ProviderId::GITHUB,
            repository: RepositoryKey {
                host: "example.com".into(),
                id: 1,
                owner: "group".into(),
                name: "repo".into(),
            },
            account: "me".into(),
            pr: 1,
            target_tip: "tip".into(),
            comparison_base: "base".into(),
            head: "head".into(),
            open: true,
            draft: false,
            pending_review: false,
        };
        let patch = parse_patch(
            b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
            Default::default(),
        )
        .unwrap();
        let mut snapshot = Snapshot::new("test".into(), patch, Some(remote), vec![]);
        let mut p = SourcePoint {
            snapshot: snapshot.id.clone(),
            file: snapshot.patch.files[0].id.clone(),
            side: Side::Left,
            line: 1,
            byte_column: 0,
        };
        assert_eq!(
            source_link(&snapshot, &p).unwrap(),
            "https://example.com/group/repo/blob/base/a.rs#L1"
        );
        p.side = Side::Right;
        assert_eq!(
            source_link(&snapshot, &p).unwrap(),
            "https://example.com/group/repo/blob/head/a.rs#L1"
        );
        snapshot.remote.as_mut().unwrap().provider = ProviderId::GITLAB;
        assert_eq!(
            source_link(&snapshot, &p).unwrap(),
            "https://example.com/group/repo/-/blob/head/a.rs#L1"
        );
    }
}
