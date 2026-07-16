use std::collections::HashMap;

use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A real `<input type=checkbox>` — the default for boolean fields.
///
/// Unchecked HTML checkboxes never submit their key at all, so
/// `read_value` treats *presence* of the key as true/false rather than
/// parsing whatever value (if any) came with it.
pub struct Checkbox;

impl ItemType for Checkbox {
    fn kind(&self) -> &'static str {
        "checkbox"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let checked = if args.value == "true" { " checked" } else { "" };
        format!(r#"<input class="pgapp-checkbox" type="checkbox" name="{name}" value="true"{checked}>"#)
    }

    fn read_value(&self, field_name: &str, values: &HashMap<String, String>) -> String {
        if values.contains_key(field_name) {
            "true".to_string()
        } else {
            "false".to_string()
        }
    }
}
