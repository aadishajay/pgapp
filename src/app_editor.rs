//! Edits an app's top-level, app-wide constructs within a `.pgapp`
//! file's raw text — entities, queries, the `nav` block's top-level
//! items, and the theme/icons/chart_lib/auth settings — the App
//! Builder's "full data model" counterpart to `page_reorder.rs`'s
//! page/component splices.
//!
//! Same discipline as `page_reorder.rs`: line-based text splices, never
//! a parse-and-regenerate, reusing the real parser's own app-body walk
//! (`markup::app_entity_start_lines`/`app_query_start_lines`/
//! `app_nav_block_lines`/`app_settings_lines`) so a splice can never
//! disagree with the parser about where a block starts or ends —
//! untouched blocks keep their exact original text, including
//! formatting and inline comments. Single-file apps only, same
//! restriction as `page_reorder.rs`.

use anyhow::{Context, Result};

use crate::markup;
use crate::page_reorder::join_lines;

/// Turns a list of (name, 1-based start line, 1-based end line
/// inclusive) triples — as `markup::app_entity_start_lines`/
/// `app_query_start_lines` return — into each block's 0-based
/// `[start, end)` line range. Each block's *own* end line is used
/// directly (not "the next one's start," and not "the app's own
/// closing line") since an entity or query can have other blocks —
/// most commonly a `page` — declared after it in the file.
fn bounds_from_starts(starts: &[(String, u32, u32)]) -> Vec<(String, usize, usize)> {
    starts
        .iter()
        .map(|(name, start, end)| (name.clone(), (*start - 1) as usize, *end as usize))
        .collect()
}

// ---- entities ----

