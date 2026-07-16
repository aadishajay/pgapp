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

/// A page item: content placed on a page beyond its entity-bound
/// table/form. `Link` is how pages reference each other outside of the
/// global nav bar.
#[derive(Debug, Clone)]
pub enum PageItem {
    Text(String),
    Link { label: String, target_page: String },
}

/// Turns one report column into a link to another page, passing the
/// row's id as a `?id=` query parameter — the common "click a row to
/// see its detail page" pattern.
#[derive(Debug, Clone)]
pub struct LinkColumn {
    pub field: String,
    pub target_page: String,
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
}

impl AppDef {
    pub fn entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.iter().find(|e| e.name == name)
    }
}
