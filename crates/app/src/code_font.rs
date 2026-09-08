//! Resolve a concrete Linux code font before GPUI sees the family name.
//! GPUI's missing-family fallback is a UI font, not necessarily monospace.
use std::process::{Command, Stdio};

pub fn resolve(requested: Option<&str>) -> Result<String, String> {
    select(requested, |pattern| {
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
        Ok(parse_match(&output))
    })
}

fn select(
    requested: Option<&str>,
    mut lookup: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<String, String> {
    let matched = match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(family) => lookup(&family_pattern(family))?,
        None => None,
    };
    if let Some(family) = matched {
        return Ok(family);
    }
    lookup("monospace")?.ok_or_else(|| {
        "no usable monospace font found; install dejavu-sans-mono-fonts (Fedora), \
         fonts-dejavu-core (Debian/Ubuntu), or ttf-dejavu (Arch)".to_string()
    })
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

fn parse_match(output: &str) -> Option<String> {
    let mut lines = output.lines();
    let family = lines.next()?;
    let spacing = lines.next()?.trim();
    // Do not request :spacing=100: Fontconfig can copy that property into a
    // proportional fallback's result, making it look falsely monospace.
    if family.is_empty()
        || family.chars().any(char::is_control)
        || !matches!(spacing, "100" | "110")
        || lines.next().is_some()
    {
        return None;
    }
    Some(family.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_system_monospace_instead_of_a_hardcoded_family() {
        let result = select(None, |pattern| {
            assert_eq!(pattern, "monospace");
            Ok(Some("Liberation Mono".into()))
        });
        assert_eq!(result.unwrap(), "Liberation Mono");
    }

    #[test]
    fn installed_explicit_font_is_preserved() {
        assert_eq!(
            select(Some("Noto Sans Mono"), |pattern| {
                assert_eq!(pattern, "Noto Sans Mono");
                Ok(Some("Noto Sans Mono".into()))
            })
            .unwrap(),
            "Noto Sans Mono"
        );
    }

    #[test]
    fn proportional_match_retries_the_system_monospace_alias() {
        let mut patterns = Vec::new();
        let result = select(Some("Missing Font"), |pattern| {
            patterns.push(pattern.to_string());
            Ok(if pattern == "monospace" {
                Some("Liberation Mono".into())
            } else {
                None
            })
        });
        assert_eq!(result.unwrap(), "Liberation Mono");
        assert_eq!(patterns, ["Missing Font", "monospace"]);
    }

    #[test]
    fn no_monospace_font_is_an_error_not_a_proportional_fallback() {
        assert!(select(None, |_| Ok(None)).unwrap_err().contains("monospace"));
    }

    #[test]
    fn process_errors_are_not_hidden() {
        assert_eq!(
            select(None, |_| Err("fc-match missing".into())).unwrap_err(),
            "fc-match missing"
        );
    }

    #[test]
    fn blank_override_uses_the_default() {
        assert_eq!(
            select(Some("  "), |p| {
                assert_eq!(p, "monospace");
                Ok(Some("DejaVu Sans Mono".into()))
            })
            .unwrap(),
            "DejaVu Sans Mono"
        );
    }

    #[test]
    fn fixed_width_results_are_accepted() {
        assert_eq!(
            parse_match("Liberation Mono\n100\n"),
            Some("Liberation Mono".into())
        );
        assert_eq!(parse_match("Fixed\n110\n"), Some("Fixed".into()));
    }

    #[test]
    fn proportional_dual_width_and_malformed_results_are_rejected() {
        for output in [
            "DejaVu Sans\n\n",
            "Sans\n0\n",
            "Dual\n90\n",
            "\n100\n",
            "Mono\n",
            "Mono\n100\nextra\n",
            "Mono\0\n100\n",
        ] {
            assert_eq!(parse_match(output), None, "{output:?}");
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
