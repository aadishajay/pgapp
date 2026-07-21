use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A masked `<input type=password>` — Oracle APEX's "Password" item
/// type. Renders the current value back into the field like any other
/// text input (pgapp has no separate "don't redisplay secrets" mode);
/// pair with `item <field> as password` on a field that's genuinely
/// sensitive knowing the raw value still round-trips through the page
/// same as any other field — this is a display affordance, not an
/// access control.
pub struct Password;

impl ItemType for Password {
    fn kind(&self) -> &'static str {
        "password"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        let required = if args.required { " required" } else { "" };
        format!(r#"<input class="pgapp-input" type="password" name="{name}" value="{value}"{required} autocomplete="new-password">"#)
    }
}
