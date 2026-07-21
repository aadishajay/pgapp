use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A multi-line `<textarea>` — Oracle APEX's "Textarea" item type.
/// `rows` config key (optional, default 4) controls its height, same
/// convention as slider's `min`/`max`.
pub struct Textarea;

impl ItemType for Textarea {
    fn kind(&self) -> &'static str {
        "textarea"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        let required = if args.required { " required" } else { "" };
        let rows = args
            .config
            .get("rows")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(4);
        format!(r#"<textarea class="pgapp-input" name="{name}" rows="{rows}"{required}>{value}</textarea>"#)
    }
}
