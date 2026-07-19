//! `pgapp new` / `pgapp create` (also reachable as `cargo pgapp` via
//! `src/bin/cargo-pgapp.rs`) — generates a minimal, runnable starter
//! app so a new project begins with real (if generic) markup to edit,
//! instead of a blank file and the README open in another tab.
//! Hand-rolled arg parsing and prompts, no CLI/prompt crate.
//!
//! Purely a file scaffolder — it never touches a database. Every app
//! is registered into a workspace's schema (`pgapp workspace create`,
//! `pgapp app create`; see `main.rs` and README's "Instance mode"
//! section), so there's no "sync this to a database" step that
//! belongs here.
//!
//! Two modes, chosen by whether an app name was already given:
//! - **Flag-driven** (`pgapp new <AppName> [path] [--dir] [--theme
//!   <name>]`): every value already on the command line, no prompts —
//!   for scripts/CI.
//! - **Interactive** (`pgapp new`/`pgapp create` bare, or `--create`):
//!   prompts for whatever wasn't already given — app name, theme,
//!   single-file vs. directory — like `create-react-app`'s
//!   questionnaire.

use std::io::Write;

use anyhow::{bail, Context, Result};

struct ParsedArgs {
    name: Option<String>,
    path: Option<String>,
    as_dir: bool,
    theme: Option<String>,
    force_interactive: bool,
}

fn parse_args(args: &[String]) -> Result<ParsedArgs> {
    let mut parsed = ParsedArgs { name: None, path: None, as_dir: false, theme: None, force_interactive: false };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                parsed.as_dir = true;
                i += 1;
            }
            "--create" | "-i" | "--interactive" => {
                parsed.force_interactive = true;
                i += 1;
            }
            "--theme" => {
                parsed.theme = Some(
                    args.get(i + 1)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("--theme needs a value, e.g. --theme vivid"))?,
                );
                i += 2;
            }
            other if parsed.name.is_none() => {
                parsed.name = Some(other.to_string());
                i += 1;
            }
            other if parsed.path.is_none() => {
                parsed.path = Some(other.to_string());
                i += 1;
            }
            other => bail!("unexpected argument '{other}' (see `pgapp new --help`)"),
        }
    }
    Ok(parsed)
}

/// `pgapp new`/`pgapp create` [`<AppName>` [path] [--dir] [--theme
/// <name>] [--create]]
pub async fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }

    let parsed = parse_args(args)?;
    if parsed.force_interactive || parsed.name.is_none() {
        run_interactive(parsed).await
    } else {
        run_noninteractive(parsed)
    }
}

/// The scriptable path: every value must already be on the command
/// line (an absent one falls back to a fixed default, never a
/// prompt) — only ever writes files.
fn run_noninteractive(parsed: ParsedArgs) -> Result<()> {
    let name = parsed.name.expect("run() only takes this path once a name is present");
    let theme = parsed.theme.unwrap_or_else(|| "shadcn".to_string());
    let slug = slugify(&name);
    let target = parsed.path.unwrap_or_else(|| if parsed.as_dir { slug.clone() } else { format!("{slug}.pgapp") });

    if parsed.as_dir {
        scaffold_dir(&target, &name, &theme)?;
    } else {
        scaffold_file(&target, &name, &theme)?;
    }

    println!("Created {target}");
    println!();
    print_next_steps(&target);
    Ok(())
}

/// The `create-react-app`-style path: prompts for anything not already
/// given (app name, theme, single-file vs. directory), then writes the
/// scaffold — registering it into an instance/workspace is a separate
/// step (`pgapp app create`), not this command's job.
async fn run_interactive(parsed: ParsedArgs) -> Result<()> {
    println!("Let's scaffold a new pgapp app.");
    println!();

    let name = match parsed.name {
        Some(n) => n,
        None => prompt_required("App name")?,
    };
    let theme = match parsed.theme {
        Some(t) => t,
        None => prompt("Theme (plain/shadcn/vivid/google_m3)", "shadcn")?,
    };
    let as_dir = if parsed.as_dir {
        true
    } else {
        prompt_yes_no("Scaffold as a directory of files instead of one?", false)?
    };

    let slug = slugify(&name);
    let target = parsed.path.unwrap_or_else(|| if as_dir { slug.clone() } else { format!("{slug}.pgapp") });

    if as_dir {
        scaffold_dir(&target, &name, &theme)?;
    } else {
        scaffold_file(&target, &name, &theme)?;
    }
    println!();
    println!("Created {target}");
    println!();
    print_next_steps(&target);
    Ok(())
}

fn print_next_steps(target: &str) {
    println!("Next steps — every app is registered into a workspace's schema (see README's \"Instance mode\" section):");
    println!("  pgapp instance init                 (once per Postgres database)");
    println!("  pgapp workspace create <dbname>      (once per schema)");
    println!("  pgapp run {target} --instance <dbname> --workspace <slug>");
}

