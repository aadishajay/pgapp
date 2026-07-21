use super::{ItemType, RenderArgs};
use crate::html::escape;

/// An add/remove/reorder-free list of short text entries — Oracle
/// APEX's "List Manager" item type (APEX joins entries with `:`; this
/// stores a comma-joined list instead, matching pgapp's own
/// `checkbox_group`/`shuttle` convention for "one field, several
/// values"). Entries render as plain `<li>`s with no `name` of their
/// own; one hidden input under the real field name holds the current
/// comma-joined value, kept in sync by `pgapp.addListManagerItem`/
/// `removeListManagerItem` (runtime.js) on every add/remove.
pub struct ListManager;

impl ItemType for ListManager {
    fn kind(&self) -> &'static str {
        "list_manager"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let items: Vec<&str> = args.value.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        let mut html = format!(
            r#"<div class="pgapp-list-manager"><input type="hidden" name="{name}" value="{value}"><ul class="pgapp-list-manager-items">"#,
            value = escape(args.value),
        );
        for item in &items {
            html.push_str(&format!(
                r#"<li><span>{item}</span><button type="button" class="pgapp-icon-btn pgapp-icon-btn-destructive" onclick="pgapp.removeListManagerItem(this)">&#10005;</button></li>"#,
                item = escape(item),
            ));
        }
        html.push_str(
            r#"</ul><div class="pgapp-list-manager-add"><input type="text" class="pgapp-input" placeholder="Add an item&hellip;" onkeydown="if(event.key==='Enter'){event.preventDefault();pgapp.addListManagerItem(this);}"><button type="button" class="pgapp-btn pgapp-btn-secondary" onclick="pgapp.addListManagerItem(this.previousElementSibling)">+ Add</button></div></div>"#,
        );
        html
    }
}
