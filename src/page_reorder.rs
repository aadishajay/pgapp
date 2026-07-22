//! Edits a page's top-level components within a `.pgapp` file's raw
//! text: reorder, delete, append, or tweak one component's label/
//! columns — the file-editing half of the App Builder's page editor
//! (see the `/:workspace/:app/admin/pages/:page/...` admin routes in
//! `server.rs`, which also keep `pgapp_meta` in sync by re-syncing the
//! whole app after every edit — see `AppEntry::reload`).
//!
//! Deliberately line-based text splices, never a parse-and-regenerate:
//! `markup::page_component_start_lines` gives real start lines (reusing
//! the actual grammar, so it's never out of sync with what the parser
//! considers a component boundary), and every function here cuts/
//! rewrites along those lines — untouched components keep their exact
//! original text, including formatting and inline comments. Single-file
//! apps only (same restriction as `page_component_start_lines`).

use anyhow::{Context, Result};

use crate::markup;

/// Each component's 0-based `[start, end)` line range within `source`
/// (comment-adjusted — see `reorder_page`'s doc), plus the 0-based line
/// the page's closing `}` is on. Shared by every function below.
fn component_bounds(source: &str, page_name: &str) -> Result<(Vec<(usize, usize)>, usize)> {
    let (start_lines, closing_line) = markup::page_component_start_lines(source, page_name)
        .with_context(|| format!("failed to locate page '{page_name}'"))?;
    let n = start_lines.len();
    let lines: Vec<&str> = source.lines().collect();

    // A component's chunk starts at its own token line, walked backward
    // over any immediately-preceding comment lines (no blank line in
    // between) so a comment describing it stays attached to it.
    let adjusted_start = |token_line: u32| -> usize {
        let mut idx = (token_line - 1) as usize; // 1-based -> 0-based
        while idx > 0 {
            let prev = lines[idx - 1].trim_start();
            if prev.starts_with('#') {
                idx -= 1;
            } else {
                break;
            }
        }
        idx
    };

    let starts: Vec<usize> = start_lines.iter().map(|&l| adjusted_start(l)).collect();
    let end_of_page_body = (closing_line - 1) as usize; // 0-based index of the line the closing '}' is on

    let bounds: Vec<(usize, usize)> = (0..n)
        .map(|i| {
            let start = starts[i];
            let end = if i + 1 < n { starts[i + 1] } else { end_of_page_body };
            (start, end)
        })
        .collect();
    Ok((bounds, end_of_page_body))
}

pub(crate) fn join_lines(lines: &[&str], source_ended_in_newline: bool) -> String {
    let mut out = lines.join("\n");
    if source_ended_in_newline {
        out.push('\n');
    }
    out
}

/// Every top-level page's 0-based `[start, end)` line range (comment-
/// adjusted, same reasoning as `component_bounds`), plus the 0-based
/// line the *app's* closing `}` is on — used by `add_page`/`delete_page`.
fn page_bounds(source: &str) -> Result<(Vec<(String, usize, usize)>, usize)> {
    let (starts, closing_line) = markup::app_page_start_lines(source).context("failed to parse app")?;
    let n = starts.len();
    let lines: Vec<&str> = source.lines().collect();

    let adjusted_start = |token_line: u32| -> usize {
        let mut idx = (token_line - 1) as usize;
        while idx > 0 {
            let prev = lines[idx - 1].trim_start();
            if prev.starts_with('#') {
                idx -= 1;
            } else {
                break;
            }
        }
        idx
    };

    let adjusted: Vec<usize> = starts.iter().map(|(_, l)| adjusted_start(*l)).collect();
    let end_of_app_body = (closing_line - 1) as usize;

    let bounds: Vec<(String, usize, usize)> = (0..n)
        .map(|i| {
            let start = adjusted[i];
            let end = if i + 1 < n { adjusted[i + 1] } else { end_of_app_body };
            (starts[i].0.clone(), start, end)
        })
        .collect();
    Ok((bounds, end_of_app_body))
}

