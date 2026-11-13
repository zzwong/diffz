//! Resolve an omitted placement URL through the authenticated Core catalog.
//! The CLI v0.12.2 resolver joins by cluster slug, then by cell in catalog URLs.
//! No default-region fallback or unvalidated credential destination is allowed.
use super::*;

const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_CLUSTERS: usize = 1024;

#[derive(Deserialize)]
struct Catalog {
    clusters: Vec<Cluster>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cluster {
    slug: String,
    jurisdiction: String,
    api_url: Option<String>,
    public_url: String,
}

impl EntireReader {
    pub(super) fn placement_origin(
        &self,
        login: &Login,
        placement: &Value,
        cancel: Cancellation,
    ) -> Result<String> {
        match placement.get("apiBaseUrl") {
            Some(Value::String(raw)) => return api_origin(raw),
            Some(_) => return Err("Entire placement has an unsupported API origin".into()),
            None => {}
        }
        // Only absence enables discovery. An explicitly invalid inline origin
        // must fail instead of being replaced by a different credential target.
        let raw =
            self.request_limited(login, None, "/api/v1/clusters", cancel, MAX_CATALOG_BYTES)?;
        let catalog: Catalog = serde_json::from_slice(&raw)
            .map_err(|_| "Entire cluster catalog has an unsupported shape")?;
        catalog_origin(placement, &catalog)
    }
}

fn route_key(value: &str) -> Result<&str> {
    let key = value.trim();
    if key.is_empty() || key.len() > 253 || value.chars().any(char::is_control) {
        return Err("Entire placement catalog has an invalid routing key".into());
    }
    Ok(key)
}

fn catalog_origin(placement: &Value, catalog: &Catalog) -> Result<String> {
    if catalog.clusters.len() > MAX_CLUSTERS {
        return Err("Entire cluster catalog exceeds its safety limit".into());
    }
    let raw_slug = placement["clusterSlug"]
        .as_str()
        .ok_or("Entire placement is missing its cluster slug")?;
    // A blank slug is a miss in the released resolver, not a reason to reject
    // an otherwise usable cell mapping. A successful slug does not need cell.
    let slug = if raw_slug.trim().is_empty() {
        ""
    } else {
        route_key(raw_slug)?
    };
    let mut matches = catalog
        .clusters
        .iter()
        .filter(|cluster| !slug.is_empty() && cluster.slug.trim().eq_ignore_ascii_case(slug));
    let cluster = if let Some(cluster) = matches.next() {
        if matches.next().is_some() {
            return Err("Entire cluster catalog has an ambiguous placement match".into());
        }
        cluster
    } else {
        let cell = route_key(text(placement, "/cell")?)?;
        // The cell must be one DNS label. Match it at a host boundary, never as
        // a substring or as a jurisdiction/default-region approximation.
        if cell.len() > 63
            || !cell.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !cell.as_bytes()[0].is_ascii_alphanumeric()
            || !cell.as_bytes()[cell.len() - 1].is_ascii_alphanumeric()
        {
            return Err("Entire placement has no usable catalog cell key".into());
        }
        let mut matches = catalog
            .clusters
            .iter()
            .filter(|cluster| cluster_matches_cell(cluster, cell));
        let cluster = matches
            .next()
            .ok_or("Entire cluster catalog does not resolve the processing placement")?;
        if matches.next().is_some() {
            return Err("Entire cluster catalog has an ambiguous placement match".into());
        }
        cluster
    };
    route_key(&cluster.jurisdiction)?;
    api_origin(
        cluster
            .api_url
            .as_deref()
            .ok_or("Entire cluster catalog is missing its API origin")?,
    )
}

fn cluster_matches_cell(cluster: &Cluster, cell: &str) -> bool {
    [
        cluster.api_url.as_deref(),
        Some(cluster.public_url.as_str()),
    ]
    .into_iter()
    .flatten()
    .any(|raw| {
        let Ok(url) = strict_https_url(raw) else {
            return false;
        };
        let host = url.host_str().unwrap_or_default();
        (host.ends_with(".entire.io") || host.ends_with(".entire.com"))
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none()
            && host
                .split_once('.')
                .is_some_and(|(label, _)| label.eq_ignore_ascii_case(cell))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EU: &str = "https://aws-eu-west-1.api.entire.io";

    fn placement() -> Value {
        json!({"clusterSlug": "eu-slug", "cell": "aws-eu-west-1"})
    }

    fn clusters() -> Value {
        json!({"clusters": [
            {"slug": "us-slug", "jurisdiction": "us", "isDefault": true,
             "apiUrl": "https://aws-us-east-2.api.entire.io",
             "publicUrl": "https://aws-us-east-2.entire.io"},
            {"slug": "eu-slug", "jurisdiction": "eu", "isDefault": false,
             "apiUrl": EU, "publicUrl": "https://aws-eu-west-1.entire.io"}
        ]})
    }

    fn resolved(placement: &Value, value: Value) -> Result<String> {
        let catalog: Catalog = serde_json::from_value(value).unwrap();
        catalog_origin(placement, &catalog)
    }

    #[test]
    fn selects_unique_slug_independently_of_order_or_default_region() {
        let mut value = clusters();
        assert_eq!(resolved(&placement(), value.clone()).unwrap(), EU);
        value["clusters"].as_array_mut().unwrap().reverse();
        assert_eq!(resolved(&placement(), value).unwrap(), EU);
        let mut selected = placement();
        selected["clusterSlug"] = json!(" EU-SLUG ");
        assert_eq!(resolved(&selected, clusters()).unwrap(), EU);
        selected["cell"] = json!("");
        assert_eq!(resolved(&selected, clusters()).unwrap(), EU);
    }

    #[test]
    fn cell_fallback_uses_trusted_catalog_hosts_without_guessing() {
        let mut selected = placement();
        selected["clusterSlug"] = json!("not-in-catalog");
        assert_eq!(resolved(&selected, clusters()).unwrap(), EU);
        selected["clusterSlug"] = json!("  ");
        assert_eq!(resolved(&selected, clusters()).unwrap(), EU);
        let mut value = clusters();
        value["clusters"][1]["apiUrl"] = json!("https://alternate.api.entire.io");
        assert_eq!(
            resolved(&selected, value).unwrap(),
            "https://alternate.api.entire.io"
        );
        for cell in ["missing", "aws-eu", "", "aws-eu-west-1.evil", "../eu"] {
            selected["cell"] = json!(cell);
            assert!(resolved(&selected, clusters()).is_err(), "{cell}");
        }
    }

    #[test]
    fn ambiguous_or_unusable_slug_matches_never_fall_back() {
        let mut duplicate = clusters();
        let row = duplicate["clusters"][1].clone();
        duplicate["clusters"].as_array_mut().unwrap().push(row);
        assert!(resolved(&placement(), duplicate).is_err());
        let mut duplicate = clusters();
        let mut row = duplicate["clusters"][1].clone();
        row["slug"] = json!(" EU-SLUG ");
        duplicate["clusters"].as_array_mut().unwrap().push(row);
        let mut selected = placement();
        selected["clusterSlug"] = json!(" Eu-SlUg ");
        assert!(resolved(&selected, duplicate).is_err());
        for field in ["apiUrl", "jurisdiction"] {
            let mut value = clusters();
            value["clusters"][1][field] = json!("");
            assert!(resolved(&placement(), value).is_err(), "{field}");
        }
        let mut value = clusters();
        value["clusters"][1]
            .as_object_mut()
            .unwrap()
            .remove("apiUrl");
        assert!(resolved(&placement(), value).is_err());
    }

    #[test]
    fn unique_slug_takes_priority_over_a_conflicting_cell_match() {
        let mut selected = placement();
        selected["cell"] = json!("aws-us-east-2");
        let mut value = clusters();
        assert_eq!(resolved(&selected, value.clone()).unwrap(), EU);
        value["clusters"].as_array_mut().unwrap().reverse();
        assert_eq!(resolved(&selected, value).unwrap(), EU);
        let mut value = clusters();
        value["clusters"][1]
            .as_object_mut()
            .unwrap()
            .remove("apiUrl");
        assert!(resolved(&selected, value).is_err());
    }

    #[test]
    fn cell_fallback_rejects_ambiguous_or_untrusted_matches() {
        let mut selected = placement();
        selected["clusterSlug"] = json!("not-in-catalog");
        let mut duplicate = clusters();
        let mut row = duplicate["clusters"][1].clone();
        row["slug"] = json!("another-slug");
        duplicate["clusters"].as_array_mut().unwrap().push(row);
        assert!(resolved(&selected, duplicate).is_err());
        let mut value = clusters();
        value["clusters"][1]["apiUrl"] = json!("https://aws-eu-west-1.api.entire.io.evil.example");
        value["clusters"][1]["publicUrl"] = json!("https://aws-eu-west-1.evil.example");
        assert!(resolved(&selected, value).is_err());
    }

    #[test]
    fn catalog_origins_keep_existing_security_and_size_boundaries() {
        for origin in [
            "http://aws-eu-west-1.api.entire.io",
            "https://aws-eu-west-1.api.entire.io.evil.example",
            "https://user@aws-eu-west-1.api.entire.io",
            "https://aws-eu-west-1.api.entire.io:443",
            "https://aws-eu-west-1.api.entire.io/path",
            "https://aws-eu-west-1.api.entire.io?query=1",
            "https://aws-eu-west-1.api.entire.io#fragment",
        ] {
            let mut value = clusters();
            value["clusters"][1]["apiUrl"] = json!(origin);
            assert!(resolved(&placement(), value).is_err(), "{origin}");
        }
        let row = clusters()["clusters"][0].clone();
        assert!(
            resolved(
                &placement(),
                json!({"clusters": vec![row; MAX_CLUSTERS + 1]})
            )
            .is_err()
        );
        assert!(resolved(&placement(), json!({"clusters": []})).is_err());
    }
}
