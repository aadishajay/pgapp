use super::{ItemType, RenderArgs};
use crate::html::escape;
use crate::model::FieldType;

/// A plain input, shaped by the underlying column type — the default
/// for text/integer/timestamp fields.
pub struct Text;

impl ItemType for Text {
    fn kind(&self) -> &'static str {
        "text"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        let required = if args.required { " required" } else { "" };

        match args.field_type {
            FieldType::Integer => format!(
                r#"<input class="pgapp-input" type="number" name="{name}" value="{value}"{required}>"#
            ),
            FieldType::Timestamp => format!(
                r#"<input class="pgapp-input" type="text" name="{name}" value="{value}" placeholder="YYYY-MM-DD HH:MM:SS"{required}>"#
            ),
            FieldType::Text | FieldType::Id | FieldType::Boolean => format!(
                r#"<input class="pgapp-input" type="text" name="{name}" value="{value}"{required}>"#
            ),
        }
    }
}
