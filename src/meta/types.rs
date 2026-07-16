//! The runtime model: what `load_app` reloads from `pgapp_meta` and
//! `server.rs`/`render.rs` actually work with. Nothing here is built
//! directly from the parsed markup (`model::AppDef`) — it's rebuilt
//! from the database every time the process starts, which is the whole
//! point of "the database is the source of truth."

use std::collections::{BTreeMap, HashMap};

use crate::model::{FieldItem, FieldType, PageKind};

/// A named query, compiled once at load time: `sql` already uses
/// positional `$N::text` parameters, and `bind_names[i]` is the bind
/// context key that fills `$(i + 1)`.
#[derive(Debug, Clone)]
pub struct RuntimeQuery {
    pub sql: String,
    pub bind_names: Vec<String>,
}

/// Runtime view of a field, as reloaded from `pgapp_meta` (not from the
/// markup file) — this is what the server uses to build SQL.
#[derive(Debug, Clone)]
pub struct RuntimeField {
    pub name: String,
    pub data_type: FieldType,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeEntity {
    pub name: String,
    pub table_name: String,
    pub fields: Vec<RuntimeField>,
}

impl RuntimeEntity {
    pub fn field(&self, name: &str) -> Option<&RuntimeField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A page item, reloaded from `pgapp_meta.page_items` (or the
/// header/footer equivalents). `Link` targets are resolved back to the
/// target page's *name* (not its id) so rendering never needs another
/// database round trip; `Region` carries the query's name, resolved
/// against the page/app query registry at render time.
#[derive(Debug, Clone)]
pub enum RuntimePageItem {
    Text(String),
    Link { label: String, target_page: String },
    Region { label: String, query: String },
}

#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
    pub extra_params: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct RuntimePage {
    pub name: String,
    pub kind: PageKind,
    pub entity: Option<RuntimeEntity>,
    pub columns: Vec<String>,
    pub form: Vec<String>,
    pub link_column: Option<LinkColumn>,
    pub items: Vec<RuntimePageItem>,
    /// Resolved item (kind + config) for every field in `form` (never
    /// missing — `sync::sync_app` always writes one, defaulting via
    /// `item_types::default_kind_for`).
    pub item_types: HashMap<String, FieldItem>,
    /// Overrides the page's row source with a named query; see
    /// `model::PageDef::source_query`.
    pub source_query: Option<String>,
    /// Queries visible only on this page.
    pub queries: HashMap<String, RuntimeQuery>,
}

impl RuntimePage {
    /// Looks up a query by name, preferring one scoped to this page over
    /// an app-scoped query of the same name.
    pub fn resolve_query<'a>(&'a self, app: &'a RuntimeApp, name: &str) -> Option<&'a RuntimeQuery> {
        self.queries.get(name).or_else(|| app.queries.get(name))
    }
}

/// One node in the reloaded nav tree; see [`crate::model::NavItem`] for
/// the markup-level equivalent.
#[derive(Debug, Clone)]
pub struct NavNode {
    pub label: String,
    pub target_page: Option<String>,
    pub children: Vec<NavNode>,
}

#[derive(Debug, Clone)]
pub struct RuntimeApp {
    pub name: String,
    pub pages: Vec<RuntimePage>,
    pub nav: Vec<NavNode>,
    pub header: Vec<RuntimePageItem>,
    pub footer: Vec<RuntimePageItem>,
    /// Queries visible from every page.
    pub queries: HashMap<String, RuntimeQuery>,
}

impl RuntimeApp {
    pub fn page(&self, name: &str) -> Option<&RuntimePage> {
        self.pages.iter().find(|p| p.name == name)
    }

    /// Everything site-wide that renderers need alongside a single
    /// page: the nav tree, header/footer chrome, and the resolved rows
    /// for every `Region` item anywhere on the current request (the
    /// page's own items plus the header/footer), keyed by query name.
    pub fn chrome<'a>(&'a self, regions: &'a RegionRows) -> Chrome<'a> {
        Chrome {
            nav: &self.nav,
            header: &self.header,
            footer: &self.footer,
            regions,
        }
    }
}

/// Rows already fetched for each `Region` page item that appears on the
/// current request, keyed by query name. Resolving these is an async DB
/// round trip, so `server.rs` does it up front and hands the result to
/// the (synchronous) render functions.
pub type RegionRows = HashMap<String, Vec<BTreeMap<String, Option<String>>>>;

/// Borrowed bundle of the app-wide chrome (nav/header/footer/regions),
/// passed into every page render function instead of four separate
/// parameters.
#[derive(Debug, Clone, Copy)]
pub struct Chrome<'a> {
    pub nav: &'a [NavNode],
    pub header: &'a [RuntimePageItem],
    pub footer: &'a [RuntimePageItem],
    pub regions: &'a RegionRows,
}
