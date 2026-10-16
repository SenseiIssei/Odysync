//! Matching a code signature's signer against the publisher the catalog pins.
//!
//! A *valid* signature only proves that somebody with a code-signing
//! certificate signed the file. Malware is routinely validly signed — with a
//! stolen certificate or a cheap one bought under another name. What makes a
//! signature mean something here is that it is **the expected publisher's**,
//! so the check is: valid, *and* the subject's common name or organisation is
//! exactly the one the catalog names.
//!
//! Exact means exact. `CN="GitHub, Inc."` must not be satisfied by
//! `CN=GitHub, Inc. Evil`, and the comma inside the quoted value must not be
//! read as the end of the attribute — which is why this parses the
//! distinguished name rather than searching it with `contains`.

/// The `(attribute, value)` pairs of an X.500 subject such as
/// `CN="GitHub, Inc.", O="GitHub, Inc.", L=San Francisco, C=US`.
///
/// Handles both quoting styles Windows emits — a double-quoted value with
/// `""` as an escaped quote, and RFC 4514 backslash escapes.
pub fn parse_subject(subject: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = subject.chars().peekable();

    loop {
        // Attribute name, up to '='.
        let mut key = String::new();
        for c in chars.by_ref() {
            if c == '=' {
                break;
            }
            key.push(c);
        }
        let key = key.trim().to_ascii_uppercase();
        if key.is_empty() {
            break;
        }

        // Skip spaces before the value.
        while chars.peek() == Some(&' ') {
            chars.next();
        }

        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        value.push('"');
                        chars.next();
                    } else {
                        break;
                    }
                } else {
                    value.push(c);
                }
            }
            // Discard anything up to the separator.
            for c in chars.by_ref() {
                if c == ',' || c == '+' {
                    break;
                }
            }
        } else {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            value.push(next);
                        }
                    }
                    ',' | '+' => break,
                    _ => value.push(c),
                }
            }
        }

        out.push((key, value.trim().to_string()));
        if chars.peek().is_none() {
            break;
        }
    }

    out
}

/// Does a signer subject belong to `expected`?
///
/// True when the subject's CN or O equals `expected`, ignoring ASCII case and
/// surrounding whitespace. Nothing else in the subject is considered: L, S and
/// C describe where a company is registered, not who it is.
pub fn signer_matches(subject: &str, expected: &str) -> bool {
    let expected = expected.trim();
    if expected.is_empty() {
        return false;
    }
    parse_subject(subject)
        .iter()
        .filter(|(k, _)| k == "CN" || k == "O")
        .any(|(_, v)| v.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_subject_is_split_into_its_attributes() {
        let parsed = parse_subject(
            "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US",
        );
        assert_eq!(parsed[0], ("CN".into(), "Microsoft Corporation".into()));
        assert_eq!(parsed[1], ("O".into(), "Microsoft Corporation".into()));
        assert_eq!(parsed.len(), 5);
    }

    /// The comma inside a quoted value is part of the name, not a separator.
    #[test]
    fn a_quoted_value_keeps_its_comma() {
        let parsed = parse_subject(r#"CN="GitHub, Inc.", O="GitHub, Inc.", C=US"#);
        assert_eq!(parsed[0], ("CN".into(), "GitHub, Inc.".into()));
        assert_eq!(parsed[1], ("O".into(), "GitHub, Inc.".into()));
        assert_eq!(parsed[2], ("C".into(), "US".into()));
    }

    #[test]
    fn escaped_quotes_and_backslashes_are_unescaped() {
        let parsed = parse_subject(r#"CN="A ""quoted"" name", O=Comma\, Ltd"#);
        assert_eq!(parsed[0].1, r#"A "quoted" name"#);
        assert_eq!(parsed[1].1, "Comma, Ltd");
    }

    /// Real subjects, read from the installers the catalog points at.
    #[test]
    fn real_signers_match_their_catalog_pins() {
        let cases = [
            (
                r#"E=don.h@free.fr, CN="NOTEPAD++", O="NOTEPAD++", L=Saint Cloud, S=Ile de France, C=FR"#,
                "NOTEPAD++",
            ),
            (
                r#"CN="GitHub, Inc.", O="GitHub, Inc.", L=San Francisco, S=California, C=US"#,
                "GitHub, Inc.",
            ),
            (
                "CN=Johannes Schindelin, O=Johannes Schindelin, S=Nordrhein-Westfalen, C=DE",
                "Johannes Schindelin",
            ),
            (
                r#"CN="Open Source Developer, Scott Rae", O=Open Source Developer, C=GB"#,
                "Open Source Developer, Scott Rae",
            ),
            (
                "CN=OpenJS Foundation, O=OpenJS Foundation, L=San Francisco, S=California, C=US",
                "OpenJS Foundation",
            ),
            (
                "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US",
                "Microsoft Corporation",
            ),
            (
                r#"CN="Meltytech, LLC", O="Meltytech, LLC", L=Oceanside, S=California, C=US"#,
                "Meltytech, LLC",
            ),
        ];
        for (subject, pin) in cases {
            assert!(signer_matches(subject, pin), "{pin} vs {subject}");
        }
    }

    #[test]
    fn a_different_publisher_with_a_valid_signature_is_refused() {
        let subject = "CN=Totally Legit Software Ltd, O=Totally Legit Software Ltd, C=US";
        assert!(!signer_matches(subject, "Microsoft Corporation"));
    }

    /// The reason this parses instead of using `contains`.
    #[test]
    fn a_name_that_merely_contains_the_pin_is_refused() {
        assert!(!signer_matches(
            r#"CN="GitHub, Inc. Evil", O=Evil"#,
            "GitHub, Inc."
        ));
        assert!(!signer_matches(
            "CN=Not Microsoft Corporation, O=Elsewhere",
            "Microsoft Corporation"
        ));
        // Only CN and O identify the signer; a locality that happens to equal
        // the pin is not a match.
        assert!(!signer_matches(
            "CN=Someone, O=Someone, L=Redmond",
            "Redmond"
        ));
    }

    #[test]
    fn matching_ignores_case_but_an_empty_pin_matches_nothing() {
        assert!(signer_matches(
            "CN=MICROSOFT CORPORATION",
            "Microsoft Corporation"
        ));
        assert!(!signer_matches("CN=Anyone", ""));
        assert!(!signer_matches("", "Anyone"));
    }
}
