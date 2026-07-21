use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A native browser date picker (`<input type="date">`) for a `text`
/// field storing a plain `YYYY-MM-DD` value — pgapp has no dedicated
/// date column type (see `FieldType`), so this is the item-type-level
/// equivalent of Oracle APEX's date-picker page item. `min`/`max`
/// config keys (each optional, same `YYYY-MM-DD` shape) bound the
/// pickable range, same convention as `slider`'s `min`/`max`.
pub struct DatePicker;

impl ItemType for DatePicker {
    fn kind(&self) -> &'static str {
        "date"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        let required = if args.required { " required" } else { "" };
        let min = config_attr(args.config, "min", "min");
        let max = config_attr(args.config, "max", "max");

        format!(r#"<input class="pgapp-input" type="date" name="{name}" value="{value}"{min}{max}{required}>"#)
    }
}

fn config_attr(config: &serde_json::Value, key: &str, attr: &str) -> String {
    match config.get(key).and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => format!(r#" {attr}="{}""#, escape(v)),
        _ => String::new(),
    }
}
