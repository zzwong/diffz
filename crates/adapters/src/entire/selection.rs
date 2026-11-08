//! Typed, read-only project Change discovery. Identity fields stay private so a
//! UI can display choices without manufacturing a selection from a URL.
use super::*;

#[derive(Debug, Clone)]
pub struct TrailChoices {
    pub(super) address: TrailAddress,
    pub(super) account: String,
    pub(super) project: ProjectTrail,
    title: String,
    possibly_partial: bool,
    changes: Vec<ChangeChoice>,
}

impl TrailChoices {
    pub fn title(&self) -> &str {
        &self.title
    }
    /// A false value describes this response, not a guarantee that the user can
    /// read every repository. Revalidate the explicit choice before opening.
    pub fn possibly_partial(&self) -> bool {
        self.possibly_partial
    }
    pub fn changes(&self) -> &[ChangeChoice] {
        &self.changes
    }
    pub(super) fn read(
        address: TrailAddress,
        account: String,
        project: ProjectTrail,
        detail: &Value,
    ) -> Result<Self> {
        let changes = detail["changes"]
            .as_array()
            .ok_or("Entire project Trail did not provide its readable Changes")?;
        if changes.len() > 10_000 {
            return Err("Entire project Trail exceeds the Change safety limit".into());
        }
        let possibly_partial = match detail.get("isPossiblyPartial") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            Some(_) => {
                return Err("Entire project Trail has an invalid access-filtering marker".into());
            }
        };
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let mut choices = Vec::with_capacity(changes.len());
        for row in changes {
            let repository = text(row, "/repository")?;
            let id = text(row, "/id")?;
            let placement = text(row, "/repositoryId")?;
            let number = row["number"]
                .as_u64()
                .filter(|n| *n > 0 && *n < 10_000_000_000)
                .ok_or("Entire Change has no valid number")?;
            if !repo_name(repository) || !opaque_id(id) || !placement_id(placement) {
                return Err(
                    "Entire Change has an unsupported repository or Change identity".into(),
                );
            }
            if !ids.insert(id.to_owned()) || !keys.insert((repository.to_ascii_lowercase(), number))
            {
                return Err(
                    "Entire project Trail has duplicate or ambiguous Change identities".into(),
                );
            }
            choices.push(ChangeChoice {
                repository: repository.into(),
                number,
                id: id.into(),
                placement: placement.into(),
                address: address.change_url(repository, number),
            });
        }
        Ok(Self {
            address,
            account,
            project,
            title: detail["title"].as_str().unwrap_or("Entire Trail").into(),
            possibly_partial,
            changes: choices,
        })
    }
    pub(super) fn choice(&self, index: usize) -> Result<&ChangeChoice> {
        self.changes
            .get(index)
            .ok_or_else(|| "Select a readable Entire Change".into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeChoice {
    repository: String,
    number: u64,
    pub(super) id: String,
    pub(super) placement: String,
    address: String,
}
impl ChangeChoice {
    pub fn repository(&self) -> &str {
        &self.repository
    }
    pub fn number(&self) -> u64 {
        self.number
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub(super) fn key(&self) -> (String, u64) {
        (self.repository.clone(), self.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> Value {
        json!({"id":"change_one", "repositoryId":"01JBBBBBBBBBBBBBBBBBBBBBBB", "repository":"widget", "number":7})
    }
    fn read(detail: Value) -> Result<TrailChoices> {
        let address = TrailAddress::parse("https://entire.com/et/acme/-/trails/42")?;
        let project = ProjectTrail {
            origin: "https://aws-us-east-2.api.entire.io".into(),
            project_id: "01JCCCCCCCCCCCCCCCCCCCCCCC".into(),
            id: "01JDDDDDDDDDDDDDDDDDDDDDDD".into(),
            endpoint: "/api/v1/et/acme/trails/42".into(),
            forge: "et".into(),
            name: "acme".into(),
            number: 42,
        };
        TrailChoices::read(address, "account_1".into(), project, &detail)
    }
    #[test]
    fn typed_choices_preserve_labels_and_opaque_identity() {
        let choices = read(json!({"title":"Project intent", "changes":[row()]})).unwrap();
        assert_eq!(choices.title(), "Project intent");
        assert!(!choices.possibly_partial());
        let choice = choices.choice(0).unwrap();
        assert_eq!(choice.repository(), "widget");
        assert_eq!(choice.number(), 7);
        assert_eq!(choice.id, "change_one");
        assert_eq!(choice.placement, "01JBBBBBBBBBBBBBBBBBBBBBBB");
        assert_eq!(
            choice.address(),
            "https://entire.com/et/acme/-/trails/42/changes/widget/7"
        );
        assert!(choices.choice(1).is_err());
    }
    #[test]
    fn partial_single_and_empty_lists_remain_explicit() {
        for rows in [vec![], vec![row()]] {
            let choices = read(json!({"changes":rows,"isPossiblyPartial":true})).unwrap();
            assert!(choices.possibly_partial());
        }
    }
    #[test]
    fn complete_multiple_choices_keep_independent_identity() {
        let mut other = row();
        other["id"] = json!("change_two");
        other["number"] = json!(8);
        let choices = read(json!({"changes":[row(),other],"isPossiblyPartial":false})).unwrap();
        assert_eq!(choices.changes().len(), 2);
        assert_eq!(choices.choice(1).unwrap().number(), 8);
    }
    #[test]
    fn malformed_completeness_markers_are_rejected() {
        for marker in [Value::Null, json!("false"), json!(0), json!({})] {
            assert!(read(json!({"changes":[row()],"isPossiblyPartial":marker})).is_err());
        }
    }
    #[test]
    fn duplicate_ids_or_case_insensitive_keys_are_rejected() {
        for field in ["id", "number"] {
            let mut other = row();
            other["repository"] = json!("WIDGET");
            other[field] = if field == "id" {
                json!("different_id")
            } else {
                json!(8)
            };
            assert!(read(json!({"changes":[row(),other]})).is_err());
        }
    }
    #[test]
    fn malformed_rows_are_not_silently_omitted() {
        for (field, value) in [
            ("id", json!("bad/id")),
            ("repositoryId", json!("repo_not_placement")),
            ("repository", json!("../other")),
            ("number", json!(0)),
            ("number", json!(10_000_000_000u64)),
        ] {
            let mut malformed = row();
            malformed[field] = value;
            assert!(read(json!({"changes":[row(),malformed]})).is_err());
        }
        assert!(read(json!({"changes":null})).is_err());
    }
    #[test]
    fn excessive_membership_is_rejected_before_selection() {
        assert!(read(json!({"changes":vec![row();10_001]})).is_err());
    }
}
