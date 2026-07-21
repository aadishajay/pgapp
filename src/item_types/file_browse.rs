use super::{ItemType, RenderArgs};
use crate::html::escape;

/// "File Browse" — Oracle APEX's file-upload item type. The stored
/// field value is just `"<file_uploads id>:<original filename>"` (see
/// `pgapp_meta.file_uploads` in `db/schema.sql`) — a plain string this
/// component never needs the database to render, since the filename
/// travels with it. The actual bytes go through a dedicated multipart
/// route (`POST /:workspace/:app/uploads`, see `server.rs`) instead of
/// the universal urlencoded `Form` extractor every other create/update
/// route uses, since that extractor can't carry a real file upload.
///
/// The visible `<input type=file>` carries no `name` of its own — it
/// never submits directly. Picking a file fires `pgapp.uploadFile`
/// (`/runtime.js`), which posts it to the uploads route, then writes
/// the returned `id:filename` into the hidden input that actually
/// submits and updates the download link — the same "one real input,
/// JS keeps it in sync" idiom as `shuttle`/`checkbox_group`/etc.
pub struct FileBrowse;

impl ItemType for FileBrowse {
    fn kind(&self) -> &'static str {
        "file_browse"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(args.value);
        let (id, filename) = args.value.split_once(':').unwrap_or(("", ""));
        let display = if filename.is_empty() { "No file selected".to_string() } else { escape(filename) };
        format!(
            r#"<div class="pgapp-file-browse">
<input type="hidden" name="{name}" value="{value}">
<input type="file" class="pgapp-file-browse-input" onchange="pgapp.uploadFile(this)">
<a class="pgapp-file-browse-link" href="javascript:void(0)" target="_blank" rel="noopener" data-file-id="{id}">{display}</a>
</div>"#,
            id = escape(id),
        )
    }
}
