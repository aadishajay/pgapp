use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A native `<input type=color>` — Oracle APEX's "Color Picker" item
/// type. Stores a `#rrggbb` hex string in a `text` field, same
/// round-trip story as `date`. Browsers require a value in that exact
/// shape, so an empty/invalid stored value falls back to `#000000`
/// rather than leaving the control in an undefined state.
pub struct ColorPicker;

impl ItemType for ColorPicker {
    fn kind(&self) -> &'static str {
        "color"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = if is_valid_hex_color(args.value) { args.value } else { "#000000" };
        format!(r#"<input class="pgapp-input pgapp-color-input" type="color" name="{name}" value="{value}">"#)
    }
}

fn is_valid_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}