/// Postgres error code 3D000 = `invalid_catalog_name`, raised when the
/// named database doesn't exist — the one connection failure worth
/// auto-recovering from; anything else (bad host, bad credentials) is
/// a real problem the user needs to see, not paper over.
pub fn is_missing_database_error(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|db_err| db_err.code()).as_deref() == Some("3D000")
}

pub fn prompt(label: &str, default: &str) -> Result<String> {
    print!("{label} [{default}]: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).context("failed to read from stdin")?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() { default.to_string() } else { trimmed.to_string() })
}

/// Like `prompt`, but with no default — re-asks until something
/// non-empty is given (an app name is the one value this CLI can't
/// sensibly make up on its own).
pub fn prompt_required(label: &str) -> Result<String> {
    loop {
        print!("{label}: ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).context("failed to read from stdin")?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        println!("(required)");
    }
}

pub fn prompt_yes_no(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    print!("{label} [{hint}]: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).context("failed to read from stdin")?;
    let trimmed = line.trim().to_ascii_lowercase();
    Ok(match trimmed.as_str() {
        "" => default,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

fn print_help() {
    println!(
        r#"pgapp new <AppName> [path] [--dir] [--theme <name>]
pgapp new [--create]                (or `pgapp create`, or `cargo pgapp create`)

Generates a minimal, runnable starter app: one entity, one page with
the classic Report+Form CRUD pattern, and a nav bar link to it.

Neither mode ever touches a database — both only ever write files;
register the result into a workspace afterward with `pgapp app
create`/`pgapp run --instance --workspace` (see README's "Instance
mode" section):
  - Flags only, with an <AppName>: non-interactive. Good for scripts/CI.
  - No <AppName> (or --create): interactive, create-react-app style —
    prompts for whatever's missing (app name, theme, single file vs.
    directory).

  <AppName>       the app's display name (quoted in the generated
                  markup, so spaces are fine: "My Project")
  [path]          where to write it (default: a slugified <AppName>,
                  plus ".pgapp" unless --dir is given)
  --dir           scaffold a directory of files instead of one file
                  (app.pgapp + items.pgapp + pages/items.pgapp) — the
                  layout examples/helpdesk-modular/ demonstrates
  --theme <name>  starting theme (default: shadcn) — see themes/ for
                  what's shipped (plain, shadcn, vivid, google_m3)
  --create, -i    force the interactive prompts even when an <AppName>
                  is also given on the command line

Examples:
  pgapp new "My Project"
  pgapp new Inventory inventory.pgapp
  pgapp new Inventory --dir --theme vivid
  pgapp create
  cargo pgapp create
"#
    );
}

/// Lowercase, non-alphanumerics collapsed to single underscores, no
/// leading/trailing underscore — used only for the generated
/// file/directory name. The app's *display* name (quoted in the
/// markup itself) is written exactly as given; this never touches it.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = true; // avoid a leading underscore
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("app");
    }
    slug
}

fn entity_block() -> String {
    r#"entity "items" {
  field id: id
  field name: text required
  field notes: text
  field done: boolean default false
}
"#
    .to_string()
}

fn page_block() -> String {
    r#"page "Items" {
  report "All items" of items {
    columns: name, notes, done
    page_size: 20
  }
  form "Create / edit" of items {
    fields: name, notes, done
  }
}
"#
    .to_string()
}

pub fn scaffold_file(target: &str, name: &str, theme: &str) -> Result<()> {
    refuse_to_overwrite(target)?;
    let markup = format!(
        r#"# {name} — generated by `pgapp new`. This file *is* the app:
# entities, pages, and navigation all live in one declarative markup
# file, synced straight into Postgres. Register and run it with:
#
#   pgapp instance init                          (once per Postgres database)
#   pgapp workspace create <dbname>               (once per schema)
#   pgapp run {target} --instance <dbname> --workspace <slug>
#
# See README.md for the full markup reference — every property below
# has more to it than this minimal starter shows (search, saved views,
# charts, auth, per-component theming, dynamic actions, ...).

app "{name}" {{
  theme: {theme}

  nav {{
    item "Items" -> page Items
  }}

  {entity}
  # The classic CRUD pattern: a Report and a Form for the same entity
  # on one page. Edit/Delete actions on each row appear automatically
  # because the Form is here too — no extra config needed.
  {page}
}}
"#,
        entity = indent(&entity_block(), 2),
        page = indent(&page_block(), 2),
    );
    std::fs::write(target, markup).with_context(|| format!("failed to write '{target}'"))?;
    Ok(())
}

pub fn scaffold_dir(target: &str, name: &str, theme: &str) -> Result<()> {
    refuse_to_overwrite(target)?;
    let base = std::path::Path::new(target);
    let pages_dir = base.join("pages");
    std::fs::create_dir_all(&pages_dir).with_context(|| format!("failed to create directory '{}'", pages_dir.display()))?;

    let app_path = base.join("app.pgapp");
    let entity_path = base.join("items.pgapp");
    let page_path = pages_dir.join("items.pgapp");

    let app_markup = format!(
        r#"# {name} — generated by `pgapp new --dir`. Directory rules (see
# src/source.rs): every .pgapp file under this directory merges into
# one app. Exactly one file — this one — declares the `app` block
# (settings, nav, header/footer); every other file holds top-level
# entity/page/query blocks, referencing each other by name exactly as
# they would inside a single file. Register and run it with:
#
#   pgapp instance init                          (once per Postgres database)
#   pgapp workspace create <dbname>               (once per schema)
#   pgapp run {target} --instance <dbname> --workspace <slug>

app "{name}" {{
  theme: {theme}

  nav {{
    item "Items" -> page Items
  }}
}}
"#
    );
    let page_markup = format!(
        r#"# The classic CRUD pattern: a Report and a Form for the same entity
# on one page. Edit/Delete actions on each row appear automatically
# because the Form is here too — no extra config needed.
{}"#,
        page_block()
    );

    std::fs::write(&app_path, app_markup).with_context(|| format!("failed to write '{}'", app_path.display()))?;
    std::fs::write(&entity_path, entity_block()).with_context(|| format!("failed to write '{}'", entity_path.display()))?;
    std::fs::write(&page_path, page_markup).with_context(|| format!("failed to write '{}'", page_path.display()))?;
    Ok(())
}

fn refuse_to_overwrite(target: &str) -> Result<()> {
    if std::path::Path::new(target).exists() {
        bail!("'{target}' already exists — pick a different name/path, or remove it first");
    }
    Ok(())
}

/// Indents every line of `block` by `spaces`, except the first (the
/// caller's own `format!` template already positions that one) — used
/// to splice a multi-line sub-block into the single-file template at
/// the right nesting level.
fn indent(block: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    let mut out = String::new();
    for (i, line) in block.lines().enumerate() {
        if i > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&pad);
            }
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugifies_names_for_the_filesystem() {
        assert_eq!(slugify("My Project"), "my_project");
        assert_eq!(slugify("Inventory"), "inventory");
        assert_eq!(slugify("  weird--name!! "), "weird_name");
        assert_eq!(slugify("已"), "app");
    }

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flag_driven_args_without_forcing_interactive_mode() {
        let parsed = parse_args(&args(&["Inventory", "inv.pgapp", "--dir", "--theme", "vivid"])).unwrap();
        assert_eq!(parsed.name.as_deref(), Some("Inventory"));
        assert_eq!(parsed.path.as_deref(), Some("inv.pgapp"));
        assert!(parsed.as_dir);
        assert_eq!(parsed.theme.as_deref(), Some("vivid"));
        assert!(!parsed.force_interactive);
    }

    #[test]
    fn a_bare_name_or_no_args_leave_the_rest_unset() {
        let parsed = parse_args(&args(&[])).unwrap();
        assert!(parsed.name.is_none());
        assert!(!parsed.force_interactive);

        let parsed = parse_args(&args(&["Inventory"])).unwrap();
        assert_eq!(parsed.name.as_deref(), Some("Inventory"));
        assert!(parsed.path.is_none());
    }

    #[test]
    fn create_and_interactive_flags_force_interactive_mode_even_with_a_name() {
        for flag in ["--create", "-i", "--interactive"] {
            let parsed = parse_args(&args(&["Inventory", flag])).unwrap();
            assert!(parsed.force_interactive, "flag {flag} should force interactive mode");
            assert_eq!(parsed.name.as_deref(), Some("Inventory"));
        }
    }

    #[test]
    fn generated_single_file_app_parses() {
        let dir = std::env::temp_dir().join(format!("pgapp_scaffold_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.pgapp");
        scaffold_file(target.to_str().unwrap(), "Test App", "shadcn").unwrap();
        let src = std::fs::read_to_string(&target).unwrap();
        let app = crate::markup::parse_app(&src).unwrap_or_else(|e| panic!("generated scaffold failed to parse: {e}"));
        assert_eq!(app.name, "Test App");
        assert_eq!(app.entities.len(), 1);
        assert_eq!(app.pages.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn generated_dir_app_parses() {
        let dir = std::env::temp_dir().join(format!("pgapp_scaffold_dirtest_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        scaffold_dir(dir.to_str().unwrap(), "Test Dir App", "shadcn").unwrap();
        let app = crate::source::load(dir.to_str().unwrap()).unwrap_or_else(|e| panic!("generated dir scaffold failed to load: {e}"));
        assert_eq!(app.name, "Test Dir App");
        assert_eq!(app.entities.len(), 1);
        assert_eq!(app.pages.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refuses_to_clobber_an_existing_path() {
        let dir = std::env::temp_dir().join(format!("pgapp_scaffold_clobber_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("exists.pgapp");
        std::fs::write(&target, "not touching this").unwrap();
        let err = scaffold_file(target.to_str().unwrap(), "X", "shadcn").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "not touching this");
        std::fs::remove_dir_all(&dir).ok();
    }
}
