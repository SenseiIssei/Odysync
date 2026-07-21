//! Column-aware parser for winget's table output.
//!
//! winget has no machine-readable output for `upgrade`, so the table must be
//! parsed. The previous implementation split rows on runs of two-or-more
//! spaces, which breaks in two ways that both caused real damage:
//!
//!   * application names containing a double space, or a column whose value is
//!     empty, shift every following field left — so a *version* string could be
//!     read as the package **Id** and handed to `winget install`
//!   * the header row is localised, so matching on the literal text "Version"
//!     fails on any non-English Windows
//!
//! Instead we take the byte offsets of the header tokens and slice every data
//! row at those offsets. Column *positions* are language-independent, and so is
//! their order (Name, Id, Version, Available, Source), so this works on a
//! German or Japanese install without knowing a single translated word.

/// One parsed row of `winget upgrade`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub id: String,
    pub version: String,
    pub available: String,
    pub source: String,
}

/// One parsed row of `winget list`.
///
/// `winget list` prints the same table minus the Available column when nothing
/// is upgradable, and *with* it when something is — so the source is taken from
/// the last column rather than a fixed index. Only Name/Id/Version have a fixed
/// position in both shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub name: String,
    pub id: String,
    pub version: String,
    pub source: String,
}

/// Start offsets of each column, derived from the header line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    starts: Vec<usize>,
}

impl Layout {
    /// Find where each column begins, measured in *terminal display columns*.
    ///
    /// Not char indices and not bytes: winget aligns the table to rendered
    /// width, so one CJK glyph occupies two columns while being a single char.
    /// Measuring in display width is the only unit that stays in step with the
    /// padding winget actually emits.
    fn from_header(header: &str) -> Option<Layout> {
        let mut starts = Vec::new();
        let mut col = 0usize;
        let mut chars = header.chars().peekable();
        let mut in_token = false;

        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                // A token ends only on two-or-more spaces, so header labels
                // containing a single space (common in localised builds) stay
                // one column.
                if in_token && chars.peek().is_none_or(|n| n.is_whitespace()) {
                    in_token = false;
                }
            } else if !in_token {
                in_token = true;
                starts.push(col);
            }
            col += display_width(c);
        }

        // Fewer than three columns is not a table we understand.
        if starts.len() < 3 {
            return None;
        }
        Some(Layout { starts })
    }

    /// Slice a data row into its columns at the header's display offsets.
    fn split(&self, line: &str) -> Vec<String> {
        // Map each column boundary onto a char index in this particular line.
        let mut cut_points = Vec::with_capacity(self.starts.len() + 1);
        let mut next_boundary = 0usize;
        let mut col = 0usize;

        for (idx, c) in line.chars().enumerate() {
            while next_boundary < self.starts.len() && col >= self.starts[next_boundary] {
                cut_points.push(idx);
                next_boundary += 1;
            }
            col += display_width(c);
        }
        let len = line.chars().count();
        while cut_points.len() < self.starts.len() {
            cut_points.push(len);
        }
        cut_points.push(len);

        let chars: Vec<char> = line.chars().collect();
        (0..self.starts.len())
            .map(|i| {
                let start = cut_points[i].min(len);
                let end = cut_points[i + 1].min(len);
                if start >= end {
                    String::new()
                } else {
                    chars[start..end]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string()
                }
            })
            .collect()
    }
}

