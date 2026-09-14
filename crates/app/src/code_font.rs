//! Resolve a concrete Linux code font before GPUI sees the family name.
//! GPUI's missing-family fallback is a UI font, not necessarily monospace.
use std::process::{Command, Stdio};

pub fn resolve(requested: Option<&str>) -> Result<String, String> {
    // An explicit --font must be confirmed fixed-width before we use it, so a
    // proportional substitution still falls back to the system monospace. The
    // monospace *alias* is fontconfig's authoritative list of monospace fonts,
    // so we trust it even when the selected font leaves `spacing` unset.
    select(
        requested,
        |pattern| run_fc_match(pattern, /* accept_unset_spacing */ false),
        |pattern| run_fc_match(pattern, /* accept_unset_spacing */ true),
    )
}

fn select(
    requested: Option<&str>,
    mut requested_lookup: impl FnMut(&str) -> Result<Option<String>, String>,
    mut alias_lookup: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<String, String> {
    let matched = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(family) => requested_lookup(&family_pattern(family))?,
        None => None,
    };
    if let Some(family) = matched {
        return Ok(family);
    }
    alias_lookup("monospace")?.ok_or_else(|| {
        "no usable monospace font found; install dejavu-sans-mono-fonts (Fedora), \
         fonts-dejavu-core (Debian/Ubuntu), or ttf-dejavu (Arch)"
            .to_string()
    })
}

fn run_fc_match(pattern: &str, accept_unset_spacing: bool) -> Result<Option<String>, String> {
    let output = Command::new("fc-match")
        .args(["--format", "%{family[0]}\n%{spacing}\n", "--", pattern])
        // Fontconfig debug output otherwise contaminates the machine format.
        .env_remove("FC_DEBUG")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run fc-match: {error}; install fontconfig"))?;
    if !output.status.success() {
        return Err(format!(
            "fc-match failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "fc-match returned a non-UTF-8 font family".to_string())?;
    Ok(parse_match(&output, accept_unset_spacing))
}

fn family_pattern(family: &str) -> String {
    let mut pattern = String::new();
    for ch in family.chars() {
        if matches!(ch, '\\' | '-' | ':' | ',') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

fn parse_match(output: &str, accept_unset_spacing: bool) -> Option<String> {
    let mut lines = output.lines();
    let family = lines.next()?;
    let spacing = lines.next()?.trim();
    // Do not request :spacing=100: Fontconfig can copy that property into a
    // proportional fallback's result, making it look falsely monospace.
    if family.is_empty() || family.chars().any(char::is_control) || lines.next().is_some() {
        return None;
    }
    match spacing {
        // Declared fixed-width: always usable.
        "100" | "110" => Some(family.to_string()),
        // Unset spacing is common for genuine monospace variable fonts (e.g.
        // Google's Noto Sans Mono) whose files never set `post.isFixedPitch`.
        // Empty here means "fontconfig could not tell", not "proportional", so
        // we accept it only when the caller already trusts the source (the
        // system `monospace` alias). Explicit family requests are strict.
        "" if accept_unset_spacing => Some(family.to_string()),
        // Declared proportional (0) or dual-width (90): never usable.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_system_monospace_instead_of_a_hardcoded_family() {
        let result = select(
            None,
            |_| unreachable!(),
            |pattern| {
                assert_eq!(pattern, "monospace");
                Ok(Some("Liberation Mono".into()))
            },
        );
        assert_eq!(result.unwrap(), "Liberation Mono");
    }

    #[test]
    fn installed_explicit_font_is_preserved() {
        assert_eq!(
            select(
                Some("Noto Sans Mono"),
                |pattern| {
                    assert_eq!(pattern, "Noto Sans Mono");
                    Ok(Some("Noto Sans Mono".into()))
                },
                |_| unreachable!(),
            )
            .unwrap(),
            "Noto Sans Mono"
        );
    }

    #[test]
    fn proportional_match_retries_the_system_monospace_alias() {
        let patterns = std::cell::RefCell::new(Vec::new());
        let lookup = |pattern: &str| {
            patterns.borrow_mut().push(pattern.to_string());
            Ok(if pattern == "monospace" {
                Some("Liberation Mono".into())
            } else {
                None
            })
        };
        let result = select(Some("Missing Font"), &lookup, &lookup);
        assert_eq!(result.unwrap(), "Liberation Mono");
        assert_eq!(*patterns.borrow(), ["Missing Font", "monospace"]);
    }

    #[test]
    fn no_monospace_font_is_an_error_not_a_proportional_fallback() {
        assert!(
            select(None, |_| unreachable!(), |_| Ok(None))
                .unwrap_err()
                .contains("monospace")
        );
    }

    #[test]
    fn process_errors_are_not_hidden() {
        assert_eq!(
            select(None, |_| unreachable!(), |_| Err("fc-match missing".into())).unwrap_err(),
            "fc-match missing"
        );
    }

    #[test]
    fn blank_override_uses_the_default() {
        assert_eq!(
            select(
                Some("  "),
                |_| unreachable!(),
                |p| {
                    assert_eq!(p, "monospace");
                    Ok(Some("DejaVu Sans Mono".into()))
                },
            )
            .unwrap(),
            "DejaVu Sans Mono"
        );
    }

    #[test]
    fn fixed_width_results_are_accepted() {
        assert_eq!(
            parse_match("Liberation Mono\n100\n", false),
            Some("Liberation Mono".into())
        );
        assert_eq!(parse_match("Fixed\n110\n", true), Some("Fixed".into()));
    }

    #[test]
    fn unset_spacing_is_accepted_only_for_the_monospace_alias() {
        // Noto Sans Mono reports no spacing on many Fedora systems; the alias
        // path accepts it, but an explicit request must still be confirmed.
        assert_eq!(
            parse_match("Noto Sans Mono\n\n", true),
            Some("Noto Sans Mono".into())
        );
        assert_eq!(parse_match("Noto Sans Mono\n\n", false), None);
    }

    #[test]
    fn proportional_dual_width_and_malformed_results_are_rejected() {
        for output in [
            "DejaVu Sans\n0\n",
            "Dual\n90\n",
            "\n100\n",
            "Mono\n",
            "Mono\n100\nextra\n",
            "Mono\0\n100\n",
        ] {
            assert_eq!(parse_match(output, true), None, "{output:?}");
            assert_eq!(parse_match(output, false), None, "{output:?}");
        }
    }

    #[test]
    fn family_names_cannot_inject_fontconfig_properties() {
        assert_eq!(family_pattern(r"A-B:C,D\E"), r"A\-B\:C\,D\\E");
        // A forced spacing property can be echoed for a proportional font by
        // Fontconfig. It must never be included in our query.
        assert_eq!(family_pattern("monospace"), "monospace");
    }
}