/// Appends a brand-new, empty `page "<name>" { }` block just before the
/// app's closing `}` — the App Builder's "Add Page". The new page has
/// no components yet; add them afterward the same way any other page's
/// components are added.
pub fn add_page(source: &str, name: &str) -> Result<String> {
    let (bounds, end_of_app_body) = page_bounds(source)?;
    if bounds.iter().any(|(n, _, _)| n == name) {
        anyhow::bail!("a page named '{name}' already exists in this app");
    }
    let lines: Vec<&str> = source.lines().collect();
    let new_page = format!("\n  page \"{}\" {{\n  }}", escape_string(name));

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 3);
    new_lines.extend_from_slice(&lines[..end_of_app_body]);
    new_lines.extend(new_page.lines());
    new_lines.extend_from_slice(&lines[end_of_app_body..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Removes an entire page block (and every component on it) by name.
pub fn delete_page(source: &str, page_name: &str) -> Result<String> {
    let (bounds, _) = page_bounds(source)?;
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == page_name)
        .ok_or_else(|| anyhow::anyhow!("no page named '{page_name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Reorders `page_name`'s components in `source` so that the component
/// currently at index `new_order[i]` becomes the `i`th one — e.g. an
/// original order of `[A, B, C]` reordered by `new_order = [2, 0, 1]`
/// becomes `[C, A, B]`. `new_order` must be a permutation of `0..n`
/// where `n` is the page's current component count.
pub fn reorder_page(source: &str, page_name: &str, new_order: &[usize]) -> Result<String> {
    let (bounds, _) = component_bounds(source, page_name)?;
    let n = bounds.len();
    if new_order.len() != n {
        anyhow::bail!("new order has {} entries but page '{page_name}' has {n} components", new_order.len());
    }
    let mut seen = vec![false; n];
    for &i in new_order {
        if i >= n {
            anyhow::bail!("index {i} out of range for page '{page_name}' ({n} components)");
        }
        if std::mem::replace(&mut seen[i], true) {
            anyhow::bail!("index {i} repeated in new order — must be a permutation of 0..{n}");
        }
    }

    let lines: Vec<&str> = source.lines().collect();
    let chunks: Vec<&[&str]> = bounds.iter().map(|&(start, end)| &lines[start..end]).collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..bounds[0].0]);
    for &i in new_order {
        new_lines.extend_from_slice(chunks[i]);
    }
    new_lines.extend_from_slice(&lines[bounds[n - 1].1..]);

    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Removes the component at `idx` from `page_name` entirely.
pub fn delete_component(source: &str, page_name: &str, idx: usize) -> Result<String> {
    let (bounds, _) = component_bounds(source, page_name)?;
    let n = bounds.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for page '{page_name}' ({n} components)");
    }
    let lines: Vec<&str> = source.lines().collect();
    let (start, end) = bounds[idx];

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);

    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Appends a brand-new component (`new_component`, caller-formatted
/// markup text for exactly one component, no trailing newline needed)
/// to the end of `page_name`'s body, just before its closing `}` — the
/// new component lands last and can be drag-reordered into place from
/// there like any other.
pub fn append_component(source: &str, page_name: &str, new_component: &str) -> Result<String> {
    let (_, end_of_page_body) = component_bounds(source, page_name)?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..end_of_page_body]);
    new_lines.extend(new_component.lines());
    new_lines.extend_from_slice(&lines[end_of_page_body..]);

    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Same escaping the markup lexer accepts back: `\` and `"` doubled up.
pub(crate) fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Returns component `idx`'s exact source text, unindented from
/// however it appears in `source` — used to prefill the App Builder's
/// full-property "Edit" panel with the real, current markup for that
/// one component (any kind, any attribute), rather than a fixed set of
/// structured fields.
pub fn component_source(source: &str, page_name: &str, idx: usize) -> Result<String> {
    let (bounds, _) = component_bounds(source, page_name)?;
    let n = bounds.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for page '{page_name}' ({n} components)");
    }
    let (start, end) = bounds[idx];
    let lines: Vec<&str> = source.lines().collect();
    Ok(lines[start..end].join("\n"))
}

/// Replaces component `idx` outright with `new_component` (caller-
/// formatted markup text for exactly one component) — the App
/// Builder's full-property "Edit": the whole block is swapped, so any
/// attribute of any kind can change, not just a fixed subset. The
/// caller is expected to validate the result (e.g. via
/// `markup::parse_app` on the whole file) before persisting it, since a
/// hand-edited block can easily be malformed.
pub fn replace_component(source: &str, page_name: &str, idx: usize, new_component: &str) -> Result<String> {
    let (bounds, _) = component_bounds(source, page_name)?;
    let n = bounds.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for page '{page_name}' ({n} components)");
    }
    let (start, end) = bounds[idx];
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend(new_component.lines());
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Renames page `old_name` to `new_name`: the page's own declaration
/// line changes, and every `-> page <old_name>` reference elsewhere in
/// the file (nav items, report `link:`, `link` components) is rewritten
/// to match, since a page target is a bare identifier that has to keep
/// matching the (unquoted) page name for those links to keep working.
/// Matches on word boundaries so e.g. renaming "Task" can't clobber a
/// reference to "TaskDetail" — the one blind spot is a `text`/label
/// string that just happens to contain the literal substring
/// `page <old_name>` as ordinary display text, which would also get
/// rewritten; considered an acceptable, rare edge case for a text-splice
/// based editor.
pub fn rename_page(source: &str, old_name: &str, new_name: &str) -> Result<String> {
    let (starts, _) = markup::app_page_start_lines(source).context("failed to parse app")?;
    if old_name != new_name && starts.iter().any(|(n, _)| n == new_name) {
        anyhow::bail!("a page named '{new_name}' already exists in this app");
    }
    let (_, token_line) = starts
        .into_iter()
        .find(|(n, _)| n == old_name)
        .ok_or_else(|| anyhow::anyhow!("no page named '{old_name}' in this app"))?;
    let idx = (token_line - 1) as usize;
    let lines: Vec<&str> = source.lines().collect();
    let line = lines[idx];
    let needle = format!("\"{}\"", escape_string(old_name));
    let pos = line
        .find(&needle)
        .ok_or_else(|| anyhow::anyhow!("couldn't locate page '{old_name}'s declaration to rename"))?;
    let replaced_line = format!("{}\"{}\"{}", &line[..pos], escape_string(new_name), &line[pos + needle.len()..]);

    let new_lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| if i == idx { replaced_line.clone() } else { l.to_string() })
        .collect();
    let joined = new_lines.join("\n");
    let renamed = replace_page_target_refs(&joined, old_name, new_name);
    Ok(if source.ends_with('\n') && !renamed.ends_with('\n') {
        format!("{renamed}\n")
    } else {
        renamed
    })
}