/// Rendered width of one character, in terminal columns.
///
/// Full-width CJK, Hangul and emoji occupy two columns; combining marks occupy
/// none. This mirrors how winget pads its table.
fn display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Does this cell look like a real winget package identifier?
///
/// Two shapes are legitimate: `Publisher.Package` from the winget source, and
/// a Store product code such as `9WZDNCRFJ3TJ`. Both are ASCII, unspaced, and
/// drawn from a small character set.
///
/// This exists because localised summary text sliced at a column boundary can
/// otherwise look plausible — German winget yields the fragment `erfügbar.`,
/// which has no whitespace and even contains a dot. Requiring ASCII and
/// rejecting a leading or trailing dot rules it out.
fn looks_like_package_id(id: &str) -> bool {
    if id.is_empty() || !id.is_ascii() {
        return false;
    }
    if id.starts_with('.') || id.ends_with('.') || id.starts_with('-') {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// True for the run of dashes winget prints under the header.
fn is_separator(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().all(|c| c == '-' || c == '─' || c == '=')
}

/// Strip the progress spinner winget writes to stdout before the table.
///
/// It emits backspaces and `\r` to animate in place; if left in, the header
/// detection latches onto spinner glyphs instead of the real header.
fn clean(line: &str) -> String {
    // Everything before the last carriage return was overwritten on screen.
    let visible = line.rsplit('\r').next().unwrap_or(line);
    // Backspaces are the other half of the animation and carry no content.
    visible
        .chars()
        .filter(|c| *c != '\u{8}')
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Parse a winget table into its raw cells, one `Vec<String>` per data row.
///
/// Every winget table (`upgrade`, `list`, …) shares this shape; only the
/// meaning of the columns after Name/Id/Version differs, so the column-offset
/// logic — the part that has to be right — lives here once.
fn parse_cells(output: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = output.lines().map(clean).collect();

    // The header is the line immediately preceding the dashed separator. Anchor
    // on the separator rather than on line numbers, because winget prints a
    // variable number of progress and banner lines first.
    let sep_idx = match lines.iter().position(|l| is_separator(l)) {
        Some(i) if i > 0 => i,
        _ => return Vec::new(),
    };

    let Some(layout) = Layout::from_header(&lines[sep_idx - 1]) else {
        return Vec::new();
    };

    let mut rows = Vec::new();

    for line in &lines[sep_idx + 1..] {
        // A blank line ends the table. winget appends a *second* table for
        // packages needing explicit targeting, and its header row would
        // otherwise be parsed as a package named "Name" with id "Id".
        if line.trim().is_empty() || is_separator(line) {
            break;
        }

        let cols = layout.split(line);
        let name = cols.first().cloned().unwrap_or_default();
        let id = cols.get(1).cloned().unwrap_or_default();

        // The table ends at the first line that is not a package row.
        //
        // winget follows the table with a summary ("37 upgrades available.")
        // and an explanatory paragraph, then sometimes a *second* table for
        // packages needing explicit targeting. Long prose spans the column
        // boundaries, so it yields a non-empty Id cell and would otherwise be
        // accepted as a package — on a German install this produced entries
        // like "Mindestens 1 Paket verfügt über…".
        //
        // A winget PackageIdentifier has a narrow, ASCII-only shape, which is
        // what separates real rows from wrapped prose in any language.
        if name.is_empty() || !looks_like_package_id(&id) {
            break;
        }

        rows.push(cols);
    }

    rows
}

/// Parse a `winget upgrade` table into rows.
///
/// Returns an empty vector when no table is present (for example "No installed
/// package found matching input criteria."), which callers treat as "nothing to
/// do" rather than an error.
pub fn parse(output: &str) -> Vec<Row> {
    parse_cells(output)
        .into_iter()
        .map(|cols| Row {
            name: cols.first().cloned().unwrap_or_default(),
            id: cols.get(1).cloned().unwrap_or_default(),
            version: cols.get(2).cloned().unwrap_or_default(),
            available: cols.get(3).cloned().unwrap_or_default(),
            source: cols.get(4).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Parse a `winget list` table into rows.
///
/// Uses the same column-offset machinery as [`parse`]; the only difference is
/// that the Available column may be absent, so the source is read from the last
/// column instead of a fixed index.
pub fn parse_list(output: &str) -> Vec<ListRow> {
    parse_cells(output)
        .into_iter()
        .map(|cols| ListRow {
            name: cols.first().cloned().unwrap_or_default(),
            id: cols.get(1).cloned().unwrap_or_default(),
            version: cols.get(2).cloned().unwrap_or_default(),
            // Name/Id/Version/[Available]/Source: with or without Available the
            // source is always last, and a 3-column table has none at all.
            source: if cols.len() >= 4 {
                cols.last().cloned().unwrap_or_default()
            } else {
                String::new()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_english_upgrade_table() {
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget
7-Zip                7zip.7zip               23.01        24.09        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Mozilla Firefox");
        assert_eq!(rows[0].id, "Mozilla.Firefox");
        assert_eq!(rows[0].version, "140.0.1");
        assert_eq!(rows[0].available, "141.0");
        assert_eq!(rows[0].source, "winget");
        assert_eq!(rows[1].id, "7zip.7zip");
    }

    #[test]
    fn a_name_containing_double_spaces_does_not_shift_columns() {
        // The old splitter read "1.2.3" as the Id here and would have run
        // `winget install --id 1.2.3`.
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Weird  App  Name     Vendor.WeirdApp         1.2.3        1.2.4        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Weird  App  Name");
        assert_eq!(rows[0].id, "Vendor.WeirdApp");
        assert_eq!(rows[0].version, "1.2.3");
    }

    #[test]
    fn localised_headers_parse_by_position() {
        // German winget: the words differ, the column order does not.
        let out = "\
Name                 ID                      Version      Verfügbar    Quelle
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "Mozilla.Firefox");
        assert_eq!(rows[0].available, "141.0");
    }

    #[test]
    fn an_empty_version_cell_keeps_later_columns_aligned() {
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Ghost App            Vendor.Ghost                         2.0.0        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "");
        assert_eq!(rows[0].available, "2.0.0");
        assert_eq!(rows[0].source, "winget");
    }

    #[test]
    fn trailing_summary_lines_are_not_treated_as_packages() {
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget
2 upgrades available.
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "Mozilla.Firefox");
    }

    #[test]
    fn localised_summary_prose_is_not_parsed_as_a_package() {
        // Real output from a German Windows install. The prose is long enough
        // to reach the Id column, so an empty-cell check alone lets it through.
        let out = "\
Name                 Id                      Version      Verfügbar    Quelle
-------------------------------------------------------------------------------
Notepad++ (64-bit)   Notepad++.Notepad++     8.9.2        8.9.7        winget
37 Aktualisierungen verfügbar.
Mindestens 1 Paket verfügt über eine Version, die nicht bestimmt werden kann.
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "Notepad++.Notepad++");
    }

    #[test]
    fn package_id_shape_accepts_real_ids_and_rejects_prose_fragments() {
        assert!(looks_like_package_id("Mozilla.Firefox"));
        assert!(looks_like_package_id("Notepad++.Notepad++"));
        assert!(looks_like_package_id("7zip.7zip"));
        // Microsoft Store product codes carry no dot.
        assert!(looks_like_package_id("9WZDNCRFJ3TJ"));
        assert!(looks_like_package_id("Microsoft.VCRedist.2015+.x64"));

        // Fragments of localised summary text.
        assert!(!looks_like_package_id("erfügbar."));
        assert!(!looks_like_package_id(". Some packages"));
        assert!(!looks_like_package_id("available."));
        assert!(!looks_like_package_id(""));
        assert!(!looks_like_package_id("has a version"));
    }

    #[test]
    fn an_english_summary_line_ends_the_table() {
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
7-Zip                7zip.7zip               23.01        24.09        winget
37 upgrades available. Some packages require explicit targeting to upgrade.
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "7zip.7zip");
    }

    #[test]
    fn a_second_table_following_prose_is_not_merged_in() {
        // No blank line separates them in real winget output, so the prose
        // guard is what has to stop the parse here.
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mattermost           Mattermost.Mattermost   6.0.4        6.2.2        winget
1 package has a version that cannot be determined.
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mattermost           Mattermost.Mattermost   Unknown                   winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].available, "6.2.2");
    }

    #[test]
    fn the_explicit_targeting_second_table_is_not_merged_in() {
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget

1 package has a pin
Name                 Id                      Version
-------------------------------------------------------------------------------
Pinned Thing         Vendor.Pinned           1.0.0
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "Mozilla.Firefox");
    }

    #[test]
    fn progress_spinner_output_before_the_table_is_ignored() {
        let out = "\
   \r  -\r  \\\r  |\rName                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Mozilla Firefox");
    }

    #[test]
    fn no_table_yields_no_rows_rather_than_an_error() {
        assert!(parse("No installed package found matching input criteria.").is_empty());
        assert!(parse("").is_empty());
    }

    #[test]
    fn wide_glyph_names_do_not_desynchronise_columns() {
        // Byte offsets would break here; char offsets hold.
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
メモ帳アプリ          Vendor.Notepad          1.0.0        1.1.0        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "Vendor.Notepad");
        assert_eq!(rows[0].version, "1.0.0");
    }

    #[test]
    fn parses_a_winget_list_table_without_an_available_column() {
        let out = "\
Name                 Id                      Version      Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         141.0        winget
7-Zip                7zip.7zip               24.09        winget
Some Store App       9WZDNCRFJ3TJ            2024.1       msstore
";
        let rows = parse_list(out);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "Mozilla Firefox");
        assert_eq!(rows[0].id, "Mozilla.Firefox");
        assert_eq!(rows[0].version, "141.0");
        assert_eq!(rows[0].source, "winget");
        assert_eq!(rows[2].source, "msstore");
    }

    #[test]
    fn winget_list_source_is_read_from_the_last_column_when_available_is_present() {
        // winget re-adds the Available column to `list` as soon as any package
        // has an upgrade; the source must not be read out of it.
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mozilla Firefox      Mozilla.Firefox         140.0.1      141.0        winget
7-Zip                7zip.7zip               24.09                     winget
";
        let rows = parse_list(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].version, "140.0.1");
        assert_eq!(rows[0].source, "winget");
        assert_eq!(rows[1].version, "24.09");
        assert_eq!(rows[1].source, "winget");
    }

    #[test]
    fn a_listed_name_containing_a_double_space_does_not_shift_columns() {
        let out = "\
Name                 Id                      Version      Source
-------------------------------------------------------------------------------
Weird  App  Name     Vendor.WeirdApp         1.2.3        winget
";
        let rows = parse_list(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Weird  App  Name");
        assert_eq!(rows[0].id, "Vendor.WeirdApp");
        assert_eq!(rows[0].version, "1.2.3");
        assert_eq!(rows[0].source, "winget");
    }

    #[test]
    fn a_list_table_with_no_source_column_yields_an_empty_source() {
        let out = "\
Name                 Id                      Version
-------------------------------------------------------------------------------
Local Thing          Vendor.Local            1.0.0
";
        let rows = parse_list(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "1.0.0");
        assert_eq!(rows[0].source, "");
    }

    #[test]
    fn no_list_table_yields_no_rows() {
        assert!(parse_list("No installed package found matching input criteria.").is_empty());
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn unknown_version_rows_are_returned_verbatim_for_policy_to_reject() {
        // The parser must not filter; policy owns that decision.
        let out = "\
Name                 Id                      Version      Available    Source
-------------------------------------------------------------------------------
Mystery App          Vendor.Mystery          Unknown      3.0.0        winget
";
        let rows = parse(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "Unknown");
    }
}
