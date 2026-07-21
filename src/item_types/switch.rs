use std::collections::HashMap;

use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A toggle-switch-styled checkbox — Oracle APEX's "Switch" item type.
/// Functionally identical to `checkbox` (same boolean semantics, same
/// `read_value` override for the "unchecked inputs don't submit"
/// quirk); only the `.pgapp-switch` class differs, so a theme can style
/// it as a sliding toggle instead of a native checkbox square.
pub struct Switch;

impl ItemType for Switch {
    fn kind(&self) -> &'static str {
        "switch"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let checked = if args.value == "true" { " checked" } else { "" };
        format!(
            r#"<label class="pgapp-switch"><input type="checkbox" name="{name}" value="true"{checked}><span class="pgapp-switch-track"></span></label>"#
        )
    }

    fn read_value(&self, field_name: &str, values: &HashMap<String, String>) -> String {
        if values.contains_key(field_name) {
            "true".to_string()
        } else {
            "false".to_string()
        }
    }
}
