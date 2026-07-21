use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A plain `<select>` dropdown over `args.choices` — Oracle APEX's
/// "Select List" item type. Same choices source as Radio/Popup (a
/// fixed markup-declared list, or a named query's rows); unlike Popup,
/// there's no search box or dialog, just the browser's native list —
/// the right choice for a short, well-known set of options.
pub struct Select;

impl ItemType for Select {
    fn kind(&self) -> &'static str {
        "select"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let required = if args.required { " required" } else { "" };

        let mut html = format!(r#"<select class="pgapp-select" name="{name}"{required}>"#);
        if !args.required {
            html.push_str(r#"<option value="">&mdash;</option>"#);
        }
        for (choice_value, choice_label) in args.choices {
            let selected = if choice_value == args.value { " selected" } else { "" };
            html.push_str(&format!(
                r#"<option value="{cv}"{selected}>{cl}</option>"#,
                cv = escape(choice_value),
                cl = escape(choice_label),
            ));
        }
        html.push_str("</select>");
        html
    }
}
