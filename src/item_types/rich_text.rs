use std::collections::HashMap;

use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A WYSIWYG editor — Oracle APEX's "Rich Text Editor" item type.
/// Dependency-free by design: a `contenteditable` `<div>` with a small
/// `document.execCommand`-driven toolbar, kept in sync with a hidden
/// input the same way `popup`/`checkbox_group`/etc are (see
/// `pgapp.syncRichText` in `/runtime.js`), since `server.rs`'s form
/// parsing only ever reads plain input values.
///
/// The stored value is HTML, submitted straight from the browser — so
/// `read_value` runs it through `ammonia::clean` (an allow-list HTML
/// sanitizer) before it's ever persisted, closing the stored-XSS hole a
/// naive "just save what the browser sent" implementation would open.
/// Because of that sanitization, the value read back out of the
/// database is already safe to re-inject as HTML (not text) into the
/// editor `<div>` on render — that's what makes editing existing
/// content round-trip instead of showing escaped tags.
pub struct RichText;

impl ItemType for RichText {
    fn kind(&self) -> &'static str {
        "rich_text"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value_attr = escape(args.value);
        // Not escaped: already-sanitized HTML, meant to render as markup
        // inside the editable div (see doc comment above).
        let value_html = args.value;
        format!(
            r#"<div class="pgapp-rich-text">
<div class="pgapp-rich-text-toolbar">
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('bold')"><b>B</b></button>
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('italic')"><i>I</i></button>
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('underline')"><u>U</u></button>
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('insertUnorderedList')">&#8226; List</button>
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('insertOrderedList')">1. List</button>
<button type="button" tabindex="-1" onmousedown="event.preventDefault()" onclick="document.execCommand('formatBlock', false, 'blockquote')">&#10077;&#10078;</button>
</div>
<input type="hidden" name="{name}" value="{value_attr}">
<div class="pgapp-rich-text-editor" contenteditable="true" oninput="pgapp.syncRichText(this)">{value_html}</div>
</div>"#
        )
    }

    fn read_value(&self, field_name: &str, values: &HashMap<String, String>) -> String {
        let raw = values.get(field_name).cloned().unwrap_or_default();
        ammonia::clean(&raw)
    }
}
