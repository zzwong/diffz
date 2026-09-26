use crate::{
    domain::{Side, Snapshot, SourcePoint},
    provider::ReviewRules,
};
pub fn encode(value: &str) -> String {
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
pub fn source_link(
    snapshot: &Snapshot,
    point: &SourcePoint,
    rules: Option<&dyn ReviewRules>,
) -> Result<String, String> {
    if point.snapshot != snapshot.id || point.line == 0 {
        return Err("Line belongs to a different review".into());
    }
    let t = snapshot
        .remote
        .as_ref()
        .ok_or("Hosted line link unavailable for local source")?;
    let rules = rules.filter(|r| r.id() == t.provider).ok_or_else(|| {
        format!(
            "No provider named {} is available for line links",
            t.provider
        )
    })?;
    let file = snapshot
        .file(&point.file)
        .ok_or("File is not in this review")?;
    let (path, revision) = match point.side {
        Side::Left => (file.old_path.as_ref(), &t.comparison_base),
        Side::Right => (file.new_path.as_ref(), &t.head),
    };
    let path = path.ok_or("This side has no source file")?.utf8()?;
    Ok(rules.line_url(t, path, revision, point.line))
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
    use crate::{domain::*, patch::parse_patch, provider::ReviewRules, review::*};
    use serde_json::value::RawValue;

    struct Host;
    impl ReviewRules for Host {
        fn id(&self) -> ProviderId {
            ProviderId::new("Host")
        }
        fn name(&self) -> &str {
            "Host"
        }
        fn open_label(&self) -> &str {
            ""
        }
        fn address_label(&self) -> &str {
            ""
        }
        fn address_hint(&self) -> &str {
            ""
        }
        fn address_help(&self) -> &str {
            ""
        }
        fn write_flag(&self) -> &str {
            ""
        }
        fn reopen_address(&self, _: &RemoteTarget) -> String {
            String::new()
        }
        fn line_url(&self, _: &RemoteTarget, path: &str, revision: &str, line: u32) -> String {
            format!("{revision}:{path}:{line}")
        }
        fn payload(&self, _: &PreparedReview) -> Box<RawValue> {
            RawValue::from_string("{}".into()).unwrap()
        }
    }

    #[test]
    fn links_use_the_frozen_revision_of_each_side() {
        let remote = RemoteTarget {
            provider: ProviderId::new("Host"),
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
            b"diff --git a/a.rs b/b.rs\n--- a/a.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+new\n",
            Default::default(),
        )
        .unwrap();
        let snapshot = Snapshot::new("test".into(), patch, Some(remote), vec![]);
        let mut p = SourcePoint {
            snapshot: snapshot.id.clone(),
            file: snapshot.patch.files[0].id.clone(),
            side: Side::Left,
            line: 1,
            byte_column: 0,
        };
        assert_eq!(
            source_link(&snapshot, &p, Some(&Host)).unwrap(),
            "base:a.rs:1"
        );
        p.side = Side::Right;
        assert_eq!(
            source_link(&snapshot, &p, Some(&Host)).unwrap(),
            "head:b.rs:1"
        );
        assert!(source_link(&snapshot, &p, None).is_err());
    }
}
