//! Loads an [`AppDef`] from a `.pgapp` source path — either a single
//! file, or a directory of them.
//!
//! Directory semantics are deliberately Terraform-shaped: every
//! `.pgapp` file under the directory (recursively) merges into one app.
//! Exactly one file declares the `app "..." { }` block — settings,
//! `auth`, `nav`, and `header`/`footer` chrome live there; every other
//! file is a *fragment* holding top-level `entity`/`page`/`query`
//! blocks (see [`markup::parse_fragment`]). There is no `include`
//! statement, no import graph, and no ordering: files are read in
//! sorted path order purely so error output is deterministic, and all
//! cross-references are by name exactly as within a single file (the
//! metadata sync already resolves forward references).
//!
//! The same name declared in two files is a hard error naming both
//! files — without this, the metadata upsert would silently collapse
//! the duplicates into one row.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::markup;
use crate::model::AppDef;

/// Loads a single `.pgapp` file (which must be a full `app` file) or a
/// directory of them.
pub fn load(path: &str) -> Result<AppDef> {
    let meta = std::fs::metadata(path).with_context(|| format!("cannot read markup path '{path}'"))?;
    if meta.is_dir() {
        load_dir(Path::new(path))
    } else {
        let src = std::fs::read_to_string(path).with_context(|| format!("failed to read markup file '{path}'"))?;
        markup::parse_app(&src).with_context(|| format!("failed to parse markup file '{path}'"))
    }
}

fn collect_pgapp_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("failed to read directory '{}'", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_pgapp_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "pgapp") {
            out.push(path);
        }
    }
    Ok(())
}

fn load_dir(dir: &Path) -> Result<AppDef> {
    let mut files = Vec::new();
    collect_pgapp_files(dir, &mut files)?;
    files.sort();
    if files.is_empty() {
        bail!("no .pgapp files found under '{}'", dir.display());
    }

    let mut app: Option<(PathBuf, AppDef)> = None;
    let mut fragments: Vec<(PathBuf, markup::Fragment)> = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(&file)
            .with_context(|| format!("failed to read markup file '{}'", file.display()))?;
        let is_app = markup::starts_app_block(&src)
            .with_context(|| format!("failed to parse markup file '{}'", file.display()))?;
        if is_app {
            let parsed = markup::parse_app(&src)
                .with_context(|| format!("failed to parse markup file '{}'", file.display()))?;
            if let Some((first, _)) = &app {
                bail!(
                    "both '{}' and '{}' declare an `app` block — exactly one file in the \
                     directory may (settings/auth/nav/header/footer live there); the rest \
                     hold entity/page/query blocks",
                    first.display(),
                    file.display()
                );
            }
            app = Some((file, parsed));
        } else {
            let fragment = markup::parse_fragment(&src)
                .with_context(|| format!("failed to parse markup file '{}'", file.display()))?;
            fragments.push((file, fragment));
        }
    }

    let (app_file, mut app) = app.ok_or_else(|| {
        anyhow::anyhow!(
            "no file under '{}' declares an `app \"...\" {{ }}` block — one (and only one) must",
            dir.display()
        )
    })?;

    // Merge, tracking which file first declared each name so a
    // collision error can point at both.
    let mut entity_files: HashMap<String, PathBuf> =
        app.entities.iter().map(|e| (e.name.clone(), app_file.clone())).collect();
    let mut page_files: HashMap<String, PathBuf> =
        app.pages.iter().map(|p| (p.name.clone(), app_file.clone())).collect();
    let mut query_files: HashMap<String, PathBuf> =
        app.queries.iter().map(|q| (q.name.clone(), app_file.clone())).collect();

    for (file, fragment) in fragments {
        for entity in fragment.entities {
            if let Some(first) = entity_files.insert(entity.name.clone(), file.clone()) {
                bail!(
                    "entity '{}' is defined in both '{}' and '{}'",
                    entity.name,
                    first.display(),
                    file.display()
                );
            }
            app.entities.push(entity);
        }
        for page in fragment.pages {
            if let Some(first) = page_files.insert(page.name.clone(), file.clone()) {
                bail!(
                    "page '{}' is defined in both '{}' and '{}'",
                    page.name,
                    first.display(),
                    file.display()
                );
            }
            app.pages.push(page);
        }
        for query in fragment.queries {
            if let Some(first) = query_files.insert(query.name.clone(), file.clone()) {
                bail!(
                    "app-scoped query '{}' is defined in both '{}' and '{}'",
                    query.name,
                    first.display(),
                    file.display()
                );
            }
            app.queries.push(query);
        }
    }

    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a throwaway app directory under the target tmp dir.
    fn write_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pgapp-source-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, content) in files {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    const APP: &str = r#"
        app "Demo" {
            theme: plain
            nav { item "Home" -> page Home }
        }
    "#;

    #[test]
    fn merges_a_directory_into_one_app() {
        let dir = write_dir(
            "merge",
            &[
                ("app.pgapp", APP),
                ("things.pgapp", r#"
                    entity "things" { field id: id field name: text required }
                    query recent { sql: "select 1 as value" }
                "#),
                ("pages/home.pgapp", r#"
                    page "Home" { report "Things" of things { columns: name } }
                "#),
            ],
        );
        let app = load(dir.to_str().unwrap()).unwrap();
        assert_eq!(app.name, "Demo");
        assert_eq!(app.theme.as_deref(), Some("plain"));
        assert_eq!(app.entities.len(), 1);
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.queries.len(), 1);
        assert_eq!(app.nav.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_duplicate_page_names_across_files() {
        let dir = write_dir(
            "dup",
            &[
                ("app.pgapp", APP),
                ("a.pgapp", r#"page "Home" { text "one" }"#),
                ("b.pgapp", r#"page "Home" { text "two" }"#),
            ],
        );
        let err = load(dir.to_str().unwrap()).unwrap_err().to_string();
        assert!(err.contains("page 'Home' is defined in both"), "got: {err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_two_app_blocks_and_zero_app_blocks() {
        let dir = write_dir("twoapps", &[("a.pgapp", APP), ("b.pgapp", APP)]);
        let err = load(dir.to_str().unwrap()).unwrap_err().to_string();
        assert!(err.contains("declare an `app` block"), "got: {err}");
        std::fs::remove_dir_all(&dir).unwrap();

        let dir = write_dir("noapp", &[("a.pgapp", r#"page "P" { text "hi" }"#)]);
        let err = load(dir.to_str().unwrap()).unwrap_err().to_string();
        assert!(err.contains("no file under"), "got: {err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fragment_rejects_app_level_blocks_with_file_and_line() {
        let dir = write_dir(
            "navfrag",
            &[
                ("app.pgapp", APP),
                ("bad.pgapp", "page \"P\" { text \"hi\" }\nnav { item \"X\" -> page P }"),
            ],
        );
        let err = format!("{:#}", load(dir.to_str().unwrap()).unwrap_err());
        assert!(err.contains("bad.pgapp"), "got: {err}");
        assert!(err.contains("line 2"), "got: {err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
