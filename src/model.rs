#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Id,
    Text,
    Boolean,
    Integer,
    Timestamp,
}

impl FieldType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "id" => Some(FieldType::Id),
            "text" => Some(FieldType::Text),
            "boolean" => Some(FieldType::Boolean),
            "integer" => Some(FieldType::Integer),
            "timestamp" => Some(FieldType::Timestamp),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Id => "id",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamp",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        Self::parse(s).unwrap_or(FieldType::Text)
    }

    /// Column type used when creating the physical data table.
    pub fn sql_column_type(&self) -> &'static str {
        match self {
            FieldType::Id => "serial primary key",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamptz",
        }
    }

    /// Cast used when binding a submitted text value to this column,
    /// and when reading it back out as text (see server.rs).
    pub fn sql_cast(&self) -> &'static str {
        match self {
            FieldType::Id => "integer",
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Integer => "integer",
            FieldType::Timestamp => "timestamptz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: FieldType,
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EntityDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

/// What a page is backed by. `List` and `Detail` pages read/write an
/// entity's data table; `Static` pages are pure composition of page
/// items (text, links) with no entity behind them at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    List,
    Detail,
    Static,
}

impl PageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PageKind::List => "list",
            PageKind::Detail => "detail",
            PageKind::Static => "static",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "detail" => PageKind::Detail,
            "static" => PageKind::Static,
            _ => PageKind::List,
        }
    }
}

/// A named, reusable SQL query. Declared at app scope (visible to every
/// page) or nested inside one `page { }` block (visible only there,
/// shadowing an app-scoped query of the same name). `sql` may contain
/// `:name` bind markers — see `meta::compile_named_query` for how those
/// get turned into safe positional parameters.
#[derive(Debug, Clone)]
pub struct QueryDef {
    pub name: String,
    pub sql: String,
}

/// A page item: content placed on a page beyond its entity-bound
/// table/form. `Link` is how pages reference each other outside of the
/// global nav bar; `Region` renders a named query's rows as a table.
#[derive(Debug, Clone)]
pub enum PageItem {
    Text(String),
    Link { label: String, target_page: String },
    Region { label: String, query: String },
}

/// Turns one report column into a link to another page, passing the
/// row's id as a `?id=` query parameter (the common "click a row to see
/// its detail page" pattern) plus, optionally, other columns from the
/// same row forwarded as additional named query parameters — this is
/// how a value on one page reaches a named query on another page.
#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
    pub extra_params: Vec<(String, String)>,
}

/// Where a Radio/Popup item's choices come from: a fixed list written
/// directly in the markup, or the live result of a named query (which
/// must alias its columns `value` and, optionally, `label`).
#[derive(Debug, Clone)]
pub enum ChoiceSource {
    Static(Vec<String>),
    Query(String),
}

/// How a form field is presented, independent of its Postgres column
/// type.
#[derive(Debug, Clone)]
pub enum FieldItemType {
    Text,
    ReadOnly,
    Checkbox,
    Radio(ChoiceSource),
    Popup(ChoiceSource),
}

impl FieldItemType {
    /// The item type a field gets when a page doesn't declare one
    /// explicitly.
    pub fn default_for(ty: FieldType) -> Self {
        match ty {
            FieldType::Boolean => FieldItemType::Checkbox,
            FieldType::Id => FieldItemType::ReadOnly,
            FieldType::Text | FieldType::Integer | FieldType::Timestamp => FieldItemType::Text,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            FieldItemType::Text => "text",
            FieldItemType::ReadOnly => "readonly",
            FieldItemType::Checkbox => "checkbox",
            FieldItemType::Radio(_) => "radio",
            FieldItemType::Popup(_) => "popup",
        }
    }

    pub fn choice_source(&self) -> Option<&ChoiceSource> {
        match self {
            FieldItemType::Radio(source) | FieldItemType::Popup(source) => Some(source),
            FieldItemType::Text | FieldItemType::ReadOnly | FieldItemType::Checkbox => None,
        }
    }

    /// Reconstructs a resolved item type from `pgapp_meta.page_field_items`:
    /// `choices` is the static list (empty when sourced from a query),
    /// `choices_query` is the query name (empty when static).
    pub fn from_parts(kind: &str, choices: Vec<String>, choices_query: Option<String>) -> Self {
        let source = || match choices_query {
            Some(name) if !name.is_empty() => ChoiceSource::Query(name),
            _ => ChoiceSource::Static(choices),
        };
        match kind {
            "readonly" => FieldItemType::ReadOnly,
            "checkbox" => FieldItemType::Checkbox,
            "radio" => FieldItemType::Radio(source()),
            "popup" => FieldItemType::Popup(source()),
            _ => FieldItemType::Text,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageDef {
    pub name: String,
    pub kind: PageKind,
    pub entity: Option<String>,
    pub columns: Vec<String>,
    pub form: Vec<String>,
    pub link_column: Option<LinkColumn>,
    pub items: Vec<PageItem>,
    /// Explicit `item <field> as <type>` overrides; fields not listed
    /// here get `FieldItemType::default_for` their column type.
    pub item_types: std::collections::HashMap<String, FieldItemType>,
    /// Queries visible only within this page (in addition to the app's).
    pub queries: Vec<QueryDef>,
    /// `source: query <name>` — when set, a `list` page's rows come from
    /// this named query instead of a flat `SELECT * FROM` the entity's
    /// table. Create/update/delete are unaffected: they still write to
    /// the underlying entity by id.
    pub source_query: Option<String>,
}

/// One entry in the app's (possibly multi-level) navigation bar. A leaf
/// links to a page; a group has children and no target of its own.
#[derive(Debug, Clone)]
pub struct NavItem {
    pub label: String,
    pub target_page: Option<String>,
    pub children: Vec<NavItem>,
}

#[derive(Debug, Clone)]
pub struct AppDef {
    pub name: String,
    pub entities: Vec<EntityDef>,
    pub pages: Vec<PageDef>,
    pub nav: Vec<NavItem>,
    /// Shown on every page, above the nav bar / below the footer
    /// respectively. Reuses `PageItem` (text/link/region) — the same
    /// content model as a page's `items`.
    pub header: Vec<PageItem>,
    pub footer: Vec<PageItem>,
    /// Queries visible from every page (a page-scoped query of the same
    /// name takes precedence).
    pub queries: Vec<QueryDef>,
}

impl AppDef {
    pub fn entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.iter().find(|e| e.name == name)
    }
}