/// Appends a brand-new entity block just before the app's closing `}`
/// — `new_entity_text` is caller-formatted markup for exactly one
/// `entity "..." { ... }` block.
pub fn add_entity(source: &str, new_entity_text: &str) -> Result<String> {
    let (_, closing_line) = markup::app_entity_start_lines(source).context("failed to parse app")?;
    let end_of_app_body = (closing_line - 1) as usize;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..end_of_app_body]);
    new_lines.extend(new_entity_text.lines());
    new_lines.extend_from_slice(&lines[end_of_app_body..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Removes entity `name`'s whole block. Metadata cleanup (and the
/// deliberate choice to never touch its physical table) happens at the
/// next sync — see `meta::sync_app`'s own entity-cleanup pass.
pub fn delete_entity(source: &str, name: &str) -> Result<String> {
    let (starts, _) = markup::app_entity_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no entity named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Entity `name`'s exact current source text — prefills the "Edit as
/// raw markup" fallback and the structured field-list editor alike.
pub fn entity_source(source: &str, name: &str) -> Result<String> {
    let (starts, _) = markup::app_entity_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no entity named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();
    Ok(lines[start..end].join("\n"))
}

/// Replaces entity `name`'s whole block outright with
/// `new_entity_text` — the structured field editor's "Save": the whole
/// block is regenerated and swapped, same as
/// `page_reorder::replace_component`. The caller validates the result
/// (`markup::parse_app` on the whole file) before persisting.
pub fn replace_entity(source: &str, name: &str, new_entity_text: &str) -> Result<String> {
    let (starts, _) = markup::app_entity_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no entity named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend(new_entity_text.lines());
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

// ---- queries ----

/// Appends a brand-new app-level named query just before the app's
/// closing `}`.
pub fn add_query(source: &str, new_query_text: &str) -> Result<String> {
    let (_, closing_line) = markup::app_query_start_lines(source).context("failed to parse app")?;
    let end_of_app_body = (closing_line - 1) as usize;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..end_of_app_body]);
    new_lines.extend(new_query_text.lines());
    new_lines.extend_from_slice(&lines[end_of_app_body..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Removes query `name`'s whole block. If anything still references
/// it (an entity `from query`, a report/chart/region/LOV), the next
/// sync's own validation rejects the result with a clear error, the
/// same way deleting a still-referenced page or entity would.
pub fn delete_query(source: &str, name: &str) -> Result<String> {
    let (starts, _) = markup::app_query_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no query named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Query `name`'s exact current source text.
pub fn query_source(source: &str, name: &str) -> Result<String> {
    let (starts, _) = markup::app_query_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no query named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();
    Ok(lines[start..end].join("\n"))
}

/// Replaces query `name`'s whole block outright with `new_query_text`.
pub fn replace_query(source: &str, name: &str, new_query_text: &str) -> Result<String> {
    let (starts, _) = markup::app_query_start_lines(source).context("failed to parse app")?;
    let bounds = bounds_from_starts(&starts);
    let (_, start, end) = bounds
        .into_iter()
        .find(|(n, _, _)| n == name)
        .ok_or_else(|| anyhow::anyhow!("no query named '{name}' in this app"))?;
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend(new_query_text.lines());
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

// ---- nav (top-level items only — a nested submenu item is edited as
// one opaque chunk via the raw-markup fallback, same "not covered yet"
// treatment as anything else the structured editor doesn't have a
// dedicated control for) ----

/// Each top-level nav item's 0-based `[start, end)` line range, plus
/// the nav block's own 0-based opening-line index (the line right
/// after which a brand-new first item would go) and closing-brace
/// line index. `None` when the app has no `nav` block at all.
struct NavBounds {
    items: Vec<(usize, usize)>,
    close_idx: usize,
}

fn nav_bounds(source: &str) -> Result<Option<NavBounds>> {
    let Some(nav) = markup::app_nav_block_lines(source).context("failed to parse app")? else {
        return Ok(None);
    };
    let n = nav.item_start_lines.len();
    let close_idx = (nav.close_line - 1) as usize;
    let items: Vec<(usize, usize)> = (0..n)
        .map(|i| {
            let start = (nav.item_start_lines[i] - 1) as usize;
            let end = if i + 1 < n { (nav.item_start_lines[i + 1] - 1) as usize } else { close_idx };
            (start, end)
        })
        .collect();
    Ok(Some(NavBounds { items, close_idx }))
}

/// Appends a brand-new top-level nav item (`new_item_text`,
/// caller-formatted markup for exactly one `item ...` line/block) —
/// creates the `nav { }` block itself, right after the app's own
/// opening line, if the app doesn't have one yet.
pub fn add_nav_item(source: &str, new_item_text: &str) -> Result<String> {
    let lines: Vec<&str> = source.lines().collect();
    match nav_bounds(source)? {
        Some(nav) => {
            let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
            new_lines.extend_from_slice(&lines[..nav.close_idx]);
            new_lines.extend(new_item_text.lines());
            new_lines.extend_from_slice(&lines[nav.close_idx..]);
            Ok(join_lines(&new_lines, source.ends_with('\n')))
        }
        None => {
            let settings = markup::app_settings_lines(source).context("failed to parse app")?;
            let insert_at = settings.app_open_line as usize; // 0-based index right after the opening line
            let block = format!("\n  nav {{\n{new_item_text}\n  }}");
            let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
            new_lines.extend_from_slice(&lines[..insert_at]);
            new_lines.extend(block.lines());
            new_lines.extend_from_slice(&lines[insert_at..]);
            Ok(join_lines(&new_lines, source.ends_with('\n')))
        }
    }
}

/// Removes the top-level nav item at `idx`.
pub fn delete_nav_item(source: &str, idx: usize) -> Result<String> {
    let nav = nav_bounds(source)?.ok_or_else(|| anyhow::anyhow!("this app has no nav block"))?;
    let n = nav.items.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for nav ({n} top-level items)");
    }
    let (start, end) = nav.items[idx];
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Top-level nav item `idx`'s exact current source text — a submenu
/// item's nested children come along as part of this same chunk, since
/// they're not individually addressable here.
pub fn nav_item_source(source: &str, idx: usize) -> Result<String> {
    let nav = nav_bounds(source)?.ok_or_else(|| anyhow::anyhow!("this app has no nav block"))?;
    let n = nav.items.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for nav ({n} top-level items)");
    }
    let (start, end) = nav.items[idx];
    let lines: Vec<&str> = source.lines().collect();
    Ok(lines[start..end].join("\n"))
}

/// Replaces top-level nav item `idx` outright with `new_item_text`.
pub fn replace_nav_item(source: &str, idx: usize, new_item_text: &str) -> Result<String> {
    let nav = nav_bounds(source)?.ok_or_else(|| anyhow::anyhow!("this app has no nav block"))?;
    let n = nav.items.len();
    if idx >= n {
        anyhow::bail!("index {idx} out of range for nav ({n} top-level items)");
    }
    let (start, end) = nav.items[idx];
    let lines: Vec<&str> = source.lines().collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 4);
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend(new_item_text.lines());
    new_lines.extend_from_slice(&lines[end..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

/// Reorders the nav's top-level items so the one currently at index
/// `new_order[i]` becomes the `i`th one — same permutation semantics as
/// `page_reorder::reorder_page`.
pub fn reorder_nav_items(source: &str, new_order: &[usize]) -> Result<String> {
    let nav = nav_bounds(source)?.ok_or_else(|| anyhow::anyhow!("this app has no nav block"))?;
    let n = nav.items.len();
    if new_order.len() != n {
        anyhow::bail!("new order has {} entries but nav has {n} top-level items", new_order.len());
    }
    let mut seen = vec![false; n];
    for &i in new_order {
        if i >= n {
            anyhow::bail!("index {i} out of range for nav ({n} top-level items)");
        }
        if std::mem::replace(&mut seen[i], true) {
            anyhow::bail!("index {i} repeated in new order — must be a permutation of 0..{n}");
        }
    }

    let lines: Vec<&str> = source.lines().collect();
    let chunks: Vec<&[&str]> = nav.items.iter().map(|&(start, end)| &lines[start..end]).collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..nav.items[0].0]);
    for &i in new_order {
        new_lines.extend_from_slice(chunks[i]);
    }
    new_lines.extend_from_slice(&lines[nav.items[n - 1].1..]);
    Ok(join_lines(&new_lines, source.ends_with('\n')))
}

// ---- app-level settings (theme/icons/chart_lib/auth) ----

/// Sets the app's theme/icons/chart_lib to exactly the given values
/// (replacing an existing declaration line in place, or inserting a
/// fresh one right after the app's own opening line if it wasn't
/// declared before) and turns the bare `auth { }` toggle block on or
/// off. A plain settings form always submits all three properties, so
/// unlike an entity/query/nav-item edit there's no "remove this
/// property" case to handle — only replace-or-insert.
pub fn set_app_settings(source: &str, theme: &str, icons: &str, chart_lib: &str, auth_enabled: bool) -> Result<String> {
    let s = markup::app_settings_lines(source).context("failed to parse app")?;
    let lines: Vec<&str> = source.lines().collect();

    // Indices to drop outright: the auth block's own lines, if turning
    // it off; nothing else is ever removed.
    let mut drop_from = None;
    let mut drop_to = None;
    if !auth_enabled {
        if let Some((start, end)) = s.auth_lines {
            drop_from = Some((start - 1) as usize);
            drop_to = Some(end as usize); // end is 1-based inclusive -> exclusive 0-based
        }
    }

    let mut replacements: Vec<(usize, String)> = Vec::new();
    if let Some(line) = s.theme_line {
        replacements.push(((line - 1) as usize, format!("  theme: {theme}")));
    }
    if let Some(line) = s.icons_line {
        replacements.push(((line - 1) as usize, format!("  icons: {icons}")));
    }
    if let Some(line) = s.chart_lib_line {
        replacements.push(((line - 1) as usize, format!("  chart_lib: {chart_lib}")));
    }

    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len() + 4);
    for (i, line) in lines.iter().enumerate() {
        if let (Some(from), Some(to)) = (drop_from, drop_to) {
            if i >= from && i < to {
                continue;
            }
        }
        if let Some((_, new_line)) = replacements.iter().find(|(idx, _)| *idx == i) {
            out_lines.push(new_line.clone());
        } else {
            out_lines.push(line.to_string());
        }
    }

    // Insert whichever of theme/icons/chart_lib weren't already
    // declared, plus a fresh `auth { }` if turning it on and it wasn't
    // there before — all right after the app's opening line, in that
    // fixed order, last-inserted-ends-up-first so the final order reads
    // theme/icons/chart_lib/auth top to bottom.
    let insert_at = s.app_open_line as usize; // 0-based index right after the opening line, unaffected by earlier removals since auth's own lines are always after it
    let mut inserts: Vec<String> = Vec::new();
    if s.chart_lib_line.is_none() {
        inserts.push(format!("  chart_lib: {chart_lib}"));
    }
    if s.icons_line.is_none() {
        inserts.push(format!("  icons: {icons}"));
    }
    if s.theme_line.is_none() {
        inserts.push(format!("  theme: {theme}"));
    }
    if auth_enabled && s.auth_lines.is_none() {
        inserts.push("  auth {\n  }".to_string());
    }
    for line in inserts {
        out_lines.insert(insert_at, line);
    }

    let mut out = out_lines.join("\n");
    if source.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"app "Demo" {
  query recent {
    sql: "select 1"
  }

  entity "t" {
    field id: id
    field name: text
  }

  page "P" {
    text "hi"
  }
}
"#;

    #[test]
    fn adds_and_deletes_an_entity() {
        let added = add_entity(SRC, "  entity \"u\" {\n    field id: id\n  }").unwrap();
        assert!(added.contains("entity \"u\""));
        assert!(markup::parse_app(&added).is_ok());

        let deleted = delete_entity(&added, "u").unwrap();
        assert_eq!(deleted, SRC);
    }

    #[test]
    fn rejects_deleting_an_unknown_entity() {
        assert!(delete_entity(SRC, "nope").is_err());
    }

    #[test]
    fn replaces_and_reads_back_an_entitys_source() {
        let src = entity_source(SRC, "t").unwrap();
        assert_eq!(src, "  entity \"t\" {\n    field id: id\n    field name: text\n  }");

        let replaced = replace_entity(SRC, "t", "  entity \"t\" {\n    field id: id\n    field label: text\n  }").unwrap();
        assert!(replaced.contains("field label: text"));
        assert!(!replaced.contains("field name: text"));
        assert!(markup::parse_app(&replaced).is_ok());
    }

    #[test]
    fn adds_deletes_and_replaces_a_query() {
        let added = add_query(SRC, "  query other {\n    sql: \"select 2\"\n  }").unwrap();
        assert!(added.contains("query other"));
        assert!(markup::parse_app(&added).is_ok());

        let src = query_source(&added, "other").unwrap();
        assert_eq!(src, "  query other {\n    sql: \"select 2\"\n  }");

        let replaced = replace_query(&added, "other", "  query other {\n    sql: \"select 3\"\n  }").unwrap();
        assert!(replaced.contains("select 3"));
        assert!(markup::parse_app(&replaced).is_ok());

        let deleted = delete_query(&replaced, "other").unwrap();
        assert!(!deleted.contains("query other"));
        assert!(markup::parse_app(&deleted).is_ok());
    }

    #[test]
    fn rejects_deleting_an_unknown_query() {
        assert!(delete_query(SRC, "nope").is_err());
    }

    #[test]
    fn creates_a_fresh_nav_block_when_none_exists() {
        let out = add_nav_item(SRC, "    item \"Go\" -> page P").unwrap();
        assert!(out.contains("nav {"));
        assert!(out.contains("item \"Go\" -> page P"));
        assert!(markup::parse_app(&out).is_ok());
    }

    #[test]
    fn adds_deletes_reorders_and_replaces_nav_items() {
        let src = r#"app "Demo" {
  nav {
    item "A" -> page P
    item "B" -> page P
  }

  page "P" { text "hi" }
}
"#;
        let added = add_nav_item(src, "    item \"C\" -> page P").unwrap();
        assert!(markup::parse_app(&added).is_ok());
        assert_eq!(nav_item_source(&added, 2).unwrap(), "    item \"C\" -> page P");

        let reordered = reorder_nav_items(&added, &[2, 0, 1]).unwrap();
        assert!(markup::parse_app(&reordered).is_ok());
        assert_eq!(nav_item_source(&reordered, 0).unwrap(), "    item \"C\" -> page P");

        let replaced = replace_nav_item(&reordered, 0, "    item \"C2\" -> page P").unwrap();
        assert!(replaced.contains("\"C2\""));
        assert!(markup::parse_app(&replaced).is_ok());

        let deleted = delete_nav_item(&replaced, 0).unwrap();
        assert!(!deleted.contains("\"C2\""));
        assert!(markup::parse_app(&deleted).is_ok());
    }

    #[test]
    fn rejects_an_out_of_range_nav_item_index() {
        let src = r#"app "Demo" {
  nav {
    item "A" -> page P
  }

  page "P" { text "hi" }
}
"#;
        assert!(delete_nav_item(src, 5).is_err());
    }

    #[test]
    fn rejects_nav_edits_when_there_is_no_nav_block() {
        assert!(delete_nav_item(SRC, 0).is_err());
        assert!(reorder_nav_items(SRC, &[]).is_err());
    }

    #[test]
    fn inserts_settings_that_were_never_declared() {
        let out = set_app_settings(SRC, "vivid", "builtin", "inline", true).unwrap();
        assert!(out.contains("theme: vivid"));
        assert!(out.contains("icons: builtin"));
        assert!(out.contains("chart_lib: inline"));
        assert!(out.contains("auth {"));
        assert!(markup::parse_app(&out).is_ok());
        let app = markup::parse_app(&out).unwrap();
        assert_eq!(app.theme.as_deref(), Some("vivid"));
        assert!(app.auth);
    }

    #[test]
    fn replaces_an_already_declared_setting_in_place() {
        let src = r#"app "Demo" {
  theme: plain
  auth {
  }

  page "P" { text "hi" }
}
"#;
        let out = set_app_settings(src, "shadcn", "builtin", "inline", false).unwrap();
        assert!(out.contains("theme: shadcn"));
        assert!(!out.contains("theme: plain"));
        assert!(!out.contains("auth {"));
        assert!(markup::parse_app(&out).is_ok());
        let app = markup::parse_app(&out).unwrap();
        assert!(!app.auth);
    }
}
