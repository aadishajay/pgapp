//! Pluggable form-field components ("page item types" in APEX terms).
//!
//! Adding a new one — say a date picker — means adding one file here
//! implementing [`ItemType`] and one line in [`registry`]. Nothing in
//! `markup.rs`, `meta.rs`, `server.rs`, or `render.rs` needs to change:
//! they only ever go through the [`Registry`] by kind string, and a
//! field's config is a generic `serde_json::Value` no component-specific
//! code elsewhere needs to know the shape of.
//!
//! This is a *compile-time* plugin point, not a hot-loading one — Rust
//! has no way to pick up a dropped-in `.rs` file without a rebuild.
//! "Drop in a file" here means: write the file, register it, rebuild,
//! restart.

use std::collections::HashMap;

use crate::model::FieldType;

mod checkbox;
mod popup;
mod radio;
mod readonly;
mod slider;
mod text;

/// Everything one item type needs to render its input. `choices` is the
/// generic (value, label) list resolved from the field's config —
/// either a literal list or a named query's rows — for types that use
/// it (Radio, Popup, ...); types that ignore config entirely (Text,
/// Checkbox, ReadOnly) just don't read it.
pub struct RenderArgs<'a> {
    pub field_name: &'a str,
    pub value: &'a str,
    pub required: bool,
    pub field_type: FieldType,
    pub config: &'a serde_json::Value,
    pub choices: &'a [(String, String)],
}

/// One pluggable form-field component.
pub trait ItemType: Send + Sync {
    /// The markup keyword naming this type, e.g. `"slider"` for
    /// `item x as slider (...)`.
    fn kind(&self) -> &'static str;

    /// Renders the `<input>`/etc for this field — just the control
    /// itself; the surrounding `<label>`/field wrapper is render.rs's
    /// job, not this component's.
    fn render(&self, args: RenderArgs) -> String;

    /// Reads this field's value out of a submitted form. The default —
    /// the raw submitted value, or `""` if the key is absent — is right
    /// for most inputs. Override it for anything that doesn't submit a
    /// plain value the usual way (a checkbox's key is only present when
    /// checked, for instance).
    fn read_value(&self, field_name: &str, values: &HashMap<String, String>) -> String {
        values.get(field_name).cloned().unwrap_or_default()
    }
}

pub type Registry = HashMap<&'static str, Box<dyn ItemType>>;

/// Builds the registry of every known item type. This is the one line
/// a new component needs outside of its own file.
pub fn registry() -> Registry {
    let components: Vec<Box<dyn ItemType>> = vec![
        Box::new(text::Text),
        Box::new(readonly::ReadOnly),
        Box::new(checkbox::Checkbox),
        Box::new(radio::Radio),
        Box::new(popup::Popup),
        Box::new(slider::Slider),
    ];
    components.into_iter().map(|c| (c.kind(), c)).collect()
}

/// The item type a field gets when a page doesn't declare one
/// explicitly. Kept centralized rather than made part of the trait:
/// it's a policy about which *registered* type wins by default for a
/// column type, not a property any one component owns.
pub fn default_kind_for(ty: FieldType) -> &'static str {
    match ty {
        FieldType::Boolean => "checkbox",
        FieldType::Id => "readonly",
        FieldType::Text | FieldType::Integer | FieldType::Timestamp => "text",
    }
}
