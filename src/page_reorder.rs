//! Reorders a page's top-level components within a `.pgapp` file's raw
//! text, given a new order (as a permutation of the page's current
//! component indices) — the file-editing half of the App Builder's
//! drag-and-drop reordering (see the `/:workspace/:app/admin/pages/:page/reorder`
//! route in `server.rs`, which also updates `pgapp_meta.components.ordinal`
//! so the database and the file agree).
//!
//! Deliberately a line-based text splice, not a parse-and-regenerate:
//! `markup::page_component_start_lines` gives real start lines (reusing
//! the actual grammar, so it's never out of sync with what the parser
//! considers a component boundary), and this module cuts along those
//! lines and reassembles them in the new order — every component's own
//! text, including its formatting and any inline comments, survives
//! completely unchanged; only its position moves. Single-file apps
//! only (same restriction as `page_component_start_lines`).

use anyhow::{Context, Result};

use crate::markup;

/// Reorders `page_name`'s components in `source` so that the component
/// currently at index `new_order[i]` becomes the `i`th one — e.g. an
/// original order of `[A, B, C]` reordered by `new_order = [2, 0, 1]`
/// becomes `[C, A, B]`. `new_order` must be a permutation of `0..n`
/// where `n` is the page's current component count.
pub fn reorder_page(source: &str, page_name: &str, new_order: &[usize]) -> Result<String> {
    let (start_lines, closing_line) = markup::page_component_start_lines(source, page_name)
        .with_context(|| format!("failed to locate page '{page_name}'"))?;
    let n = start_lines.len();
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
    // A component's chunk starts at its own token line, walked backward
    // over any immediately-preceding comment lines (no blank line in
    // between) so a comment describing it travels along when it moves —
    // otherwise it would silently stay behind, now describing whatever
    // component ended up in its old spot.
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

    let boundaries: Vec<usize> = start_lines.iter().map(|&l| adjusted_start(l)).collect();
    let end_of_page_body = (closing_line - 1) as usize; // 0-based index of the line the closing '}' is on

    let chunks: Vec<&[&str]> = (0..n)
        .map(|i| {
            let start = boundaries[i];
            let end = if i + 1 < n { boundaries[i + 1] } else { end_of_page_body };
            &lines[start..end]
        })
        .collect();

    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..boundaries[0]]);
    for &i in new_order {
        new_lines.extend_from_slice(chunks[i]);
    }
    new_lines.extend_from_slice(&lines[end_of_page_body..]);

    let mut out = new_lines.join("\n");
    if source.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
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
}