/// Rewrites every word-boundary-matched `page <old_name>` occurrence
/// (as used after `->` in nav items, report `link:`, and `link`
/// components) to `page <new_name>`. Never touches a *quoted* page
/// name (a page's own declaration, `page "Name" {`), since the needle
/// has no leading quote.
fn replace_page_target_refs(source: &str, old_name: &str, new_name: &str) -> String {
    let needle = format!("page {old_name}");
    let mut result = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(pos) = rest.find(needle.as_str()) {
        let before = &rest[..pos];
        let after = &rest[pos + needle.len()..];
        let prev_ok = before.chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let next_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_');
        result.push_str(before);
        if prev_ok && next_ok {
            result.push_str("page ");
            result.push_str(new_name);
        } else {
            result.push_str(&needle);
        }
        rest = after;
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"app "Demo" {
  page "Target" {
    text "first"
    # a comment right above second
    text "second"
    text "third"
  }
}
"#;

    #[test]
    fn reorders_components_and_keeps_attached_comments() {
        let out = reorder_page(SRC, "Target", &[2, 0, 1]).unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "third"
    text "first"
    # a comment right above second
    text "second"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn identity_order_leaves_the_file_byte_identical() {
        let out = reorder_page(SRC, "Target", &[0, 1, 2]).unwrap();
        assert_eq!(out, SRC);
    }

    #[test]
    fn rejects_a_non_permutation() {
        assert!(reorder_page(SRC, "Target", &[0, 1]).is_err(), "wrong length");
        assert!(reorder_page(SRC, "Target", &[0, 1, 1]).is_err(), "repeated index");
        assert!(reorder_page(SRC, "Target", &[0, 1, 5]).is_err(), "out of range");
    }

    #[test]
    fn rejects_an_unknown_page() {
        assert!(reorder_page(SRC, "Nope", &[0, 1, 2]).is_err());
    }

    #[test]
    fn reorders_a_page_amid_other_app_content_untouched() {
        let src = r#"app "Demo" {
  entity "t" { field id: id field name: text }

  page "Other" {
    text "leave me alone"
  }

  page "Target" {
    text "A"
    text "B"
  }

  page "Later" {
    text "also untouched"
  }
}
"#;
        let out = reorder_page(src, "Target", &[1, 0]).unwrap();
        let expected = r#"app "Demo" {
  entity "t" { field id: id field name: text }

  page "Other" {
    text "leave me alone"
  }

  page "Target" {
    text "B"
    text "A"
  }

  page "Later" {
    text "also untouched"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn deletes_a_component_and_keeps_its_attached_comment_gone_too() {
        let out = delete_component(SRC, "Target", 1).unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "first"
    text "third"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn deletes_the_last_component() {
        let out = delete_component(SRC, "Target", 2).unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "first"
    # a comment right above second
    text "second"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn rejects_deleting_an_out_of_range_index() {
        assert!(delete_component(SRC, "Target", 3).is_err());
    }

    #[test]
    fn appends_a_new_component_before_the_closing_brace() {
        let out = append_component(SRC, "Target", "    text \"fourth\"").unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "first"
    # a comment right above second
    text "second"
    text "third"
    text "fourth"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn adds_a_new_empty_page_before_the_apps_closing_brace() {
        let out = add_page(SRC, "NewPage").unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "first"
    # a comment right above second
    text "second"
    text "third"
  }

  page "NewPage" {
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn rejects_adding_a_page_whose_name_already_exists() {
        assert!(add_page(SRC, "Target").is_err());
    }

    #[test]
    fn deletes_a_whole_page_and_its_components() {
        let src = r#"app "Demo" {
  page "Other" {
    text "leave me alone"
  }

  page "Target" {
    text "A"
    text "B"
  }

  page "Later" {
    text "also untouched"
  }
}
"#;
        let out = delete_page(src, "Target").unwrap();
        let expected = r#"app "Demo" {
  page "Other" {
    text "leave me alone"
  }

  page "Later" {
    text "also untouched"
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn rejects_deleting_an_unknown_page() {
        assert!(delete_page(SRC, "Nope").is_err());
    }

    #[test]
    fn returns_a_components_exact_source_text() {
        let out = component_source(SRC, "Target", 1).unwrap();
        assert_eq!(out, "    # a comment right above second\n    text \"second\"");
    }

    #[test]
    fn replaces_a_component_outright_with_new_markup() {
        let out = replace_component(SRC, "Target", 2, "    report \"New\" of items {\n      columns: name\n    }").unwrap();
        let expected = r#"app "Demo" {
  page "Target" {
    text "first"
    # a comment right above second
    text "second"
    report "New" of items {
      columns: name
    }
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn renames_a_page_and_its_own_link_targets() {
        let src = r#"app "Demo" {
  nav {
    item "Go" -> page Target
  }

  page "Target" {
    text "first"
  }

  page "Other" {
    link "Back" -> page Target
  }
}
"#;
        let out = rename_page(src, "Target", "Renamed").unwrap();
        let expected = r#"app "Demo" {
  nav {
    item "Go" -> page Renamed
  }

  page "Renamed" {
    text "first"
  }

  page "Other" {
    link "Back" -> page Renamed
  }
}
"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn rename_page_does_not_clobber_a_longer_identifier_sharing_a_prefix() {
        let src = r#"app "Demo" {
  page "Task" {
    text "a"
  }

  page "TaskDetail" {
    link "Go" -> page Task
    link "Detail" -> page TaskDetail
  }
}
"#;
        let out = rename_page(src, "Task", "Job").unwrap();
        assert!(out.contains("page \"Job\""));
        assert!(out.contains("-> page Job"));
        assert!(out.contains("-> page TaskDetail"), "must not have rewritten TaskDetail's own reference");
        assert!(out.contains("page \"TaskDetail\""));
    }

    #[test]
    fn rejects_renaming_to_an_existing_page_name() {
        assert!(rename_page(SRC, "Target", "Target").is_ok());
        let src = r#"app "Demo" {
  page "A" { text "x" }
  page "B" { text "y" }
}
"#;
        assert!(rename_page(src, "A", "B").is_err());
    }

    #[test]
    fn rejects_renaming_an_unknown_page() {
        assert!(rename_page(SRC, "Nope", "Whatever").is_err());
    }
}
