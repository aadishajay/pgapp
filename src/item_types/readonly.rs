use super::{ItemType, RenderArgs};
use crate::html::escape;

/// Displays the value but isn't editable; the value still round-trips
/// via a hidden input so it survives a form submit unchanged.
pub struct ReadOnly;

impl ItemType for ReadOnly {
    fn kind(&self) -> &'static str {
        "readonly"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        format!(
            r#"<span class="pgapp-readonly">{value}</span><input type="hidden" name="{name}" value="{value}">"#
        )
    }
}
