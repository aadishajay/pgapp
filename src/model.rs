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

/// How a form field is presented, independent of its Postgres column
/// type. `Radio` and `Popup` carry a static list of choices (a "static
/// LOV" in APEX terms — no lookup against another entity yet).
#[derive(Debug, Clone)]
pub enum FieldItemType {
    Text,
    ReadOnly,
    Checkbox,
    Radio(Vec<String>),
    Popup(Vec<String>),
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

    pub fn choices(&self) -> &[String] {
        match self {
            FieldItemType::Radio(choices) | FieldItemType::Popup(choices) => choices,
            FieldItemType::Text | FieldItemType::ReadOnly | FieldItemType::Checkbox => &[],
        }
    }

    pub fn from_parts(kind: &str, choices: Vec<String>) -> Self {
        match kind {
            "readonly" => FieldItemType::ReadOnly,
            "checkbox" => FieldItemType::Checkbox,
            "radio" => FieldItemType::Radio(choices),
            "popup" => FieldItemType::Popup(choices),
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
    /// respectively. Reuses `PageItem` (text/link) — the same content
    /// model as a page's `items`.
    pub header: Vec<PageItem>,
    pub footer: Vec<PageItem>,
}

impl AppDef {
    pub fn entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.iter().find(|e| e.name == name)
    }
}
