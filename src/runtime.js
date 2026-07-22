// pgapp runtime — stored in Postgres (pgapp_meta.app_runtime_js), served
// at /runtime.js. This file is only the seed: it's written into the
// database once (on first sync, ON CONFLICT DO NOTHING) and can be
// edited there afterward without touching the binary.
//
// Primary purpose: capture/set a page item's value by a method call
// instead of hand-rolled DOM lookups. An "item" is anything named
// consistently across its inputs — a plain text/number input, a
// checkbox, a radio group (several <input>s sharing one name), or a
// popup LOV's hidden input — and getItem/setItem understand all of
// them the same way.
window.pgapp = (function () {
  function elements(name) {
    return document.getElementsByName(name);
  }

  function getItem(name) {
    var els = elements(name);
    if (!els.length) return null;
    var first = els[0];
    if (first.type === "checkbox") return first.checked ? "true" : "false";
    if (first.type === "radio") {
      for (var i = 0; i < els.length; i++) {
        if (els[i].checked) return els[i].value;
      }
      return null;
    }
    return first.value;
  }

  function setItem(name, value) {
    var els = elements(name);
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      if (el.type === "checkbox") {
        el.checked = value === "true" || value === true;
      } else if (el.type === "radio") {
        el.checked = el.value === String(value);
      } else {
        el.value = value;
      }
      el.dispatchEvent(new Event("change", { bubbles: true }));
    }
    // Popup LOVs show their current choice in a companion span, by
    // convention named pgapp-popup-display-<item name>.
    var display = document.getElementById("pgapp-popup-display-" + name);
    if (display) display.textContent = value;
  }

  // ---- dynamic actions ----
  //
  // Pages emit their `on <event> of <item> { ... }` blocks as a JSON
  // <script class="pgapp-dynamic-actions"> blob; this dispatcher binds
  // them. Ops: show/hide/toggle a field wrapper ([data-pgapp-item]),
  // set another item from a JS expression, or refresh a region
  // ([data-pgapp-region]) by re-fetching its rows with the page's
  // current item values as query bind parameters.

  var daDepth = 0; // guard: setItem fires change events, which may chain

  function evalExpr(expr) {
    try {
      return new Function("pgapp", "return (" + expr + ");")(window.pgapp);
    } catch (e) {
      console.error("pgapp: dynamic action expression failed:", expr, e);
      return null;
    }
  }

  function setItemVisible(name, visible) {
    var wrappers = document.querySelectorAll('[data-pgapp-item="' + name + '"]');
    for (var i = 0; i < wrappers.length; i++) {
      wrappers[i].style.display = visible ? "" : "none";
    }
  }

  function collectItemParams() {
    var params = new URLSearchParams();
    var named = document.querySelectorAll("[name]");
    var seen = {};
    for (var i = 0; i < named.length; i++) {
      var name = named[i].getAttribute("name");
      if (!name || seen[name]) continue;
      seen[name] = true;
      var value = getItem(name);
      if (value !== null && value !== "") params.set(name, value);
    }
    // Cross-page parameters (?id=, forwarded link params) still apply.
    new URLSearchParams(location.search).forEach(function (v, k) {
      if (!params.has(k)) params.set(k, v);
    });
    return params;
  }

  function refreshRegion(query) {
    var container = document.querySelector('[data-pgapp-region="' + query + '"]');
    if (!container) return;
    var url =
      location.pathname + "/region/" + encodeURIComponent(query) + "?" + collectItemParams().toString();
    fetch(url)
      .then(function (r) {
        if (!r.ok) throw new Error("region refresh failed: " + r.status);
        return r.text();
      })
      .then(function (html) {
        container.outerHTML = html;
      })
      .catch(function (e) {
        console.error("pgapp:", e);
      });
  }

  function runOps(ops, componentIdx) {
    if (daDepth > 8) return; // break show/set feedback loops
    daDepth++;
    try {
      for (var i = 0; i < ops.length; i++) {
        var op = ops[i];
        if (op.op === "show") setItemVisible(op.item, true);
        else if (op.op === "hide") setItemVisible(op.item, false);
        else if (op.op === "toggle") setItemVisible(op.item, !!evalExpr(op.when));
        else if (op.op === "set") setItem(op.item, String(evalExpr(op.expr)));
        else if (op.op === "refresh") refreshRegion(op.query);
        else if (op.op === "call") runDynamicActionCall(componentIdx, i, op.target);
      }
    } finally {
      daDepth--;
    }
  }

  // The "ajax callback" (model::DaOp::Call): posts to the
  // DynamicAction component's own /c/:idx/call/:op_idx route
  // (server.rs's call_dynamic_action) with the page's current item
  // values as the body — same params refreshRegion sends, and the same
  // server-side action-module dispatch run_action uses, just returning
  // JSON instead of a redirect. Applying the result: if `target` names
  // a region/query currently on the page, that region gets refreshed
  // (the callback's own result string is just the trigger, not
  // injected directly — see model::DaOp::Call's doc); otherwise
  // `target` is treated as an item and set to the result string.
  function runDynamicActionCall(componentIdx, opIdx, target) {
    var url = location.pathname + "/c/" + componentIdx + "/call/" + opIdx;
    fetch(url, { method: "POST", body: collectItemParams() })
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        if (!data.ok) throw new Error(data.error || "call failed");
        if (document.querySelector('[data-pgapp-region="' + target + '"]')) {
          refreshRegion(target);
        } else {
          setItem(target, data.result);
        }
      })
      .catch(function (e) {
        pgappAlert("Ajax callback failed: " + e.message);
      });
  }

  function bindDynamicActions() {
    var script = document.querySelector("script.pgapp-dynamic-actions");
    if (!script) return;
    var actions;
    try {
      actions = JSON.parse(script.textContent);
    } catch (e) {
      console.error("pgapp: bad dynamic-actions JSON", e);
      return;
    }
    actions.forEach(function (da) {
      var els = elements(da.item);
      for (var i = 0; i < els.length; i++) {
        els[i].addEventListener(da.event, function () {
          runOps(da.ops, da.idx);
        });
      }
    });
  }

  // ---- popup LOV search ----
  //
  // A popup LOV's dialog (see item_types/popup.rs) carries a search
  // input above its <ul class="pgapp-popup-list">; openPopup resets it
  // fresh every time the dialog opens (so a stale filter from last time
  // never lingers), and filterPopup hides/shows <li>s by substring match
  // against their own rendered text — no server round trip, since every
  // choice is already in the DOM.

  function filterPopup(dialogId, query) {
    var dialog = document.getElementById(dialogId);
    if (!dialog) return;
    var q = query.trim().toLowerCase();
    var items = dialog.querySelectorAll(".pgapp-popup-list li:not(.pgapp-popup-empty)");
    var visible = 0;
    for (var i = 0; i < items.length; i++) {
      var li = items[i];
      var match = q === "" || (li.textContent || "").toLowerCase().indexOf(q) !== -1;
      li.style.display = match ? "" : "none";
      if (match) visible++;
    }
    var empty = dialog.querySelector(".pgapp-popup-empty");
    if (empty) empty.style.display = visible === 0 ? "" : "none";
  }

  function openPopup(dialogId, searchId) {
    var dialog = document.getElementById(dialogId);
    if (!dialog) return;
    var search = document.getElementById(searchId);
    if (search) {
      search.value = "";
      filterPopup(dialogId, "");
    }
    dialog.showModal();
    if (search) search.focus();
  }

  // Keeps a `checkbox_group` item's one real (hidden) input in sync
  // with whichever of its display-only checkboxes are currently
  // checked — called on every one of their onchange (see
  // `item_types::checkbox_group`). `checkboxEl` is any checkbox inside
  // the group; its `.pgapp-checkbox-group` ancestor holds both the
  // hidden input and every sibling checkbox.
  function syncCheckboxGroup(checkboxEl) {
    var group = checkboxEl.closest(".pgapp-checkbox-group");
    if (!group) return;
    var hidden = group.querySelector('input[type="hidden"]');
    if (!hidden) return;
    var checked = group.querySelectorAll('input[type="checkbox"]:checked');
    var values = [];
    for (var i = 0; i < checked.length; i++) values.push(checked[i].value);
    hidden.value = values.join(",");
    hidden.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // A `star_rating` item's click handler (see `item_types::star_rating`):
  // fills every star up to `value` and writes it into the group's one
  // real (hidden) input.
  function setStarRating(starEl, value) {
    var wrapper = starEl.closest(".pgapp-star-rating");
    if (!wrapper) return;
    var hidden = wrapper.querySelector('input[type="hidden"]');
    var stars = wrapper.querySelectorAll(".pgapp-star");
    for (var i = 0; i < stars.length; i++) {
      var starValue = parseInt(stars[i].getAttribute("data-value"), 10);
      if (starValue <= value) stars[i].classList.add("pgapp-star-on");
      else stars[i].classList.remove("pgapp-star-on");
    }
    if (hidden) {
      hidden.value = String(value);
      hidden.dispatchEvent(new Event("change", { bubbles: true }));
    }
  }

  // Rebuilds a `list_manager` item's one real (hidden) input from its
  // currently-listed entries, in DOM order (see
  // `item_types::list_manager`).
  function syncListManager(wrapper) {
    var hidden = wrapper.querySelector('input[type="hidden"]');
    if (!hidden) return;
    var items = wrapper.querySelectorAll(".pgapp-list-manager-items li span");
    var values = [];
    for (var i = 0; i < items.length; i++) values.push(items[i].textContent);
    hidden.value = values.join(",");
    hidden.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // Appends `inputEl`'s current (trimmed) value as a new entry — called
  // from the "+ Add" button or Enter in a `list_manager`'s text input.
  // A no-op on an empty/whitespace-only value.
  function addListManagerItem(inputEl) {
    var value = inputEl.value.trim();
    if (!value) return;
    var wrapper = inputEl.closest(".pgapp-list-manager");
    if (!wrapper) return;
    var ul = wrapper.querySelector(".pgapp-list-manager-items");
    var li = document.createElement("li");
    var span = document.createElement("span");
    span.textContent = value;
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
    btn.textContent = "✕";
    btn.addEventListener("click", function () {
      removeListManagerItem(btn);
    });
    li.appendChild(span);
    li.appendChild(btn);
    ul.appendChild(li);
    inputEl.value = "";
    inputEl.focus();
    syncListManager(wrapper);
  }

  // Removes `btnEl`'s own `<li>` from a `list_manager` — `btnEl` is
  // either a server-rendered delete button (inline `onclick`) or one
  // `addListManagerItem` just created (bound via `addEventListener`),
  // both call this the same way.
  function removeListManagerItem(btnEl) {
    var wrapper = btnEl.closest(".pgapp-list-manager");
    var li = btnEl.closest("li");
    if (li) li.remove();
    if (wrapper) syncListManager(wrapper);
  }

  // Moves every highlighted `<option>` between a `shuttle` item's two
  // `<select multiple>`s (`toRight`: available -> selected, or back)
  // and rebuilds the one real hidden input from the selected list's
  // resulting option order (see `item_types::shuttle`). Moving the
  // *whole* `<option>` node (not just its value) preserves each
  // choice's label without re-deriving it from anywhere.
  function shuttleMove(btnEl, toRight) {
    var wrapper = btnEl.closest(".pgapp-shuttle");
    if (!wrapper) return;
    var available = wrapper.querySelector(".pgapp-shuttle-available");
    var selected = wrapper.querySelector(".pgapp-shuttle-selected");
    var from = toRight ? available : selected;
    var to = toRight ? selected : available;
    var moving = [];
    for (var i = 0; i < from.options.length; i++) {
      if (from.options[i].selected) moving.push(from.options[i]);
    }
    moving.forEach(function (opt) {
      opt.selected = false;
      to.appendChild(opt);
    });
    var hidden = wrapper.querySelector('input[type="hidden"]');
    if (hidden) {
      var values = [];
      for (var j = 0; j < selected.options.length; j++) values.push(selected.options[j].value);
      hidden.value = values.join(",");
      hidden.dispatchEvent(new Event("change", { bubbles: true }));
    }
  }

  // The rich_text item type's contenteditable <div> carries no `name`
  // of its own; this keeps the preceding hidden input (the field that
  // actually submits) in sync with the editor's current HTML on every
  // edit — same "one real input, JS keeps it in sync" idiom as
  // shuttle/checkbox_group/etc above. Server-side sanitization (see
  // item_types::rich_text) is what makes it safe to persist and
  // re-render this HTML rather than a pre-escaped copy of it.
  function syncRichText(editorEl) {
    var hidden = editorEl.previousElementSibling;
    if (hidden && hidden.tagName === "INPUT") {
      hidden.value = editorEl.innerHTML;
    }
  }

  // Every app route is "/<workspace>/<app>/...", so the upload
  // endpoint's own prefix can always be derived from the current page's
  // own URL — no need for item_types::file_browse's render() to know
  // the app's URL prefix (it doesn't have it) or for a template
  // placeholder to be baked into the HTML at render time.
  function fileUploadsUrl() {
    var parts = window.location.pathname.split("/").filter(Boolean);
    return "/" + parts[0] + "/" + parts[1] + "/uploads";
  }

  // The file_browse item type's <input type=file> carries no `name` of
  // its own; picking a file here posts it to the dedicated multipart
  // upload route (server.rs's upload_file — the one route that isn't
  // the universal urlencoded Form extractor) and writes the returned
  // "id:filename" into the preceding hidden input, which is what
  // actually submits — same idiom as syncRichText/shuttleMove/etc.
  function uploadFile(inputEl) {
    var file = inputEl.files && inputEl.files[0];
    if (!file) return;
    var wrapper = inputEl.closest(".pgapp-file-browse");
    if (!wrapper) return;
    var hidden = wrapper.querySelector('input[type="hidden"]');
    var link = wrapper.querySelector(".pgapp-file-browse-link");
    var body = new FormData();
    body.append("file", file);
    fetch(fileUploadsUrl(), { method: "POST", body: body })
      .then(function (r) {
        if (!r.ok) throw new Error("upload failed (" + r.status + ")");
        return r.json();
      })
      .then(function (result) {
        if (hidden) hidden.value = result.id + ":" + result.filename;
        if (link) {
          link.textContent = result.filename;
          link.href = fileUploadsUrl() + "/" + result.id;
          link.dataset.fileId = result.id;
        }
      })
      .catch(function (e) {
        pgappAlert("Upload failed: " + e.message);
      });
  }

  // Existing (already-uploaded) file_browse values render with a
  // data-file-id but a placeholder href, since the download URL's
  // workspace/app prefix isn't known at render time either — wired up
  // here from the current page's own URL, same derivation as
  // fileUploadsUrl().
  function wireFileBrowseLinks() {
    document.querySelectorAll(".pgapp-file-browse-link[data-file-id]").forEach(function (el) {
      if (el.dataset.fileId) {
        el.href = fileUploadsUrl() + "/" + el.dataset.fileId;
      }
    });
  }

  // ---- App Builder: drag-and-drop row reordering ----
  //
  // A region/report table wrapped in a ".pgapp-draggable-rows" element
  // (see the App Builder app's "Edit page" page) becomes a reorderable
  // list: every <tbody> row becomes draggable, and dropping one row
  // reorders the DOM. On drop, each row's first cell (by convention the
  // component's id) is read top-to-bottom and POSTed as a comma-
  // separated `order` param — to a URL built from *this page's own*
  // query string (?target_workspace=&target_app=&target_page=), since
  // the row list describes some *other* app's page, not the one
  // currently being viewed (the App Builder itself).
  // Shared by every "act on some *other* app's page" admin affordance
  // below (reorder/preview/add/edit/delete component) — all of them
  // read this page's own ?target_workspace=&target_app=&target_page=,
  // since the App Builder itself never has its own page to act on
  // (see the self-edit guard on the server side of each route).
  function pgappEditTarget() {
    var params = new URLSearchParams(location.search);
    return {
      workspace: params.get("target_workspace"),
      app: params.get("target_app"),
      page: params.get("target_page"),
    };
  }

  function pgappEditTargetValid(target) {
    return !!(target.workspace && target.app && target.page);
  }

  function pgappAdminPagesUrl(target, suffix) {
    return (
      "/" +
      encodeURIComponent(target.workspace) +
      "/" +
      encodeURIComponent(target.app) +
      "/admin/pages/" +
      encodeURIComponent(target.page) +
      suffix
    );
  }

  function saveDraggedOrder(tbody) {
    var target = pgappEditTarget();
    if (!pgappEditTargetValid(target)) {
      console.error("pgapp: draggable rows need ?target_workspace=&target_app=&target_page= on this page's own URL");
      return;
    }
    var ids = [];
    var rows = tbody.querySelectorAll("tr");
    for (var i = 0; i < rows.length; i++) {
      var firstCell = rows[i].querySelector("td");
      if (firstCell) ids.push(firstCell.textContent.trim());
    }
    var url = pgappAdminPagesUrl(target, "/reorder");
    fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: "order=" + encodeURIComponent(ids.join(",")),
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        if (!data.ok) console.error("pgapp: reorder failed:", data.error);
      })
      .catch(function (e) {
        console.error("pgapp:", e);
      });
  }

  function bindDraggableRows() {
    var tbodies = document.querySelectorAll(".pgapp-draggable-rows tbody");
    for (var t = 0; t < tbodies.length; t++) {
      (function (tbody) {
        var dragging = null;
        var rows = tbody.querySelectorAll("tr");
        for (var i = 0; i < rows.length; i++) {
          rows[i].setAttribute("draggable", "true");
        }
        tbody.addEventListener("dragstart", function (ev) {
          dragging = ev.target.closest("tr");
          if (ev.dataTransfer) ev.dataTransfer.effectAllowed = "move";
        });
        tbody.addEventListener("dragover", function (ev) {
          ev.preventDefault();
          var target = ev.target.closest("tr");
          if (!target || target === dragging || !tbody.contains(target)) return;
          var rect = target.getBoundingClientRect();
          var before = (ev.clientY - rect.top) / rect.height < 0.5;
          tbody.insertBefore(dragging, before ? target : target.nextSibling);
        });
        tbody.addEventListener("drop", function (ev) {
          ev.preventDefault();
          if (dragging) saveDraggedOrder(tbody);
          dragging = null;
        });
      })(tbodies[t]);
    }
  }

  // The App Builder's "Preview this page" link: a `text ... attrs (id:
  // "pgapp-preview-slot")` placeholder (see examples/app_builder.pgapp's
  // EditPage) gets a real, working <a> appended into it, built from this
  // page's own ?target_workspace=&target_app=&target_page= — same
  // params saveDraggedOrder already reads, just used to link out to the
  // live app instead of POSTing.
  function bindPreviewLink() {
    var slot = document.getElementById("pgapp-preview-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid(target)) return;
    slot.textContent = "";
    slot.classList.add("pgapp-toolbar-slot");
    var a = document.createElement("a");
    a.className = "pgapp-link pgapp-btn pgapp-btn-secondary";
    a.href = "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/" + encodeURIComponent(target.page);
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = "Run this page ↗";
    slot.appendChild(a);
  }

  // The App Builder's "Advanced" escape hatch: a `text ... attrs (id:
  // "pgapp-advanced-source-slot")` placeholder (see examples/
  // app_builder.pgapp's Pages page) gets a link to the target app's
  // own, already-existing `/admin/reload` page — a full-file raw
  // markup editor built into every app (see `admin_reload_page` in
  // server.rs), not something the App Builder adds. Entities, queries,
  // nav, header/footer, and app-level settings (theme/auth/icons) have
  // no dedicated GUI here — this is how to reach them without SSHing
  // in to hand-edit the file.
  function bindAdvancedSourceLink() {
    var slot = document.getElementById("pgapp-advanced-source-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    slot.textContent = "";
    slot.classList.add("pgapp-toolbar-slot");
    var a = document.createElement("a");
    a.className = "pgapp-link pgapp-btn pgapp-btn-secondary";
    a.href = "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin/reload";
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = "Advanced: edit full app source ↗";
    slot.appendChild(a);
  }

  // A link from the Pages screen to the "AppSettings" page (data
  // model, queries, nav, app settings, teardown) — same "carry
  // ?target_workspace=&target_app= forward" pattern as
  // `bindAdvancedSourceLink`, just staying inside the App Builder
  // itself rather than opening the target app's own raw editor.
  function bindAppSettingsLink() {
    var slot = document.getElementById("pgapp-app-settings-link-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    slot.textContent = "";
    slot.classList.add("pgapp-toolbar-slot");
    var a = document.createElement("a");
    a.className = "pgapp-link pgapp-btn pgapp-btn-secondary";
    a.href =
      "AppSettings?target_workspace=" + encodeURIComponent(target.workspace) + "&target_app=" + encodeURIComponent(target.app);
    a.textContent = "Data Model, Queries, Nav & Settings →";
    slot.appendChild(a);
  }

  // The App Builder's breadcrumb: a `text ... attrs (id:
  // "pgapp-context-slot")` placeholder (on both the Pages and EditPage
  // pages) gets filled with which app/page is actually being edited —
  // otherwise that's only visible in the URL's own query string, not
  // anywhere in the page itself. `target_page` is absent on the Pages
  // page (you haven't picked one yet), present on EditPage.
  function bindContextHeader() {
    var slot = document.getElementById("pgapp-context-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!target.workspace || !target.app) return;
    var text = "Editing " + target.workspace + "/" + target.app;
    if (target.page) text += " — " + target.page;
    slot.textContent = text;
    slot.classList.add("pgapp-context-header");
  }

  // The App Builder's "Add Page" panel on the Pages page: a `text ...
  // attrs (id: "pgapp-add-page-slot")` placeholder gets a name input +
  // button, POSTing to the target app's own `/admin/pages/add`.
  function bindAddPageForm() {
    var slot = document.getElementById("pgapp-add-page-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;

    slot.textContent = "";
    slot.classList.add("pgapp-panel-card");
    var title = document.createElement("div");
    title.className = "pgapp-panel-card-title";
    title.textContent = "Add Page";
    slot.appendChild(title);

    var form = document.createElement("form");
    form.className = "pgapp-add-component-form";
    var nameInput = document.createElement("input");
    nameInput.className = "pgapp-input";
    nameInput.placeholder = "Page name";
    var addBtn = document.createElement("button");
    addBtn.type = "submit";
    addBtn.className = "pgapp-btn pgapp-btn-primary";
    addBtn.textContent = "Add";
    form.appendChild(nameInput);
    form.appendChild(addBtn);
    slot.appendChild(form);

    form.addEventListener("submit", function (ev) {
      ev.preventDefault();
      fetch(
        "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin/pages/add",
        {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: "name=" + encodeURIComponent(nameInput.value),
        }
      )
        .then(function (r) {
          return r.json();
        })
        .then(function (data) {
          if (data.ok) location.reload();
          else pgappAlert("Couldn't add page: " + data.error);
        })
        .catch(function (e) {
          pgappAlert("pgapp: " + e);
        });
    });
  }

  // The Pages page's target only ever carries workspace/app (no page
  // yet — you're here to pick one), unlike EditPage's, which also
  // needs target_page. A small variant of pgappEditTargetValid for
  // call sites that don't need the page part.
  function pgappEditTargetValid2(target) {
    return !!(target.workspace && target.app);
  }

  // A "Rename"/"Delete" button per row of the Pages page's Interactive
  // Report (`#pgapp-pages-report`) — the page's own name is the row's
  // visible link text, so no hidden id column is needed the way the
  // component list needs one for its ordinal-based `idx`. The buttons
  // land in their own trailing `<td>` (a plain IR row, not a
  // `.pgapp-cards` tile, so a bare `<button>` can't be a direct `<tr>`
  // child).
  function bindPageCardActions() {
    var rows = document.querySelectorAll("#pgapp-pages-report tbody tr");
    if (!rows.length) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    for (var i = 0; i < rows.length; i++) {
      (function (row) {
        var link = row.querySelector("a.pgapp-link");
        var pageName = link ? link.textContent.trim() : row.textContent.trim();
        var pageUrl =
          "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin/pages/" + encodeURIComponent(pageName);

        var actionsTd = document.createElement("td");
        var renameBtn = document.createElement("button");
        renameBtn.type = "button";
        renameBtn.className = "pgapp-icon-btn";
        renameBtn.title = "Rename page";
        renameBtn.setAttribute("aria-label", "Rename page");
        renameBtn.textContent = "✎";
        renameBtn.addEventListener("click", function (ev) {
          ev.preventDefault();
          ev.stopPropagation();
          pgappPrompt("New name for this page:", pageName).then(function (newName) {
            if (newName === null || newName === "" || newName === pageName) return;
            fetch(pageUrl + "/rename", {
              method: "POST",
              headers: { "Content-Type": "application/x-www-form-urlencoded" },
              body: "new_name=" + encodeURIComponent(newName),
            })
              .then(function (r) {
                return r.json();
              })
              .then(function (data) {
                if (data.ok) location.reload();
                else pgappAlert("Couldn't rename page: " + data.error);
              })
              .catch(function (e) {
                pgappAlert("pgapp: " + e);
              });
          });
        });
        actionsTd.appendChild(renameBtn);

        var btn = document.createElement("button");
        btn.type = "button";
        btn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
        btn.title = "Delete page";
        btn.setAttribute("aria-label", "Delete page");
        btn.textContent = "✕";
        btn.addEventListener("click", function (ev) {
          ev.preventDefault();
          ev.stopPropagation();
          pgappConfirm('Delete page "' + pageName + '" and all its components? This can\'t be undone.').then(function (ok) {
            if (!ok) return;
            fetch(pageUrl + "/delete", { method: "POST" })
              .then(function (r) {
                return r.json();
              })
              .then(function (data) {
                if (data.ok) location.reload();
                else pgappAlert("Couldn't delete page: " + data.error);
              })
              .catch(function (e) {
                pgappAlert("pgapp: " + e);
              });
          });
        });
        actionsTd.appendChild(btn);
        row.appendChild(actionsTd);
      })(rows[i]);
    }
  }

  // ---- App Builder: entities, queries, nav, app settings, teardown ----
  //
  // The "full data model" layer above pages/components: every panel
  // below lives on the App Builder's "AppSettings" page (see
  // examples/app_builder.pgapp), reached with the same
  // ?target_workspace=&target_app= this page's own URL already carries
  // (no ?target_page= needed — these are all app-wide, not page-scoped
  // — see `pgappEditTargetValid2`). Same shape as everything above:
  // a `text ... attrs (id: "pgapp-...-slot")` placeholder gets replaced
  // client-side, fetches list data fresh from the server (never the
  // page's own static HTML), and every mutation POSTs then
  // `location.reload()`s on success.

  function pgappAdminAppUrl(target, suffix) {
    return "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin" + suffix;
  }

  // A minimal modal shell shared by the entity/query/nav-item editors
  // below — same visual chrome as `pgappStructuredEditor`, but driven
  // directly by a `renderFn(body)` (returning `{generate}`) instead of
  // a `PGAPP_KIND_RENDERERS` lookup, since none of these are component
  // kinds.
  function pgappGenericEditor(dialogTitle, renderFn) {
    return new Promise(function (resolve) {
      pgappEnsureBuilderStyle();
      var overlay = document.createElement("div");
      overlay.className = "pgapp-dialog-overlay";
      var box = document.createElement("div");
      box.className = "pgapp-dialog-box pgapp-builder-form-box";
      box.setAttribute("role", "alertdialog");
      box.setAttribute("aria-modal", "true");
      var p = document.createElement("p");
      p.className = "pgapp-dialog-message";
      p.textContent = dialogTitle;
      box.appendChild(p);

      var body = document.createElement("div");
      body.className = "pgapp-builder-form-body";
      box.appendChild(body);
      var rendered = renderFn(body);

      var actions = document.createElement("div");
      actions.className = "pgapp-dialog-actions";
      var cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "pgapp-btn pgapp-btn-secondary";
      cancelBtn.textContent = "Cancel";
      var saveBtn = document.createElement("button");
      saveBtn.type = "button";
      saveBtn.className = "pgapp-btn pgapp-btn-primary";
      saveBtn.textContent = "Save";

      function cleanup() {
        document.removeEventListener("keydown", onKey);
        overlay.remove();
      }
      function onKey(ev) {
        if (ev.key === "Escape") {
          cleanup();
          resolve(null);
        }
      }
      cancelBtn.addEventListener("click", function () {
        cleanup();
        resolve(null);
      });
      saveBtn.addEventListener("click", function () {
        var text;
        try {
          text = rendered.generate();
        } catch (e) {
          pgappAlert("Couldn't build markup from the form: " + e);
          return;
        }
        cleanup();
        resolve(text);
      });
      actions.appendChild(cancelBtn);
      actions.appendChild(saveBtn);
      box.appendChild(actions);
      overlay.appendChild(box);
      document.body.appendChild(overlay);
      document.addEventListener("keydown", onKey);
    });
  }

  var ENTITY_FIELD_TYPES = ["id", "text", "boolean", "integer", "timestamp"];

  // Renders the entity structured form into `container`; `data` is
  // `{name, source_query, source_collection, fields: [{name, type,
  // required, default}]}` (blank/absent for a brand-new entity — see
  // `admin_entities_list`'s JSON shape in server.rs). `queries` (from
  // the same fetch) populates the "from query" dropdown. `columns` (from
  // `/admin/schema-metadata`, only non-empty for an *existing* physical
  // entity) suggests real Postgres column names for the field-name
  // input — a datalist, not a hard dropdown, since adding a field that
  // doesn't exist in the table yet is normal (the next sync creates it).
  function renderEntityForm(container, data, queries, columns) {
    var nameInput = pgappTextInput(data.name);
    if (data.name) nameInput.disabled = true; // renaming isn't supported here — delete + recreate, or the Advanced editor
    pgappFieldRow(container, "Name", nameInput);

    var sourceSel = pgappSelect(
      ["physical table"].concat((queries || []).map(function (q) { return "from query " + q; })),
      data.source_query ? "from query " + data.source_query : "physical table"
    );
    pgappFieldRow(container, "Source", sourceSel);

    pgappSectionTitle(container, "Fields");
    if (columns && columns.length) {
      var hint = document.createElement("p");
      hint.className = "pgapp-designer-properties-empty";
      hint.textContent = "Name suggestions come from the table's real columns.";
      container.appendChild(hint);
    }
    var fieldsList = pgappRowList(
      [
        { key: "name", label: "Name", type: "datalist", options: columns || [] },
        { key: "type", label: "Type", type: "select", options: ENTITY_FIELD_TYPES },
        { key: "required", label: "Required", type: "checkbox" },
        { key: "default", label: "Default", type: "text" },
      ],
      (data.fields || []).map(function (f) {
        return { name: f.name, type: f.type, required: !!f.required, default: f.default || "" };
      })
    );
    container.appendChild(fieldsList.el);

    return {
      generate: function () {
        var name = nameInput.value.trim();
        if (!name) throw "an entity needs a name";
        var fromClause = "";
        if (sourceSel.value !== "physical table") {
          fromClause = " from query " + sourceSel.value.replace(/^from query /, "");
        }
        var lines = ["  entity " + pgappMarkupStr(name) + fromClause + " {"];
        fieldsList.getRows().forEach(function (r) {
          if (!r.name || !r.name.trim()) return;
          var line = "    field " + r.name.trim() + ": " + (r.type || "text");
          if (r.required) line += " required";
          if (r.default && r.default.trim()) line += " default " + r.default.trim();
          lines.push(line);
        });
        lines.push("  }");
        return lines.join("\n");
      },
    };
  }

  // The "Data Model" panel — a `text ... attrs (id:
  // "pgapp-entities-slot")` placeholder.
  function bindEntitiesPanel() {
    var slot = document.getElementById("pgapp-entities-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    pgappEnsureBuilderStyle();

    function render() {
      fetch(pgappAdminAppUrl(target, "/entities-list"))
        .then(function (r) { return r.json(); })
        .then(function (data) {
          if (!data.ok) {
            slot.textContent = "Couldn't load entities: " + data.error;
            return;
          }
          renderEntitiesList(data.meta.entities, data.meta.queries);
        })
        .catch(function (e) {
          slot.textContent = "pgapp: " + e;
        });
    }

    function openEditor(title, existing) {
      Promise.all([
        fetch(pgappAdminAppUrl(target, "/entities-list")).then(function (r) { return r.json(); }),
        fetch(pgappAdminAppUrl(target, "/schema-metadata")).then(function (r) { return r.json(); }),
      ]).then(function (results) {
          var data = results[0];
          var schema = results[1];
          var queries = data.ok ? data.meta.queries : [];
          var columns = [];
          if (existing && existing.name && schema.ok && schema.entities[existing.name]) {
            columns = schema.entities[existing.name].columns.map(function (c) { return c.name; });
          }
          pgappGenericEditor(title, function (body) {
            return renderEntityForm(body, existing || {}, queries, columns);
          }).then(function (generated) {
            if (generated === null) return;
            var isNew = !existing || !existing.name;
            var url = isNew
              ? pgappAdminAppUrl(target, "/entities/add")
              : pgappAdminAppUrl(target, "/entities/" + encodeURIComponent(existing.name) + "/edit");
            fetch(url, {
              method: "POST",
              headers: { "Content-Type": "application/x-www-form-urlencoded" },
              body: "source=" + encodeURIComponent(generated),
            })
              .then(function (r) { return r.json(); })
              .then(function (d) {
                if (d.ok) location.reload();
                else pgappAlert("Couldn't save entity: " + d.error);
              })
              .catch(function (e) { pgappAlert("pgapp: " + e); });
          });
        });
    }

    function renderEntitiesList(entities, queries) {
      slot.textContent = "";
      slot.classList.add("pgapp-panel-card");
      var title = document.createElement("div");
      title.className = "pgapp-panel-card-title";
      title.textContent = "Data Model";
      slot.appendChild(title);

      entities.forEach(function (e) {
        var row = document.createElement("div");
        row.className = "pgapp-builder-two-col";
        var label = document.createElement("div");
        var sourceNote = e.source_query ? " (from query " + e.source_query + ")" : e.source_collection ? " (from collection)" : "";
        label.textContent = e.name + sourceNote + " — " + e.fields.length + " field(s)";
        row.appendChild(label);

        var btns = document.createElement("div");
        var editBtn = document.createElement("button");
        editBtn.type = "button";
        editBtn.className = "pgapp-icon-btn";
        editBtn.title = "Edit";
        editBtn.textContent = "✎";
        editBtn.addEventListener("click", function () {
          openEditor("Edit entity", e);
        });
        var delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
        delBtn.title = "Delete";
        delBtn.textContent = "✕";
        delBtn.addEventListener("click", function () {
          pgappConfirm('Delete entity "' + e.name + '"? Its physical table (if any) is left in place.').then(function (ok) {
            if (!ok) return;
            fetch(pgappAdminAppUrl(target, "/entities/" + encodeURIComponent(e.name) + "/delete"), { method: "POST" })
              .then(function (r) { return r.json(); })
              .then(function (d) {
                if (d.ok) location.reload();
                else pgappAlert("Couldn't delete entity: " + d.error);
              })
              .catch(function (err) { pgappAlert("pgapp: " + err); });
          });
        });
        btns.appendChild(editBtn);
        btns.appendChild(delBtn);
        row.appendChild(btns);
        slot.appendChild(row);
      });

      var addBtn = document.createElement("button");
      addBtn.type = "button";
      addBtn.className = "pgapp-btn pgapp-btn-secondary";
      addBtn.textContent = "+ Add Entity";
      addBtn.addEventListener("click", function () {
        openEditor("Add entity", null);
      });
      slot.appendChild(addBtn);
    }

    render();
  }

  // The "Queries" panel — a `text ... attrs (id: "pgapp-queries-slot")`
  // placeholder.
  function bindQueriesPanel() {
    var slot = document.getElementById("pgapp-queries-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    pgappEnsureBuilderStyle();

    // Filled in once from `/admin/schema-metadata` before the panel first
    // renders — every physical entity's real table/column names, shown as
    // a quick reference so an author doesn't have to leave the editor to
    // remember what a `from` clause can bind against.
    var schemaEntities = {};

    function renderForm(container, data) {
      var nameInput = pgappTextInput(data.name);
      if (data.name) nameInput.disabled = true;
      pgappFieldRow(container, "Name", nameInput);
      var sqlArea = pgappTextArea(data.sql, 5);
      pgappFieldRow(container, "SQL", sqlArea);

      var entityNames = Object.keys(schemaEntities).sort();
      if (entityNames.length) {
        pgappSectionTitle(container, "Available tables");
        var ref = document.createElement("div");
        ref.className = "pgapp-query-schema-ref";
        entityNames.forEach(function (name) {
          var e = schemaEntities[name];
          var line = document.createElement("div");
          var colNames = e.columns.map(function (c) { return c.name; }).join(", ");
          line.textContent = e.table_name + " (" + name + "): " + colNames;
          ref.appendChild(line);
        });
        container.appendChild(ref);
      }

      var testRow = document.createElement("div");
      testRow.className = "pgapp-field";
      var testBtn = document.createElement("button");
      testBtn.type = "button";
      testBtn.className = "pgapp-btn pgapp-btn-secondary";
      testBtn.textContent = "Test Query";
      var testResult = document.createElement("span");
      testResult.className = "pgapp-query-test-result";
      testBtn.addEventListener("click", function () {
        testResult.className = "pgapp-query-test-result";
        testResult.textContent = "Testing…";
        fetch(pgappAdminAppUrl(target, "/queries/test"), {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: "sql=" + encodeURIComponent(sqlArea.value),
        })
          .then(function (r) { return r.json(); })
          .then(function (d) {
            if (d.ok) {
              testResult.className = "pgapp-query-test-result pgapp-query-test-ok";
              testResult.textContent = d.binds.length ? "Valid SQL — binds: " + d.binds.join(", ") : "Valid SQL — no binds";
            } else {
              testResult.className = "pgapp-query-test-result pgapp-query-test-error";
              testResult.textContent = d.error;
            }
          })
          .catch(function (e) {
            testResult.className = "pgapp-query-test-result pgapp-query-test-error";
            testResult.textContent = "pgapp: " + e;
          });
      });
      testRow.appendChild(testBtn);
      testRow.appendChild(testResult);
      container.appendChild(testRow);

      return {
        generate: function () {
          var name = nameInput.value.trim();
          if (!name) throw "a query needs a name";
          return "  query " + name + " {\n    sql: " + pgappMarkupStr(sqlArea.value) + "\n  }";
        },
      };
    }

    function openEditor(title, existing) {
      pgappGenericEditor(title, function (body) {
        return renderForm(body, existing || {});
      }).then(function (generated) {
        if (generated === null) return;
        var isNew = !existing || !existing.name;
        var url = isNew
          ? pgappAdminAppUrl(target, "/queries/add")
          : pgappAdminAppUrl(target, "/queries/" + encodeURIComponent(existing.name) + "/edit");
        fetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: "source=" + encodeURIComponent(generated),
        })
          .then(function (r) { return r.json(); })
          .then(function (d) {
            if (d.ok) location.reload();
            else pgappAlert("Couldn't save query: " + d.error);
          })
          .catch(function (e) { pgappAlert("pgapp: " + e); });
      });
    }

    Promise.all([
      fetch(pgappAdminAppUrl(target, "/queries-list")).then(function (r) { return r.json(); }),
      fetch(pgappAdminAppUrl(target, "/schema-metadata")).then(function (r) { return r.json(); }),
    ])
      .then(function (results) {
        var data = results[0];
        var schema = results[1];
        if (schema.ok) schemaEntities = schema.entities;
        if (!data.ok) {
          slot.textContent = "Couldn't load queries: " + data.error;
          return;
        }
        slot.textContent = "";
        slot.classList.add("pgapp-panel-card");
        var title = document.createElement("div");
        title.className = "pgapp-panel-card-title";
        title.textContent = "Queries";
        slot.appendChild(title);

        data.meta.queries.forEach(function (q) {
          var row = document.createElement("div");
          row.className = "pgapp-builder-two-col";
          var label = document.createElement("div");
          label.textContent = q.name;
          row.appendChild(label);

          var btns = document.createElement("div");
          var editBtn = document.createElement("button");
          editBtn.type = "button";
          editBtn.className = "pgapp-icon-btn";
          editBtn.title = "Edit";
          editBtn.textContent = "✎";
          editBtn.addEventListener("click", function () {
            openEditor("Edit query", q);
          });
          var delBtn = document.createElement("button");
          delBtn.type = "button";
          delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
          delBtn.title = "Delete";
          delBtn.textContent = "✕";
          delBtn.addEventListener("click", function () {
            pgappConfirm('Delete query "' + q.name + '"?').then(function (ok) {
              if (!ok) return;
              fetch(pgappAdminAppUrl(target, "/queries/" + encodeURIComponent(q.name) + "/delete"), { method: "POST" })
                .then(function (r) { return r.json(); })
                .then(function (d) {
                  if (d.ok) location.reload();
                  else pgappAlert("Couldn't delete query: " + d.error);
                })
                .catch(function (err) { pgappAlert("pgapp: " + err); });
            });
          });
          btns.appendChild(editBtn);
          btns.appendChild(delBtn);
          row.appendChild(btns);
          slot.appendChild(row);
        });

        var addBtn = document.createElement("button");
        addBtn.type = "button";
        addBtn.className = "pgapp-btn pgapp-btn-secondary";
        addBtn.textContent = "+ Add Query";
        addBtn.addEventListener("click", function () {
          openEditor("Add query", null);
        });
        slot.appendChild(addBtn);
      })
      .catch(function (e) {
        slot.textContent = "pgapp: " + e;
      });
  }

  // The "Navigation" panel — a `text ... attrs (id:
  // "pgapp-nav-slot")` placeholder. Reordering uses plain ▲▼ buttons
  // (not drag-and-drop) — nav lists are short enough that this is
  // simpler to get right than wiring up a second drag surface, same
  // convention `pgappRowList` already uses everywhere else.
  function bindNavPanel() {
    var slot = document.getElementById("pgapp-nav-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    pgappEnsureBuilderStyle();

    function renderForm(container, data, pages) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var pageSel = pgappSelect(pages || [], data.target_page);
      pgappFieldRow(container, "Target page", pageSel);
      return {
        generate: function () {
          var label = labelInput.value.trim();
          if (!label) throw "a nav item needs a label";
          if (!pageSel.value) throw "a nav item needs a target page";
          return "    item " + pgappMarkupStr(label) + " -> page " + pageSel.value;
        },
      };
    }

    function reorder(items, fromIdx, toIdx) {
      var order = items.map(function (_, i) { return i; });
      var moved = order.splice(fromIdx, 1)[0];
      order.splice(toIdx, 0, moved);
      fetch(pgappAdminAppUrl(target, "/nav/reorder"), {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: "order=" + encodeURIComponent(order.join(",")),
      })
        .then(function (r) { return r.json(); })
        .then(function (d) {
          if (d.ok) location.reload();
          else pgappAlert("Couldn't reorder: " + d.error);
        })
        .catch(function (e) { pgappAlert("pgapp: " + e); });
    }

    function openEditor(title, idx, existing, pages) {
      pgappGenericEditor(title, function (body) {
        return renderForm(body, existing || {}, pages);
      }).then(function (generated) {
        if (generated === null) return;
        var isNew = idx === null;
        var url = isNew ? pgappAdminAppUrl(target, "/nav/add") : pgappAdminAppUrl(target, "/nav/" + idx + "/edit");
        fetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: "source=" + encodeURIComponent(generated),
        })
          .then(function (r) { return r.json(); })
          .then(function (d) {
            if (d.ok) location.reload();
            else pgappAlert("Couldn't save nav item: " + d.error);
          })
          .catch(function (e) { pgappAlert("pgapp: " + e); });
      });
    }

    fetch(pgappAdminAppUrl(target, "/nav-list"))
      .then(function (r) { return r.json(); })
      .then(function (data) {
        if (!data.ok) {
          slot.textContent = "Couldn't load navigation: " + data.error;
          return;
        }
        var items = data.meta.items;
        var pages = data.meta.pages;
        slot.textContent = "";
        slot.classList.add("pgapp-panel-card");
        var title = document.createElement("div");
        title.className = "pgapp-panel-card-title";
        title.textContent = "Navigation";
        slot.appendChild(title);

        items.forEach(function (item, idx) {
          var row = document.createElement("div");
          row.className = "pgapp-builder-two-col";
          var label = document.createElement("div");
          label.textContent = item.submenu
            ? item.label + " (submenu — edit as raw markup via Advanced)"
            : item.label + " → " + item.target_page;
          row.appendChild(label);

          var btns = document.createElement("div");
          if (idx > 0) {
            var upBtn = document.createElement("button");
            upBtn.type = "button";
            upBtn.className = "pgapp-icon-btn";
            upBtn.title = "Move up";
            upBtn.textContent = "▲";
            upBtn.addEventListener("click", function () { reorder(items, idx, idx - 1); });
            btns.appendChild(upBtn);
          }
          if (idx < items.length - 1) {
            var downBtn = document.createElement("button");
            downBtn.type = "button";
            downBtn.className = "pgapp-icon-btn";
            downBtn.title = "Move down";
            downBtn.textContent = "▼";
            downBtn.addEventListener("click", function () { reorder(items, idx, idx + 1); });
            btns.appendChild(downBtn);
          }
          if (!item.submenu) {
            var editBtn = document.createElement("button");
            editBtn.type = "button";
            editBtn.className = "pgapp-icon-btn";
            editBtn.title = "Edit";
            editBtn.textContent = "✎";
            editBtn.addEventListener("click", function () {
              openEditor("Edit nav item", idx, item, pages);
            });
            btns.appendChild(editBtn);
          }
          var delBtn = document.createElement("button");
          delBtn.type = "button";
          delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
          delBtn.title = "Delete";
          delBtn.textContent = "✕";
          delBtn.addEventListener("click", function () {
            pgappConfirm('Delete nav item "' + item.label + '"?').then(function (ok) {
              if (!ok) return;
              fetch(pgappAdminAppUrl(target, "/nav/" + idx + "/delete"), { method: "POST" })
                .then(function (r) { return r.json(); })
                .then(function (d) {
                  if (d.ok) location.reload();
                  else pgappAlert("Couldn't delete nav item: " + d.error);
                })
                .catch(function (err) { pgappAlert("pgapp: " + err); });
            });
          });
          btns.appendChild(delBtn);
          row.appendChild(btns);
          slot.appendChild(row);
        });

        var addBtn = document.createElement("button");
        addBtn.type = "button";
        addBtn.className = "pgapp-btn pgapp-btn-secondary";
        addBtn.textContent = "+ Add Nav Item";
        addBtn.addEventListener("click", function () {
          openEditor("Add nav item", null, null, pages);
        });
        slot.appendChild(addBtn);
      })
      .catch(function (e) {
        slot.textContent = "pgapp: " + e;
      });
  }

  // The "App Settings" panel — a `text ... attrs (id:
  // "pgapp-app-settings-slot")` placeholder: theme/icons/chart_lib
  // selects plus the auth on/off toggle (see `admin_settings_set`'s own
  // doc for why that's the deliberate scope boundary — auth *schemes*
  // stay Advanced-editor-only).
  function bindAppSettingsForm() {
    var slot = document.getElementById("pgapp-app-settings-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;

    fetch(pgappAdminAppUrl(target, "/settings"))
      .then(function (r) { return r.json(); })
      .then(function (data) {
        if (!data.ok) {
          slot.textContent = "Couldn't load settings: " + data.error;
          return;
        }
        var s = data.meta;
        slot.textContent = "";
        slot.classList.add("pgapp-panel-card");
        var title = document.createElement("div");
        title.className = "pgapp-panel-card-title";
        title.textContent = "App Settings";
        slot.appendChild(title);

        var themeSel = pgappSelect(["plain", "shadcn", "vivid", "google_m3", "apex_universal"], s.theme);
        pgappFieldRow(slot, "Theme", themeSel);
        var iconsSel = pgappSelect(["builtin"], s.icons);
        pgappFieldRow(slot, "Icons", iconsSel);
        var chartSel = pgappSelect(["inline"], s.chart_lib);
        pgappFieldRow(slot, "Chart library", chartSel);

        var authRow = document.createElement("div");
        authRow.className = "pgapp-field";
        var authLabel = document.createElement("label");
        authLabel.className = "pgapp-label";
        var authCheck = document.createElement("input");
        authCheck.type = "checkbox";
        authCheck.checked = !!s.auth_enabled;
        authLabel.appendChild(authCheck);
        authLabel.appendChild(document.createTextNode(" Require sign-in (auth enabled)"));
        authRow.appendChild(authLabel);
        slot.appendChild(authRow);

        var saveBtn = document.createElement("button");
        saveBtn.type = "button";
        saveBtn.className = "pgapp-btn pgapp-btn-primary";
        saveBtn.textContent = "Save Settings";
        saveBtn.addEventListener("click", function () {
          var body =
            "theme=" + encodeURIComponent(themeSel.value) +
            "&icons=" + encodeURIComponent(iconsSel.value) +
            "&chart_lib=" + encodeURIComponent(chartSel.value) +
            "&auth_enabled=" + (authCheck.checked ? "true" : "false");
          fetch(pgappAdminAppUrl(target, "/settings"), {
            method: "POST",
            headers: { "Content-Type": "application/x-www-form-urlencoded" },
            body: body,
          })
            .then(function (r) { return r.json(); })
            .then(function (d) {
              if (d.ok) location.reload();
              else pgappAlert("Couldn't save settings: " + d.error);
            })
            .catch(function (e) { pgappAlert("pgapp: " + e); });
        });
        slot.appendChild(saveBtn);
      })
      .catch(function (e) {
        slot.textContent = "pgapp: " + e;
      });
  }

  // The "Secrets" panel — a `text ... attrs (id: "pgapp-secrets-slot")`
  // placeholder: add/list/remove app-scoped secrets, the same
  // encrypted-at-rest store `pgapp secret set/list/rm` uses (see
  // src/secrets.rs) — `{{secret.name}}` in an `http_request` action
  // resolves against these. Only names ever come back from the server;
  // a value, once saved, can be overwritten but never displayed again.
  function bindSecretsPanel() {
    var slot = document.getElementById("pgapp-secrets-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    pgappEnsureBuilderStyle();

    function render() {
      fetch(pgappAdminAppUrl(target, "/secrets-list"))
        .then(function (r) { return r.json(); })
        .then(function (data) {
          if (!data.ok) {
            slot.textContent = "Couldn't load secrets: " + data.error;
            return;
          }
          slot.textContent = "";
          slot.classList.add("pgapp-panel-card");
          var title = document.createElement("div");
          title.className = "pgapp-panel-card-title";
          title.textContent = "Secrets";
          slot.appendChild(title);

          data.names.forEach(function (name) {
            var row = document.createElement("div");
            row.className = "pgapp-builder-two-col";
            var label = document.createElement("div");
            label.textContent = name;
            row.appendChild(label);

            var delBtn = document.createElement("button");
            delBtn.type = "button";
            delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
            delBtn.title = "Delete";
            delBtn.textContent = "✕";
            delBtn.addEventListener("click", function () {
              pgappConfirm(
                'Delete secret "' + name + '"? Anything using {{secret.' + name + "}} will fail until it's replaced."
              ).then(function (ok) {
                if (!ok) return;
                fetch(pgappAdminAppUrl(target, "/secrets/" + encodeURIComponent(name) + "/delete"), { method: "POST" })
                  .then(function (r) { return r.json(); })
                  .then(function (d) {
                    if (d.ok) render();
                    else pgappAlert("Couldn't delete secret: " + d.error);
                  })
                  .catch(function (e) { pgappAlert("pgapp: " + e); });
              });
            });
            row.appendChild(delBtn);
            slot.appendChild(row);
          });

          var addRow = document.createElement("div");
          addRow.className = "pgapp-builder-two-col";
          var nameInput = pgappTextInput("");
          nameInput.placeholder = "name";
          var valueInput = document.createElement("input");
          valueInput.type = "password";
          valueInput.className = "pgapp-input";
          valueInput.placeholder = "value";
          addRow.appendChild(nameInput);
          addRow.appendChild(valueInput);
          slot.appendChild(addRow);

          var addBtn = document.createElement("button");
          addBtn.type = "button";
          addBtn.className = "pgapp-btn pgapp-btn-secondary";
          addBtn.textContent = "+ Add / Update Secret";
          addBtn.addEventListener("click", function () {
            var name = nameInput.value.trim();
            if (!name) {
              pgappAlert("A secret needs a name.");
              return;
            }
            fetch(pgappAdminAppUrl(target, "/secrets/set"), {
              method: "POST",
              headers: { "Content-Type": "application/x-www-form-urlencoded" },
              body: "name=" + encodeURIComponent(name) + "&value=" + encodeURIComponent(valueInput.value),
            })
              .then(function (r) { return r.json(); })
              .then(function (d) {
                if (d.ok) render();
                else pgappAlert("Couldn't save secret: " + d.error);
              })
              .catch(function (e) { pgappAlert("pgapp: " + e); });
          });
          slot.appendChild(addBtn);
        })
        .catch(function (e) {
          slot.textContent = "pgapp: " + e;
        });
    }

    render();
  }

  // The "Delete App" panel — a `text ... attrs (id:
  // "pgapp-destroy-app-slot")` placeholder: soft (disable) needs only a
  // confirm; hard (permanently drop its data tables) also needs the
  // app's own slug typed in, mirroring `pgapp app destroy --hard`'s own
  // confirmation.
  function bindDestroyAppPanel() {
    var slot = document.getElementById("pgapp-destroy-app-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;

    slot.textContent = "";
    slot.classList.add("pgapp-panel-card");
    var title = document.createElement("div");
    title.className = "pgapp-panel-card-title";
    title.textContent = "Delete This App";
    slot.appendChild(title);

    function post(mode, confirm) {
      fetch(pgappAdminAppUrl(target, "/destroy"), {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: "mode=" + mode + "&confirm=" + encodeURIComponent(confirm || ""),
      })
        .then(function (r) { return r.json(); })
        .then(function (d) {
          if (d.ok) {
            pgappAlert("App " + (mode === "hard" ? "permanently deleted." : "disabled.")).then(function () {
              location.href = "/pgapp/builder/Apps";
            });
          } else {
            pgappAlert("Couldn't delete app: " + d.error);
          }
        })
        .catch(function (e) { pgappAlert("pgapp: " + e); });
    }

    var disableBtn = document.createElement("button");
    disableBtn.type = "button";
    disableBtn.className = "pgapp-btn pgapp-btn-secondary";
    disableBtn.textContent = "Disable (soft)";
    disableBtn.addEventListener("click", function () {
      pgappConfirm("Disable this app? Its data is untouched; re-registering it later reactivates it.").then(function (ok) {
        if (ok) post("soft", "");
      });
    });
    slot.appendChild(disableBtn);

    var deleteBtn = document.createElement("button");
    deleteBtn.type = "button";
    deleteBtn.className = "pgapp-btn pgapp-btn-destructive";
    deleteBtn.textContent = "Delete permanently (hard)";
    deleteBtn.addEventListener("click", function () {
      pgappPrompt('Type "' + target.app + '" to permanently delete this app and its data tables:', "").then(function (typed) {
        if (typed === null) return;
        if (typed !== target.app) {
          pgappAlert("That didn't match — nothing was deleted.");
          return;
        }
        post("hard", typed);
      });
    });
    slot.appendChild(deleteBtn);
  }

  // The "Delete Workspace" panel — a `text ... attrs (id:
  // "pgapp-destroy-workspace-slot")` placeholder. Hard delete also
  // needs a superuser-capable connection string (used once to drop the
  // schema/role, never stored — same contract as "New Workspace"'s
  // existing-schema attach).
  function bindDestroyWorkspacePanel() {
    var slot = document.getElementById("pgapp-destroy-workspace-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;

    slot.textContent = "";
    slot.classList.add("pgapp-panel-card");
    var title = document.createElement("div");
    title.className = "pgapp-panel-card-title";
    title.textContent = "Delete Workspace \"" + target.workspace + "\"";
    slot.appendChild(title);

    var url = pgappAdminAppUrl(target, "/destroy-workspace");

    function post(body) {
      fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: body,
      })
        .then(function (r) { return r.json(); })
        .then(function (d) {
          if (d.ok) {
            pgappAlert("Workspace " + (d.hard ? "permanently deleted." : "disabled.")).then(function () {
              location.href = "/pgapp/builder/Apps";
            });
          } else {
            pgappAlert("Couldn't delete workspace: " + d.error);
          }
        })
        .catch(function (e) { pgappAlert("pgapp: " + e); });
    }

    var disableBtn = document.createElement("button");
    disableBtn.type = "button";
    disableBtn.className = "pgapp-btn pgapp-btn-secondary";
    disableBtn.textContent = "Disable (soft)";
    disableBtn.addEventListener("click", function () {
      pgappConfirm("Disable this whole workspace? Its schema and data are untouched.").then(function (ok) {
        if (ok) post("mode=soft");
      });
    });
    slot.appendChild(disableBtn);

    var deleteBtn = document.createElement("button");
    deleteBtn.type = "button";
    deleteBtn.className = "pgapp-btn pgapp-btn-destructive";
    deleteBtn.textContent = "Delete permanently (hard)";
    deleteBtn.addEventListener("click", function () {
      pgappPrompt('Type "' + target.workspace + '" to permanently destroy this workspace:', "").then(function (typed) {
        if (typed === null) return;
        if (typed !== target.workspace) {
          pgappAlert("That didn't match — nothing was deleted.");
          return;
        }
        pgappPrompt("Superuser-capable Postgres connection string (used once to drop the schema/role, never stored):", "").then(function (conn) {
          if (conn === null || !conn.trim()) return;
          post("mode=hard&confirm=" + encodeURIComponent(typed) + "&grantor_conn=" + encodeURIComponent(conn.trim()));
        });
      });
    });
    slot.appendChild(deleteBtn);
  }

  // The App Builder's "New App" processing: on every load of the
  // NewApp page (identified by the `pgapp-new-app-requests` id on its
  // history report — see examples/app_builder.pgapp), asks the server
  // to process the oldest pending request, if any (a harmless no-op
  // otherwise — see `admin_create_pending_app` in server.rs). Reloads
  // on success so the row's updated status/result show immediately;
  // this is what gives Form's own create-and-redirect (which already
  // lands back on this same page) its "submit and see it done" feel,
  // in place of a `before_load` action (which can't reach `AppState`
  // to hot-register the new app — see actions/create_app.rs's doc).
  function bindNewAppProcessing() {
    if (!document.getElementById("pgapp-new-app-requests")) return;
    fetch("/pgapp/builder/admin/apps/create-pending", { method: "POST" })
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        if (data.ok && data.processed) location.reload();
      })
      .catch(function () {});
  }

  // Starter markup text per component kind — seeds the "Add Component"
  // panel's raw textarea (see bindAddComponentForm) with a valid, if
  // generic, block the user then edits in place: fill in the real
  // entity/query name, tweak columns, add item overrides, whatever the
  // kind supports — since the textarea submits verbatim, every
  // attribute the grammar has is reachable here, not just a fixed
  // subset. `<>`-bracketed tokens are placeholders to replace, not
  // valid syntax on their own.
  var COMPONENT_TEMPLATES = {
    text: '    text "New text"',
    report: '    report "New Report" of <entity_name> {\n      columns: <col1>, <col2>\n    }',
    region: '    region "New Region" from query <query_name> {\n      columns: <col1>, <col2>\n    }',
    editable_table: '    editable_table "New Table" of <entity_name> {\n      columns: <col1>, <col2>\n    }',
    form: '    form "New Form" of <entity_name> {\n      fields: <field1>, <field2>\n    }',
    chart: '    chart "New Chart" from query <query_name> {\n      type: bar\n      x: <label_column>\n      y: <value_column>\n    }',
    action: '    action "Run action" calls <action_name>',
    link: '    link "Go" -> page <PageName>',
    button: '    button "Go" -> page <PageName>',
    dynamic_action: '    on change of <item> {\n      show <other_item>\n    }',
    dynamic_content: '    dynamic_content "New Dynamic Content" calls <action_name>',
    calendar: '    calendar "New Calendar" of <entity_name> {\n      date: <date_field>\n      title: <title_field>\n    }',
    map: '    map "New Map" of <entity_name> {\n      lat: <lat_field>\n      lng: <lng_field>\n      title: <title_field>\n    }',
    faceted_search: '    faceted_search "New Faceted Search" of <entity_name> {\n      facet <column> as checkbox_list\n    }',
  };

  // The App Builder's "Add Component" panel: a `text ... attrs (id:
  // "pgapp-add-component-slot")` placeholder gets a kind picker plus a
  // raw markup textarea appended into it. Picking a kind just seeds the
  // textarea with a starter template (COMPONENT_TEMPLATES) — the
  // textarea's own content, not the kind picker, is what's actually
  // submitted, so any of the 9 component kinds and any of their
  // attributes can be added, not a fixed structured-fields subset. If
  // the textarea's text targets a page (a `link` component, or a
  // report's `link:` property), `renderLinkControls` also renders a
  // proper "Target page" dropdown (+ parameter rows for a report link)
  // above it — a real GUI control for the one property that's
  // otherwise easy to typo, re-rendered whenever the kind changes.
  // POSTs the raw text to the target app's own
  // `/admin/pages/:page/components/add`.
  function submitNewComponent(target, sourceText) {
    fetch(pgappAdminPagesUrl(target, "/components/add"), {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: "source=" + encodeURIComponent(sourceText),
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        if (data.ok) location.reload();
        else pgappAlert("Couldn't add component: " + data.error);
      })
      .catch(function (e) {
        pgappAlert("pgapp: " + e);
      });
  }

  // The App Builder's "Add Component" panel: a kind picker plus an
  // "Add..." button that opens the same structured, per-attribute
  // editor the docked Property Editor uses to edit an existing one
  // (see `pgappStructuredEditor`/`PGAPP_KIND_RENDERERS`, shared by
  // both), just prefilled blank instead of from an existing
  // component's data — a modal here since there's no selected tree row
  // yet to dock a panel next to. A secondary "Add as raw markup" link
  // reveals the original kind-picker + raw-textarea flow (still backed
  // by `COMPONENT_TEMPLATES`) as a fallback/escape hatch for anything
  // the structured form doesn't cover well.
  function bindAddComponentForm() {
    var slot = document.getElementById("pgapp-add-component-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid(target)) return;
    slot.textContent = "";
    slot.classList.add("pgapp-panel-card");

    var title = document.createElement("div");
    title.className = "pgapp-panel-card-title";
    title.textContent = "Add Component";
    slot.appendChild(title);

    var kindSel = document.createElement("select");
    kindSel.className = "pgapp-select";
    // The structured picker's own kind list (every kind with a real
    // `PGAPP_KIND_RENDERERS` entry) — a superset of `COMPONENT_TEMPLATES`,
    // since `dynamic_action` has a structured editor but was never one of
    // the original raw-textarea starter templates.
    Object.keys(PGAPP_KIND_RENDERERS).forEach(function (k) {
      var opt = document.createElement("option");
      opt.value = k;
      opt.textContent = k;
      kindSel.appendChild(opt);
    });
    slot.appendChild(kindSel);

    var addBtn = document.createElement("button");
    addBtn.type = "button";
    addBtn.className = "pgapp-btn pgapp-btn-primary";
    addBtn.textContent = "Add…";
    addBtn.addEventListener("click", function () {
      fetchAppMeta(target).then(function (meta) {
        pgappStructuredEditor("Add component (" + kindSel.value + ")", kindSel.value, {}, meta).then(function (generated) {
          if (generated === null) return;
          submitNewComponent(target, generated);
        });
      });
    });
    slot.appendChild(addBtn);

    var rawToggle = document.createElement("a");
    rawToggle.href = "#";
    rawToggle.className = "pgapp-link";
    rawToggle.textContent = "Add as raw markup";
    rawToggle.style.display = "block";
    rawToggle.style.marginTop = "0.5rem";
    slot.appendChild(rawToggle);

    var rawForm = document.createElement("form");
    rawForm.className = "pgapp-add-component-form";
    rawForm.style.display = "none";

    var rawKindSel = document.createElement("select");
    rawKindSel.className = "pgapp-select";
    Object.keys(COMPONENT_TEMPLATES).forEach(function (k) {
      var opt = document.createElement("option");
      opt.value = k;
      opt.textContent = k;
      rawKindSel.appendChild(opt);
    });

    var sourceArea = document.createElement("textarea");
    sourceArea.className = "pgapp-input pgapp-source-textarea";
    sourceArea.rows = 4;
    sourceArea.value = COMPONENT_TEMPLATES[rawKindSel.value];

    var pagesListCache = [];
    fetchPagesList(target).then(function (pages) {
      pagesListCache = pages;
      renderLinkControls(rawForm, sourceArea, pagesListCache);
    });

    rawKindSel.addEventListener("change", function () {
      sourceArea.value = COMPONENT_TEMPLATES[rawKindSel.value];
      renderLinkControls(rawForm, sourceArea, pagesListCache);
    });

    var rawAddBtn = document.createElement("button");
    rawAddBtn.type = "submit";
    rawAddBtn.className = "pgapp-btn pgapp-btn-primary";
    rawAddBtn.textContent = "Add";

    [rawKindSel, sourceArea, rawAddBtn].forEach(function (el) {
      rawForm.appendChild(el);
    });
    slot.appendChild(rawForm);

    rawToggle.addEventListener("click", function (ev) {
      ev.preventDefault();
      rawForm.style.display = rawForm.style.display === "none" ? "" : "none";
    });

    rawForm.addEventListener("submit", function (ev) {
      ev.preventDefault();
      submitNewComponent(target, sourceArea.value);
    });
  }

  // One glyph per component kind, purely decorative (an APEX-Page-
  // Designer-style visual cue in the component list) — falls back to a
  // generic dot for anything not listed (DynamicAction has no wrapper
  // and never shows up here; every other kind is covered).
  var COMPONENT_KIND_ICONS = {
    report: "▤",
    form: "✎",
    editable_table: "▦",
    chart: "📈",
    text: "¶",
    link: "↗",
    region: "▥",
    action: "⚡",
  };

  // ------------------------------------------------------------------
  // Structured component editor — an APEX-Page-Designer-style property
  // sheet: pick a component, get real typed fields for every attribute
  // it supports (including its own nested lists — a Report's computed
  // columns/format masks, a Form's per-field item types, a dynamic
  // action's ops, ...) instead of a raw markup textarea. Prefilled from
  // `RuntimeComponent::to_json` (see `admin_component_structured` in
  // server.rs) and `admin_app_meta`'s entity/query/action/item-type/
  // page lists (for dropdowns). On Save, walks the form and *generates*
  // fresh markup text client-side (mirroring the grammar `markup.rs`
  // parses — see each `pgappGenerate*` below) and submits that through
  // the exact same raw-text `/components/.../add`|`edit` routes the
  // original raw editor already used, so no new write-side route was
  // needed: this only ever *generates* markup, never *parses* it back
  // (the server already validates whatever comes out via
  // `validate_markup` before writing, same as the raw editor).
  //
  // Every row-list (`pgappRowList`) — used for anything the grammar
  // itself repeats zero-or-more times — is entirely client-side DOM
  // bookkeeping: add/remove/reorder never round-trips to the server,
  // unlike the page-level component reorder feature (`bindDraggableRows`)
  // which persists on every drop. Only this dialog's own Save button
  // ever writes anything.

  // Escapes a string for a double-quoted markup string literal —
  // mirrors `page_reorder::escape_string` exactly (`\` and `"` doubled,
  // nothing else).
  function pgappMarkupStr(s) {
    return '"' + String(s == null ? "" : s).replace(/\\/g, "\\\\").replace(/"/g, '\\"') + '"';
  }

  // A one-time stylesheet for the structured editor's own layout hooks
  // — injected lazily (once) rather than added to every theme's
  // theme.css, since this is App-Builder-only chrome, not part of the
  // served app's own themed UI.
  function pgappEnsureBuilderStyle() {
    if (document.getElementById("pgapp-builder-style")) return;
    var style = document.createElement("style");
    style.id = "pgapp-builder-style";
    style.textContent =
      ".pgapp-builder-form-box { max-width: 40rem; } " +
      ".pgapp-builder-form-body { max-height: 60vh; overflow-y: auto; text-align: left; } " +
      ".pgapp-builder-section-title { font-weight: 700; margin: 1rem 0 0.35rem; } " +
      ".pgapp-builder-rowlist { margin-bottom: 0.5rem; } " +
      ".pgapp-builder-rowlist table { width: 100%; margin-bottom: 0.4rem; } " +
      ".pgapp-builder-row-actions { white-space: nowrap; } " +
      ".pgapp-builder-attrs { margin-bottom: 0.5rem; } " +
      ".pgapp-builder-two-col { display: flex; gap: 0.75rem; align-items: center; justify-content: space-between; margin-bottom: 0.4rem; } " +
      ".pgapp-builder-two-col > div { flex: 1; }" +
      ".pgapp-danger-zone { border: 1px solid #b91c1c; border-radius: 0.375rem; padding: 0.75rem; margin-top: 1rem; }" +
      // APEX Page-Designer-style tree + Property Editor split (EditPage) —
      // stacks on narrow screens since a fixed 2-column split doesn't fit
      // a phone-width viewport (same responsive philosophy as the rest of
      // pgapp's own default mobile handling).
      ".pgapp-designer-layout { display: flex; gap: 1.25rem; align-items: flex-start; flex-wrap: wrap; } " +
      ".pgapp-designer-layout > *:first-child { flex: 1 1 22rem; min-width: 0; } " +
      ".pgapp-designer-layout > *:last-child { flex: 1 1 22rem; min-width: 0; position: sticky; top: 1rem; } " +
      ".pgapp-designer-row { cursor: pointer; } " +
      ".pgapp-designer-row.pgapp-selected { background: rgba(99, 102, 241, 0.12); box-shadow: inset 3px 0 0 0 #6366f1; } " +
      ".pgapp-designer-properties { border: 1px solid rgba(120, 120, 120, 0.3); border-radius: 0.5rem; padding: 1rem; min-height: 10rem; } " +
      ".pgapp-designer-properties-empty { color: #888; font-style: italic; margin: 0; }" +
      ".pgapp-query-schema-ref { max-height: 8rem; overflow-y: auto; font-family: monospace; font-size: 0.85em; border: 1px solid rgba(120, 120, 120, 0.3); border-radius: 0.375rem; padding: 0.5rem; margin-bottom: 0.5rem; }" +
      ".pgapp-query-test-result { margin-left: 0.75rem; }" +
      ".pgapp-query-test-ok { color: #15803d; }" +
      ".pgapp-query-test-error { color: #b91c1c; }";
    document.head.appendChild(style);
  }

  function pgappFieldRow(container, labelText, inputEl) {
    var row = document.createElement("div");
    row.className = "pgapp-field";
    var label = document.createElement("label");
    label.className = "pgapp-label";
    label.textContent = labelText;
    row.appendChild(label);
    row.appendChild(inputEl);
    container.appendChild(row);
    return inputEl;
  }

  function pgappTextInput(value) {
    var el = document.createElement("input");
    el.type = "text";
    el.className = "pgapp-input";
    el.value = value == null ? "" : value;
    return el;
  }

  function pgappNumberInput(value) {
    var el = document.createElement("input");
    el.type = "number";
    el.className = "pgapp-input";
    el.value = value == null ? "" : value;
    return el;
  }

  function pgappTextArea(value, rows) {
    var el = document.createElement("textarea");
    el.className = "pgapp-input";
    el.rows = rows || 3;
    el.value = value == null ? "" : value;
    return el;
  }

  // `options` empty falls back to a plain text input — e.g. a fresh app
  // with no entities/queries/pages yet shouldn't leave the editor
  // completely unusable, just less guided.
  function pgappSelect(options, value) {
    if (!options || options.length === 0) return pgappTextInput(value);
    var el = document.createElement("select");
    el.className = "pgapp-select";
    options.forEach(function (opt) {
      var o = document.createElement("option");
      o.value = opt;
      o.textContent = opt;
      el.appendChild(o);
    });
    if (value != null && options.indexOf(value) !== -1) el.value = value;
    return el;
  }

  function pgappSectionTitle(container, text) {
    var h = document.createElement("div");
    h.className = "pgapp-builder-section-title";
    h.textContent = text;
    container.appendChild(h);
  }

  // A minimal, dependency-free repeatable-row editor: given `cols` (an
  // array of `{key, label, type: "text"|"select"|"textarea", options}`)
  // and `rows` (an initial array of `{<key>: <value>, ...}` objects),
  // renders an editable `<table>` with one `<tr>` per row plus Add/
  // Remove/reorder (▲▼) controls. `getRows()` reads the table's current
  // live values back out in display order, silently skipping any row
  // every column is still blank on (so a stray "Add row" click that's
  // never filled in doesn't turn into a bogus markup line). A `"datalist"`
  // column type renders a plain text input with `<datalist>` suggestions
  // (not a hard dropdown — the value doesn't have to be one of them, e.g.
  // an entity field that doesn't exist in the table yet).
  var pgappDatalistCounter = 0;
  function pgappRowList(cols, rows) {
    var wrap = document.createElement("div");
    wrap.className = "pgapp-builder-rowlist";
    var table = document.createElement("table");
    table.className = "pgapp-table";
    var thead = document.createElement("thead");
    var headRow = document.createElement("tr");
    cols.forEach(function (c) {
      var th = document.createElement("th");
      th.textContent = c.label;
      headRow.appendChild(th);
    });
    headRow.appendChild(document.createElement("th"));
    thead.appendChild(headRow);
    table.appendChild(thead);
    var tbody = document.createElement("tbody");
    table.appendChild(tbody);
    wrap.appendChild(table);

    function addRow(initial) {
      var tr = document.createElement("tr");
      cols.forEach(function (c) {
        var td = document.createElement("td");
        var field;
        if (c.type === "select") {
          field = pgappSelect(c.options, initial ? initial[c.key] : null);
        } else if (c.type === "textarea") {
          field = pgappTextArea(initial ? initial[c.key] : "", 2);
        } else if (c.type === "datalist" && c.options && c.options.length) {
          field = pgappTextInput(initial ? initial[c.key] : "");
          var listId = "pgapp-datalist-" + pgappDatalistCounter++;
          var dl = document.createElement("datalist");
          dl.id = listId;
          c.options.forEach(function (opt) {
            var o = document.createElement("option");
            o.value = opt;
            dl.appendChild(o);
          });
          field.setAttribute("list", listId);
          td.appendChild(dl);
        } else if (c.type === "checkbox") {
          field = document.createElement("input");
          field.type = "checkbox";
          field.checked = !!(initial && initial[c.key]);
        } else {
          field = pgappTextInput(initial ? initial[c.key] : "");
        }
        field.dataset.key = c.key;
        td.appendChild(field);
        tr.appendChild(td);
      });
      var actionsTd = document.createElement("td");
      actionsTd.className = "pgapp-builder-row-actions";
      var upBtn = document.createElement("button");
      upBtn.type = "button";
      upBtn.className = "pgapp-icon-btn";
      upBtn.title = "Move up";
      upBtn.textContent = "▲";
      upBtn.addEventListener("click", function () {
        var prev = tr.previousElementSibling;
        if (prev) tbody.insertBefore(tr, prev);
      });
      var downBtn = document.createElement("button");
      downBtn.type = "button";
      downBtn.className = "pgapp-icon-btn";
      downBtn.title = "Move down";
      downBtn.textContent = "▼";
      downBtn.addEventListener("click", function () {
        var next = tr.nextElementSibling;
        if (next) tbody.insertBefore(next, tr);
      });
      var delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
      delBtn.title = "Remove row";
      delBtn.textContent = "✕";
      delBtn.addEventListener("click", function () {
        tr.remove();
      });
      actionsTd.appendChild(upBtn);
      actionsTd.appendChild(downBtn);
      actionsTd.appendChild(delBtn);
      tr.appendChild(actionsTd);
      tbody.appendChild(tr);
    }

    (rows || []).forEach(function (r) {
      addRow(r);
    });

    var addBtn = document.createElement("button");
    addBtn.type = "button";
    addBtn.className = "pgapp-btn pgapp-btn-secondary";
    addBtn.textContent = "+ Add row";
    addBtn.addEventListener("click", function () {
      addRow(null);
    });
    wrap.appendChild(addBtn);

    function getRows() {
      var out = [];
      var trs = tbody.querySelectorAll("tr");
      for (var i = 0; i < trs.length; i++) {
        var row = {};
        var fields = trs[i].querySelectorAll("[data-key]");
        var allBlank = true;
        for (var j = 0; j < fields.length; j++) {
          if (fields[j].type === "checkbox") {
            row[fields[j].dataset.key] = fields[j].checked;
            if (fields[j].checked) allBlank = false;
          } else {
            var v = fields[j].value;
            row[fields[j].dataset.key] = v;
            if (v && v.trim() !== "") allBlank = false;
          }
        }
        if (!allBlank) out.push(row);
      }
      return out;
    }

    return { el: wrap, getRows: getRows };
  }

  // An ordered subset of an entity's own fields — what a Report's
  // `columns:`, a Form's `fields:`, and an EditableTable's `columns:`
  // all actually are. Modeled as a row list of one `<select>` column
  // (the field to include next), so add/remove/reorder all just work
  // via `pgappRowList`.
  function pgappFieldPickerList(entityFields, selected) {
    var options = (entityFields || []).map(function (f) {
      return f.name;
    });
    var initial = (selected || []).map(function (name) {
      return { field: name };
    });
    return pgappRowList([{ key: "field", label: "Field", type: "select", options: options }], initial);
  }

  function pgappFieldPickerText(rowList) {
    return rowList
      .getRows()
      .map(function (r) {
        return r.field;
      })
      .filter(function (f) {
        return f && f.trim() !== "";
      })
      .join(", ");
  }

  // The shared `attrs (id: "...", class: "...", <key>: "<value>", ...)`
  // suffix every component kind (and every form/editable_table field)
  // accepts — one widget, reused everywhere instead of rebuilt per kind.
  function pgappAttrsEditor(container, html) {
    html = html || { id: null, class: null, attrs: [] };
    pgappSectionTitle(container, "Attributes");
    var idInput = pgappTextInput(html.id);
    pgappFieldRow(container, "id", idInput);
    var classInput = pgappTextInput(html.class);
    pgappFieldRow(container, "class", classInput);
    var extraLabel = document.createElement("div");
    extraLabel.className = "pgapp-label";
    extraLabel.textContent = "Extra attributes (use _ for a hyphen, e.g. data_foo)";
    container.appendChild(extraLabel);
    var rowList = pgappRowList(
      [
        { key: "key", label: "Attribute", type: "text" },
        { key: "value", label: "Value", type: "text" },
      ],
      (html.attrs || []).map(function (pair) {
        return { key: pair[0], value: pair[1] };
      })
    );
    container.appendChild(rowList.el);

    return function generateAttrsClause() {
      var parts = [];
      if (idInput.value.trim()) parts.push("id: " + pgappMarkupStr(idInput.value.trim()));
      if (classInput.value.trim()) parts.push("class: " + pgappMarkupStr(classInput.value.trim()));
      rowList.getRows().forEach(function (r) {
        var key = (r.key || "").trim();
        if (!key) return;
        parts.push(key.replace(/-/g, "_") + ": " + pgappMarkupStr(r.value));
      });
      if (parts.length === 0) return "";
      return " attrs (" + parts.join(", ") + ")";
    };
  }

  // `requires: <role-or-scheme>` — a plain text field (any role/scheme
  // name is valid; roles themselves are never a fixed list, only
  // auth_schemes are, and those are offered as a `<datalist>` so a
  // known scheme can be picked without giving up free text for a
  // literal role).
  function pgappRequiresEditor(container, requires, authSchemes) {
    pgappSectionTitle(container, "Access");
    var input = pgappTextInput(requires);
    input.placeholder = "role or auth_scheme name (blank = no restriction)";
    if (authSchemes && authSchemes.length > 0) {
      var listId = "pgapp-builder-schemes-" + Math.random().toString(36).slice(2);
      var datalist = document.createElement("datalist");
      datalist.id = listId;
      authSchemes.forEach(function (name) {
        var opt = document.createElement("option");
        opt.value = name;
        datalist.appendChild(opt);
      });
      container.appendChild(datalist);
      input.setAttribute("list", listId);
    }
    pgappFieldRow(container, "requires:", input);
    return function generateRequiresClause() {
      var t = input.value.trim();
      return t ? " requires: " + t : "";
    };
  }

  // A generic `(<key>: "<value>", ...)` config editor — used by
  // `action`/`button calls`/`before_load`'s generic config blob (every
  // value always emitted as a quoted string; `markup.rs`'s
  // `parse_item_config` accepts a bare identifier or number too, but a
  // quoted string round-trips as the same JSON value either way).
  function pgappConfigEditor(container, config) {
    pgappSectionTitle(container, "Config");
    var initial = Object.keys(config || {}).map(function (k) {
      return { key: k, value: config[k] };
    });
    var rowList = pgappRowList(
      [
        { key: "key", label: "Key", type: "text" },
        { key: "value", label: "Value", type: "text" },
      ],
      initial
    );
    container.appendChild(rowList.el);
    return function generateConfigClause() {
      var parts = rowList
        .getRows()
        .filter(function (r) {
          return (r.key || "").trim() !== "";
        })
        .map(function (r) {
          return r.key.trim() + ": " + pgappMarkupStr(r.value);
        });
      if (parts.length === 0) return "";
      return " (" + parts.join(", ") + ")";
    };
  }

  function pgappEntityFields(meta, name) {
    var entities = meta.entities || [];
    for (var i = 0; i < entities.length; i++) {
      if (entities[i].name === name) return entities[i].fields;
    }
    return [];
  }

  function pgappEntityNames(meta) {
    return (meta.entities || []).map(function (e) {
      return e.name;
    });
  }

  // A field's default item-type kind, purely from its column type —
  // mirrors `item_types::default_kind_for`'s tiny fixed mapping. Used
  // to decide whether a Form/EditableTable field's `item_types` row is
  // actually redundant (kind == this field's own default, config
  // empty) and can be skipped when regenerating markup, rather than
  // emitting a needless explicit `item <field> as <kind>` line for
  // every single field every time.
  function pgappDefaultKindFor(fieldType) {
    if (fieldType === "boolean") return "checkbox";
    if (fieldType === "id") return "readonly";
    return "text";
  }

  // Renders the per-field `item <field> [as <kind> [(...)]] [attrs
  // (...)]` editor for a Form/EditableTable: one row per field the
  // component actually includes (`fieldNames`, in order), each with an
  // item-type-kind dropdown, a raw config-string input (comma-separated
  // `key: value` pairs — a full sub-row-list per field would be one
  // nesting level too many for this dialog to stay usable), and its own
  // `attrs (...)` sub-editor. Returns a function producing every
  // needed `item ...` line (one per field that actually differs from
  // its type's default with no config/attrs — matching how a hand-
  // written file only bothers with `item` lines it actually needs).
  function pgappItemTypesEditor(container, fieldNames, entityFields, itemTypes, fieldHtml, itemTypeKinds) {
    pgappSectionTitle(container, "Field item types");
    var byName = {};
    (itemTypes || []).forEach(function (row) {
      byName[row.field] = row;
    });
    var htmlByName = {};
    (fieldHtml || []).forEach(function (row) {
      htmlByName[row.field] = row.html;
    });
    var fieldTypeByName = {};
    (entityFields || []).forEach(function (f) {
      fieldTypeByName[f.name] = f.type;
    });

    var rows = [];
    fieldNames.forEach(function (name) {
      var current = byName[name] || { kind: pgappDefaultKindFor(fieldTypeByName[name]), config: {} };
      var configText = "";
      var configKeys = Object.keys(current.config || {});
      if (configKeys.length === 1 && configKeys[0] === "query") {
        configText = "from query " + current.config.query;
      } else {
        configText = configKeys
          .map(function (k) {
            var v = current.config[k];
            if (k === "choices" && Array.isArray(v)) return v.join(", ");
            return k + ": " + v;
          })
          .join(", ");
      }
      var row = document.createElement("div");
      row.className = "pgapp-field";
      var label = document.createElement("label");
      label.className = "pgapp-label";
      label.textContent = name;
      row.appendChild(label);
      var kindSel = pgappSelect(itemTypeKinds, current.kind);
      kindSel.dataset.field = name;
      row.appendChild(kindSel);
      var configInput = pgappTextInput(configText);
      configInput.placeholder = 'key: "value", ... OR "choice1", "choice2" OR from query <name>';
      configInput.dataset.field = name;
      row.appendChild(configInput);
      container.appendChild(row);
      var attrsGen = pgappAttrsEditor(container, htmlByName[name]);
      rows.push({ field: name, kindSel: kindSel, configInput: configInput, attrsGen: attrsGen, fieldType: fieldTypeByName[name] });
    });

    return function generateItemLines() {
      var lines = [];
      rows.forEach(function (r) {
        var kind = r.kindSel.value;
        var raw = r.configInput.value.trim();
        var configClause = "";
        if (raw.indexOf("from query ") === 0) {
          configClause = " from query " + raw.slice("from query ".length).trim();
        } else if (raw) {
          // Either a comma-separated `key: value` list, or a comma-separated
          // list of bare choices (no colons at all) — mirrors
          // `parse_item_config`'s two accepted shapes.
          var parts = raw.split(",").map(function (s) {
            return s.trim();
          });
          var hasColon = parts.some(function (p) {
            return p.indexOf(":") !== -1;
          });
          if (hasColon) {
            var kvs = parts.map(function (p) {
              var idx = p.indexOf(":");
              var k = p.slice(0, idx).trim();
              var v = p.slice(idx + 1).trim().replace(/^"|"$/g, "");
              return k + ": " + pgappMarkupStr(v);
            });
            configClause = " (" + kvs.join(", ") + ")";
          } else {
            var choices = parts.map(function (p) {
              return pgappMarkupStr(p.replace(/^"|"$/g, ""));
            });
            configClause = " (" + choices.join(", ") + ")";
          }
        }
        var attrsClause = r.attrsGen();
        var isDefaultKind = kind === pgappDefaultKindFor(r.fieldType) && !configClause;
        if (isDefaultKind && !attrsClause) return;
        var asClause = isDefaultKind ? "" : " as " + kind;
        lines.push("      item " + r.field + asClause + configClause + attrsClause);
      });
      return lines;
    };
  }

  // Every RuntimeComponent kind's structured-editor renderer: `(container,
  // data, meta) -> { generate() }`. `data`/`meta` are `{}`/already-
  // fetched JSON (see `admin_component_structured`/`admin_app_meta`);
  // `generate()` returns the complete markup text for exactly one
  // component (no page wrapper), 4-space indented to match
  // `page_reorder`'s splice points and the existing `COMPONENT_TEMPLATES`
  // convention.
  var PGAPP_KIND_RENDERERS = {
    text: function (container, data, meta) {
      var textArea = pgappTextArea(data.text, 3);
      pgappFieldRow(container, "Text", textArea);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          return "    text " + pgappMarkupStr(textArea.value) + requiresGen() + attrsGen();
        },
      };
    },

    link: function (container, data, meta) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var pageSel = pgappSelect(meta.pages, data.target_page);
      pgappFieldRow(container, "Target page", pageSel);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          return "    link " + pgappMarkupStr(labelInput.value) + " -> page " + pageSel.value + requiresGen() + attrsGen();
        },
      };
    },

    region: function (container, data, meta) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var querySel = pgappSelect(meta.queries, data.query);
      pgappFieldRow(container, "From query", querySel);
      pgappSectionTitle(container, "Columns (blank = show every column the query returns)");
      var colsList = pgappRowList([{ key: "col", label: "Column", type: "text" }], (data.columns || []).map(function (c) {
        return { col: c };
      }));
      container.appendChild(colsList.el);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          var cols = colsList
            .getRows()
            .map(function (r) {
              return r.col;
            })
            .filter(function (c) {
              return c && c.trim() !== "";
            });
          var body = cols.length > 0 ? " {\n      columns: " + cols.join(", ") + "\n    }" : "";
          return (
            "    region " + pgappMarkupStr(labelInput.value) + " from query " + querySel.value + body + requiresGen() + attrsGen()
          );
        },
      };
    },

    action: function (container, data, meta) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var nameSel = pgappSelect(meta.actions, data.name);
      pgappFieldRow(container, "Calls", nameSel);
      var configGen = pgappConfigEditor(container, data.config || {});
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          return (
            "    action " + pgappMarkupStr(labelInput.value) + " calls " + nameSel.value + configGen() + requiresGen() + attrsGen()
          );
        },
      };
    },

    button: function (container, data, meta) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var behavior = (data.behavior && data.behavior.type) || "redirect";
      var behaviorSel = pgappSelect(["redirect", "run_action"], behavior);
      pgappFieldRow(container, "Behavior", behaviorSel);

      var redirectWrap = document.createElement("div");
      var pageSel = pgappSelect(meta.pages, data.behavior && data.behavior.target_page);
      pgappFieldRow(redirectWrap, "Target page", pageSel);
      pgappSectionTitle(redirectWrap, "Forwarded parameters (this page's field -> new name)");
      var paramsList = pgappRowList(
        [
          { key: "param", label: "New param name", type: "text" },
          { key: "source", label: "Source field on this page", type: "text" },
        ],
        ((data.behavior && data.behavior.extra_params) || []).map(function (pair) {
          return { param: pair[0], source: pair[1] };
        })
      );
      redirectWrap.appendChild(paramsList.el);
      container.appendChild(redirectWrap);

      var runActionWrap = document.createElement("div");
      var actionSel = pgappSelect(meta.actions, data.behavior && data.behavior.name);
      pgappFieldRow(runActionWrap, "Calls", actionSel);
      var configGen = pgappConfigEditor(runActionWrap, (data.behavior && data.behavior.config) || {});
      container.appendChild(runActionWrap);

      function syncVisibility() {
        redirectWrap.style.display = behaviorSel.value === "redirect" ? "" : "none";
        runActionWrap.style.display = behaviorSel.value === "run_action" ? "" : "none";
      }
      behaviorSel.addEventListener("change", syncVisibility);
      syncVisibility();

      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          var head = "    button " + pgappMarkupStr(labelInput.value);
          if (behaviorSel.value === "redirect") {
            var params = paramsList.getRows().filter(function (r) {
              return r.param && r.param.trim() && r.source && r.source.trim();
            });
            var paramsClause = params.length > 0
              ? " (" + params.map(function (r) { return r.source.trim() + ": " + r.param.trim(); }).join(", ") + ")"
              : "";
            return head + " -> page " + pageSel.value + paramsClause + requiresGen() + attrsGen();
          }
          return head + " calls " + actionSel.value + configGen() + requiresGen() + attrsGen();
        },
      };
    },

    chart: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var querySel = pgappSelect(meta.queries, data.query);
      pgappFieldRow(container, "From query", querySel);
      var typeSel = pgappSelect(meta.chart_types, data.chart_type || "bar");
      pgappFieldRow(container, "Chart type", typeSel);
      var xInput = pgappTextInput(data.x);
      pgappFieldRow(container, "X column", xInput);
      var yInput = pgappTextInput(data.y);
      pgappFieldRow(container, "Y column", yInput);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          return (
            "    chart " +
            pgappMarkupStr(titleInput.value) +
            " from query " +
            querySel.value +
            " {\n      type: " +
            typeSel.value +
            "\n      x: " +
            xInput.value.trim() +
            "\n      y: " +
            yInput.value.trim() +
            "\n    }" +
            requiresGen() +
            attrsGen()
          );
        },
      };
    },

    report: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);
      var displaySel = pgappSelect(["table", "cards", "list"], data.display || "table");
      pgappFieldRow(container, "Display mode", displaySel);

      pgappSectionTitle(container, "Read-only computed columns (name: SQL expression)");
      var computedList = pgappRowList(
        [
          { key: "name", label: "Name", type: "text" },
          { key: "sql", label: "SQL (e.g. t.qty * t.rate)", type: "text" },
        ],
        (data.computed || []).map(function (c) {
          return { name: c.name, sql: c.sql };
        })
      );
      container.appendChild(computedList.el);

      // A computed column is just another name the Columns picker can
      // place anywhere among the physical fields — `columns:` accepts
      // either kind interchangeably (see meta/sync.rs's validation), so
      // this is the only way to control a computed column's actual
      // render position (there's no separate "order" on the computed
      // list itself). Seeded from whatever's defined above at the time
      // this panel opened; a computed column added after that isn't
      // selectable here until the panel is reopened.
      pgappSectionTitle(container, "Columns (drag any field or computed column into place)");
      var colsList = pgappFieldPickerList(
        entityFields.concat(computedNamesPlaceholder(computedList).map(function (n) { return { name: n }; })),
        data.columns
      );
      container.appendChild(colsList.el);

      pgappSectionTitle(container, "Display format masks");
      var formatsList = pgappRowList(
        [
          { key: "field", label: "Column", type: "text" },
          { key: "kind", label: "Mask", type: "select", options: ["currency", "percent", "number", "date"] },
          { key: "param", label: "Decimals (number) / pattern (date)", type: "text" },
        ],
        (data.formats || []).map(function (f) {
          var param = f.mask.kind === "number" ? String(f.mask.decimals || 0) : f.mask.kind === "date" ? f.mask.pattern || "%Y-%m-%d" : "";
          return { field: f.field, kind: f.mask.kind, param: param };
        })
      );
      container.appendChild(formatsList.el);

      pgappSectionTitle(container, "Column headings & alignment (optional overrides)");
      var headingsList = pgappRowList(
        [
          { key: "field", label: "Column", type: "select", options: entityFields.map(function (f) { return f.name; }) },
          { key: "heading", label: "Display name", type: "text" },
          { key: "align", label: "Align", type: "select", options: ["left", "center", "right"] },
        ],
        entityFields
          .filter(function (f) {
            return (data.headings && data.headings[f.name]) || (data.aligns && data.aligns[f.name] && data.aligns[f.name] !== "left");
          })
          .map(function (f) {
            return {
              field: f.name,
              heading: (data.headings && data.headings[f.name]) || "",
              align: (data.aligns && data.aligns[f.name]) || "left",
            };
          })
      );
      container.appendChild(headingsList.el);

      pgappSectionTitle(container, "Aggregates (footer sum/avg/count/min/max per column)");
      var aggregatesList = pgappRowList(
        [
          {
            key: "field",
            label: "Column",
            type: "select",
            options: entityFields.map(function (f) { return f.name; }).concat(computedNamesPlaceholder(computedList)),
          },
          { key: "fn", label: "Function", type: "select", options: ["sum", "avg", "count", "min", "max"] },
        ],
        (data.aggregates || []).map(function (a) {
          return { field: a.field, fn: a.fn };
        })
      );
      container.appendChild(aggregatesList.el);

      pgappSectionTitle(container, "Control break (group consecutive rows by this column)");
      var breakOnWrap = document.createElement("div");
      var breakOnSel = pgappSelect(
        ["(none)"].concat(entityFields.map(function (f) { return f.name; }), computedNamesPlaceholder(computedList)),
        data.break_on || "(none)"
      );
      pgappFieldRow(breakOnWrap, "Break on", breakOnSel);
      container.appendChild(breakOnWrap);

      pgappSectionTitle(container, "Row highlight rules (first match wins, table display mode only)");
      var highlightsList = pgappRowList(
        [
          { key: "when", label: "When (SQL boolean expression)", type: "text" },
          { key: "color", label: "Color", type: "text" },
        ],
        (data.highlights || []).map(function (h) {
          return { when: h.when, color: h.color };
        })
      );
      container.appendChild(highlightsList.el);

      pgappSectionTitle(container, "Link a column to another page");
      var linkWrap = document.createElement("div");
      var linkFieldSel = pgappSelect(entityFields.map(function (f) { return f.name; }).concat(computedNamesPlaceholder(computedList)), data.link_column && data.link_column.field);
      pgappFieldRow(linkWrap, "Column", linkFieldSel);
      var linkPageSel = pgappSelect(["(none)"].concat(meta.pages || []), (data.link_column && data.link_column.target_page) || "(none)");
      pgappFieldRow(linkWrap, "Target page", linkPageSel);
      var linkParamsList = pgappRowList(
        [
          { key: "field", label: "Row field", type: "text" },
          { key: "param", label: "New param name", type: "text" },
        ],
        ((data.link_column && data.link_column.extra_params) || []).map(function (pair) {
          return { field: pair[0], param: pair[1] };
        })
      );
      pgappSectionTitle(linkWrap, "Extra forwarded parameters");
      linkWrap.appendChild(linkParamsList.el);
      container.appendChild(linkWrap);

      var pageSizeInput = pgappNumberInput(data.page_size == null ? 20 : data.page_size);
      pgappFieldRow(container, "Rows per page", pageSizeInput);

      pgappSectionTitle(container, "Run an action automatically before this report loads (optional)");
      var beforeLoadWrap = document.createElement("div");
      var beforeLoadSel = pgappSelect(["(none)"].concat(meta.actions || []), (data.before_load && data.before_load.name) || "(none)");
      pgappFieldRow(beforeLoadWrap, "Before-load action", beforeLoadSel);
      var beforeLoadConfigGen = pgappConfigEditor(beforeLoadWrap, (data.before_load && data.before_load.config) || {});
      container.appendChild(beforeLoadWrap);

      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);

      return {
        generate: function () {
          var lines = [];
          lines.push("    report " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          if (displaySel.value !== "table") {
            lines.push("      display: " + displaySel.value);
          }
          lines.push("      columns: " + pgappFieldPickerText(colsList));
          if (linkPageSel.value !== "(none)") {
            var params = linkParamsList.getRows().filter(function (r) {
              return r.field && r.field.trim() && r.param && r.param.trim();
            });
            var paramsClause = params.length > 0
              ? " (" + params.map(function (r) { return r.field.trim() + ": " + r.param.trim(); }).join(", ") + ")"
              : "";
            lines.push("      link: " + linkFieldSel.value + " -> page " + linkPageSel.value + paramsClause);
          }
          lines.push("      page_size: " + (parseInt(pageSizeInput.value, 10) || 20));
          if (beforeLoadSel.value !== "(none)") {
            lines.push("      before_load: " + beforeLoadSel.value + beforeLoadConfigGen());
          }
          computedList.getRows().forEach(function (r) {
            if (!r.name || !r.name.trim()) return;
            lines.push("      computed " + r.name.trim() + ": " + pgappMarkupStr(r.sql));
          });
          formatsList.getRows().forEach(function (r) {
            if (!r.field || !r.field.trim()) return;
            var maskText = r.kind === "number" ? "number(" + (parseInt(r.param, 10) || 0) + ")" : r.kind === "date" ? "date(" + pgappMarkupStr(r.param || "%Y-%m-%d") + ")" : r.kind;
            lines.push("      format " + r.field.trim() + ": " + maskText);
          });
          headingsList.getRows().forEach(function (r) {
            if (!r.field || !r.field.trim()) return;
            if (r.heading && r.heading.trim()) {
              lines.push("      heading " + r.field.trim() + ": " + pgappMarkupStr(r.heading));
            }
            if (r.align && r.align !== "left") {
              lines.push("      align " + r.field.trim() + ": " + r.align);
            }
          });
          aggregatesList.getRows().forEach(function (r) {
            if (!r.field || !r.field.trim()) return;
            lines.push("      aggregate " + r.field.trim() + ": " + r.fn);
          });
          if (breakOnSel.value !== "(none)") {
            lines.push("      break_on: " + breakOnSel.value);
          }
          highlightsList.getRows().forEach(function (r) {
            if (!r.when || !r.when.trim()) return;
            lines.push("      highlight (when: " + pgappMarkupStr(r.when) + ", color: " + pgappMarkupStr(r.color || "") + ")");
          });
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },

    form: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);

      pgappSectionTitle(container, "Fields");
      var fieldsList = pgappFieldPickerList(entityFields, data.fields);
      container.appendChild(fieldsList.el);

      var itemTypesGen = pgappItemTypesEditor(
        container,
        (data.fields || entityFields.map(function (f) { return f.name; })),
        entityFields,
        data.item_types,
        data.field_html,
        meta.item_types
      );

      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);

      return {
        generate: function () {
          var lines = [];
          lines.push("    form " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          lines.push("      fields: " + pgappFieldPickerText(fieldsList));
          itemTypesGen().forEach(function (l) {
            lines.push(l);
          });
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },

    editable_table: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);

      pgappSectionTitle(container, "Columns");
      var colsList = pgappFieldPickerList(entityFields, data.columns);
      container.appendChild(colsList.el);

      var itemTypesGen = pgappItemTypesEditor(
        container,
        (data.columns || entityFields.map(function (f) { return f.name; })),
        entityFields,
        data.item_types,
        data.field_html,
        meta.item_types
      );

      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);

      return {
        generate: function () {
          var lines = [];
          lines.push("    editable_table " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          lines.push("      columns: " + pgappFieldPickerText(colsList));
          itemTypesGen().forEach(function (l) {
            lines.push(l);
          });
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },

    dynamic_action: function (container, data, meta) {
      var eventSel = pgappSelect(["click", "change"], data.event || "change");
      pgappFieldRow(container, "On event", eventSel);
      var itemInput = pgappTextInput(data.item);
      pgappFieldRow(container, "Of item", itemInput);

      pgappSectionTitle(container, "Operations");
      var opsWrap = document.createElement("div");
      container.appendChild(opsWrap);
      var opRows = [];

      function addOpRow(initial) {
        initial = initial || { op: "show", item: "" };
        var row = document.createElement("div");
        row.className = "pgapp-builder-two-col";
        var opSel = pgappSelect(["show", "hide", "toggle", "set", "refresh"], initial.op);
        var targetInput = pgappTextInput(initial.item || initial.query);
        var extraInput = pgappTextInput(initial.when || initial.expr);
        extraInput.placeholder = "when (toggle) / to (set)";
        var delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
        delBtn.textContent = "✕";
        delBtn.addEventListener("click", function () {
          row.remove();
          opRows = opRows.filter(function (r) {
            return r.row !== row;
          });
        });

        function syncExtraVisibility() {
          extraInput.style.display = opSel.value === "toggle" || opSel.value === "set" ? "" : "none";
        }
        opSel.addEventListener("change", syncExtraVisibility);
        syncExtraVisibility();

        var col1 = document.createElement("div");
        col1.appendChild(opSel);
        col1.appendChild(targetInput);
        var col2 = document.createElement("div");
        col2.appendChild(extraInput);
        col2.appendChild(delBtn);
        row.appendChild(col1);
        row.appendChild(col2);
        opsWrap.appendChild(row);
        opRows.push({ row: row, opSel: opSel, targetInput: targetInput, extraInput: extraInput });
      }

      (data.ops || []).forEach(function (op) {
        addOpRow(op);
      });

      var addOpBtn = document.createElement("button");
      addOpBtn.type = "button";
      addOpBtn.className = "pgapp-btn pgapp-btn-secondary";
      addOpBtn.textContent = "+ Add operation";
      addOpBtn.addEventListener("click", function () {
        addOpRow(null);
      });
      container.appendChild(addOpBtn);

      return {
        generate: function () {
          var lines = [];
          lines.push("    on " + eventSel.value + " of " + itemInput.value.trim() + " {");
          opRows.forEach(function (r) {
            var target = r.targetInput.value.trim();
            if (!target) return;
            if (r.opSel.value === "show") lines.push("      show " + target);
            else if (r.opSel.value === "hide") lines.push("      hide " + target);
            else if (r.opSel.value === "refresh") lines.push("      refresh " + target);
            else if (r.opSel.value === "toggle") lines.push("      toggle " + target + " when " + pgappMarkupStr(r.extraInput.value));
            else if (r.opSel.value === "set") lines.push("      set " + target + " to " + pgappMarkupStr(r.extraInput.value));
          });
          lines.push("    }");
          return lines.join("\n");
        },
      };
    },

    dynamic_content: function (container, data, meta) {
      var labelInput = pgappTextInput(data.label);
      pgappFieldRow(container, "Label", labelInput);
      var nameSel = pgappSelect(meta.actions, data.name);
      pgappFieldRow(container, "Calls", nameSel);
      var configGen = pgappConfigEditor(container, data.config || {});
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          return (
            "    dynamic_content " +
            pgappMarkupStr(labelInput.value) +
            " calls " +
            nameSel.value +
            configGen() +
            requiresGen() +
            attrsGen()
          );
        },
      };
    },

    calendar: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);
      var fieldNames = entityFields.map(function (f) {
        return f.name;
      });
      var dateSel = pgappSelect(fieldNames, data.date_field);
      pgappFieldRow(container, "Date field", dateSel);
      var titleFieldSel = pgappSelect(fieldNames, data.title_field);
      pgappFieldRow(container, "Title field", titleFieldSel);
      var linkPageSel = pgappSelect(["(none)"].concat(meta.pages || []), data.link_page || "(none)");
      pgappFieldRow(container, "Link to page (optional)", linkPageSel);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          var lines = [];
          lines.push("    calendar " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          lines.push("      date: " + dateSel.value);
          lines.push("      title: " + titleFieldSel.value);
          if (linkPageSel.value !== "(none)") lines.push("      link: page " + linkPageSel.value);
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },

    map: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);
      var fieldNames = entityFields.map(function (f) {
        return f.name;
      });
      var latSel = pgappSelect(fieldNames, data.lat_field);
      pgappFieldRow(container, "Latitude field", latSel);
      var lngSel = pgappSelect(fieldNames, data.lng_field);
      pgappFieldRow(container, "Longitude field", lngSel);
      var titleFieldSel = pgappSelect(fieldNames, data.title_field);
      pgappFieldRow(container, "Title field", titleFieldSel);
      var linkPageSel = pgappSelect(["(none)"].concat(meta.pages || []), data.link_page || "(none)");
      pgappFieldRow(container, "Link to page (optional)", linkPageSel);
      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          var lines = [];
          lines.push("    map " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          lines.push("      lat: " + latSel.value);
          lines.push("      lng: " + lngSel.value);
          lines.push("      title: " + titleFieldSel.value);
          if (linkPageSel.value !== "(none)") lines.push("      link: page " + linkPageSel.value);
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },

    faceted_search: function (container, data, meta) {
      var titleInput = pgappTextInput(data.title);
      pgappFieldRow(container, "Title", titleInput);
      var entitySel = pgappSelect(pgappEntityNames(meta), data.entity);
      pgappFieldRow(container, "Of entity", entitySel);
      var entityFields = data.entity_fields && data.entity_fields.length > 0 ? data.entity_fields : pgappEntityFields(meta, entitySel.value);
      var fieldNames = entityFields.map(function (f) {
        return f.name;
      });

      pgappSectionTitle(container, "Facets (the sibling Report bound to the same entity gets filtered by these)");
      var facetsList = pgappRowList(
        [
          { key: "column", label: "Column", type: "select", options: fieldNames },
          { key: "kind", label: "Kind", type: "select", options: ["checkbox_list", "range", "date_range"] },
        ],
        (data.facets || []).map(function (f) {
          return { column: f.column, kind: f.kind };
        })
      );
      container.appendChild(facetsList.el);

      var requiresGen = pgappRequiresEditor(container, data.requires, meta.auth_schemes);
      var attrsGen = pgappAttrsEditor(container, data.html);
      return {
        generate: function () {
          var lines = [];
          lines.push("    faceted_search " + pgappMarkupStr(titleInput.value) + " of " + entitySel.value + " {");
          facetsList.getRows().forEach(function (r) {
            if (!r.column || !r.column.trim()) return;
            lines.push("      facet " + r.column.trim() + " as " + r.kind);
          });
          lines.push("    }" + requiresGen() + attrsGen());
          return lines.join("\n");
        },
      };
    },
  };

  // A Report's `link:` column can also target one of its own computed
  // columns, not just an entity field — this pulls their current names
  // back out of the (already-rendered) computed-columns row list, live,
  // so the "Column" dropdown above always reflects whatever's currently
  // typed into that section rather than only what the component had at
  // load time. Best-effort: computed column names typed *after* the
  // dropdown was built won't retroactively appear in it (a full re-
  // render on every keystroke would be needlessly complex for this
  // dialog); picking the plain field-name text is always still possible
  // since the dropdown falls back to a text input when its options list
  // doesn't already contain the desired value... actually a `<select>`
  // can't hold an arbitrary typed value, so this is a known, minor
  // limitation — see the doc on `pgappSelect`.
  function computedNamesPlaceholder(computedList) {
    return computedList
      .getRows()
      .map(function (r) {
        return r.name;
      })
      .filter(function (n) {
        return n && n.trim() !== "";
      });
  }

  // Opens the structured editor for `kind`, prefilled from `data`
  // (already-fetched JSON) using `meta` (already-fetched app-meta JSON)
  // for its dropdowns. Resolves to the generated markup text on Save,
  // or null on Cancel/Escape — same contract as `pgappSourceEditor`, so
  // callers barely change: only which editor function they call, and
  // what they submit is still just `source=<text>` to the same routes.
  function pgappStructuredEditor(dialogTitle, kind, data, meta) {
    return new Promise(function (resolve) {
      pgappEnsureBuilderStyle();
      var overlay = document.createElement("div");
      overlay.className = "pgapp-dialog-overlay";
      var box = document.createElement("div");
      box.className = "pgapp-dialog-box pgapp-builder-form-box";
      box.setAttribute("role", "alertdialog");
      box.setAttribute("aria-modal", "true");
      var p = document.createElement("p");
      p.className = "pgapp-dialog-message";
      p.textContent = dialogTitle;
      box.appendChild(p);

      var body = document.createElement("div");
      body.className = "pgapp-builder-form-body";
      box.appendChild(body);

      var spec = PGAPP_KIND_RENDERERS[kind];
      var rendered = null;
      if (spec) {
        rendered = spec(body, data || {}, meta || {});
      } else {
        var err = document.createElement("p");
        err.textContent = "No structured editor for kind '" + kind + "' yet.";
        body.appendChild(err);
      }

      var actions = document.createElement("div");
      actions.className = "pgapp-dialog-actions";
      var cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "pgapp-btn pgapp-btn-secondary";
      cancelBtn.textContent = "Cancel";
      var saveBtn = document.createElement("button");
      saveBtn.type = "button";
      saveBtn.className = "pgapp-btn pgapp-btn-primary";
      saveBtn.textContent = "Save";

      function cleanup() {
        document.removeEventListener("keydown", onKey);
        overlay.remove();
      }
      function onKey(ev) {
        if (ev.key === "Escape") {
          cleanup();
          resolve(null);
        }
      }
      cancelBtn.addEventListener("click", function () {
        cleanup();
        resolve(null);
      });
      saveBtn.addEventListener("click", function () {
        if (!rendered) {
          cleanup();
          resolve(null);
          return;
        }
        var text;
        try {
          text = rendered.generate();
        } catch (e) {
          pgappAlert("Couldn't build markup from the form: " + e);
          return;
        }
        cleanup();
        resolve(text);
      });
      actions.appendChild(cancelBtn);
      actions.appendChild(saveBtn);
      box.appendChild(actions);
      overlay.appendChild(box);
      document.body.appendChild(overlay);
      document.addEventListener("keydown", onKey);
    });
  }

  // Fetches `admin_app_meta` for `target.page` — resolves to `{}`
  // (never rejects) on failure, mirroring `fetchPagesList`.
  function fetchAppMeta(target) {
    return fetch(pgappAdminPagesUrl(target, "/app-meta"))
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        return data.ok ? data.meta : {};
      })
      .catch(function () {
        return {};
      });
  }

  // Restyles the App Builder's plain id/kind/ordinal table row (from
  // `columns: id, kind, ordinal` in examples/app_builder.pgapp) into a
  // compact list row: a kind icon, an ordinal badge, and icon-only
  // Edit/Delete actions. Edit fetches the component's exact current
  // markup text and opens it in a full raw-source editor (see
  // `pgappSourceEditor`) — every attribute of every kind is editable
  // this way, not a fixed label/columns subset. The ordinal column
  // doubles as the `idx` the edit/delete routes expect, since
  // `meta::sync_app` always re-derives ordinal from file order on
  // every reload — it's never stale relative to the file.
  // APEX Page-Designer-style: the flat component list becomes a
  // clickable "rendering tree" on the left, and a persistent Property
  // Editor panel on the right shows the selected component's full
  // structured form inline — same `PGAPP_KIND_RENDERERS` functions
  // `pgappStructuredEditor`'s modal already uses, just targeting a
  // docked container instead of a dialog. The modal itself
  // (`pgappStructuredEditor`) and the raw-markup editor
  // (`pgappSourceEditor`) both stay reachable from the panel's own
  // footer — this replaces *where* editing happens, not the
  // structured-form code that does it.
  function bindComponentDesigner() {
    var treeSlot = document.getElementById("pgapp-designer-tree");
    var propsSlot = document.getElementById("pgapp-designer-properties-slot");
    if (!treeSlot || !propsSlot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid(target)) return;
    pgappEnsureBuilderStyle();

    // Re-parent the tree region and the properties placeholder (two
    // separate top-level page components, each in its own numbered
    // `#pgapp-cN` wrapper) into one flex layout, side by side.
    var treeWrap = treeSlot.closest('[id^="pgapp-c"]') || treeSlot;
    var propsWrap = propsSlot.closest('[id^="pgapp-c"]') || propsSlot;
    var layout = document.createElement("div");
    layout.className = "pgapp-designer-layout";
    treeWrap.parentNode.insertBefore(layout, treeWrap);
    layout.appendChild(treeWrap);
    layout.appendChild(propsWrap);

    propsSlot.textContent = "";
    propsSlot.classList.add("pgapp-designer-properties");
    var placeholder = document.createElement("p");
    placeholder.className = "pgapp-designer-properties-empty";
    placeholder.textContent = "Select a component on the left to edit its properties.";
    propsSlot.appendChild(placeholder);

    var tbodies = treeWrap.querySelectorAll(".pgapp-draggable-rows tbody, tbody");
    var allRows = [];
    for (var t = 0; t < tbodies.length; t++) {
      var rows = tbodies[t].querySelectorAll("tr");
      for (var i = 0; i < rows.length; i++) {
        allRows.push(rows[i]);
      }
    }

    function selectRow(row, idx, kind) {
      for (var r = 0; r < allRows.length; r++) {
        allRows[r].classList.remove("pgapp-selected");
      }
      row.classList.add("pgapp-selected");
      loadPropertyPanel(target, idx, kind, propsSlot);
    }

    allRows.forEach(function (row) {
      var cells = row.querySelectorAll("td");
      if (cells.length < 3) return;
      var kind = cells[1].textContent.trim();
      var idx = cells[2].textContent.trim();

      var icon = document.createElement("span");
      icon.className = "pgapp-component-kind-icon";
      icon.title = kind;
      icon.textContent = COMPONENT_KIND_ICONS[kind] || "•";
      cells[1].textContent = "";
      cells[1].className = "pgapp-component-label";
      cells[1].appendChild(icon);
      cells[1].appendChild(document.createTextNode(" " + kind));

      cells[2].className = "pgapp-component-ordinal";
      cells[2].textContent = "#" + idx;

      row.classList.add("pgapp-designer-row");
      row.addEventListener("click", function (ev) {
        if (ev.target.closest("button")) return; // let the delete button's own handler run
        selectRow(row, idx, kind);
      });

      var actionsTd = document.createElement("td");
      actionsTd.className = "pgapp-component-actions";
      var deleteBtn = document.createElement("button");
      deleteBtn.type = "button";
      deleteBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
      deleteBtn.title = "Delete";
      deleteBtn.setAttribute("aria-label", "Delete");
      deleteBtn.textContent = "✕";
      deleteBtn.addEventListener("click", function () {
        pgappConfirm("Delete this component? This can't be undone.").then(function (ok) {
          if (!ok) return;
          fetch(pgappAdminPagesUrl(target, "/components/" + encodeURIComponent(idx) + "/delete"), { method: "POST" })
            .then(function (r) {
              return r.json();
            })
            .then(function (data) {
              if (data.ok) location.reload();
              else pgappAlert("Couldn't delete component: " + data.error);
            })
            .catch(function (e) {
              pgappAlert("pgapp: " + e);
            });
        });
      });
      actionsTd.appendChild(deleteBtn);
      row.appendChild(actionsTd);
    });

    if (allRows.length > 0) {
      var firstCells = allRows[0].querySelectorAll("td");
      if (firstCells.length >= 3) {
        selectRow(allRows[0], firstCells[2].textContent.replace("#", "").trim(), firstCells[1].textContent.trim());
      }
    }
  }

  // Fetches component `idx`'s structured data + this page's app-meta,
  // then renders its property form directly into `panelEl` (the docked
  // panel), with Save/Edit-as-raw/Delete actions in its own footer —
  // the inline counterpart to `pgappStructuredEditor`'s modal, reusing
  // the exact same `PGAPP_KIND_RENDERERS[kind]` function to build the
  // fields either way.
  function loadPropertyPanel(target, idx, kind, panelEl) {
    panelEl.textContent = "";
    var loading = document.createElement("p");
    loading.className = "pgapp-designer-properties-empty";
    loading.textContent = "Loading " + kind + " #" + idx + "…";
    panelEl.appendChild(loading);

    var structuredFetch = fetch(pgappAdminPagesUrl(target, "/components/" + encodeURIComponent(idx) + "/structured")).then(function (r) {
      return r.json();
    });
    Promise.all([structuredFetch, fetchAppMeta(target)])
      .then(function (results) {
        var structured = results[0];
        var meta = results[1];
        panelEl.textContent = "";
        if (!structured.ok) {
          var err = document.createElement("p");
          err.className = "pgapp-alert pgapp-alert-error";
          err.textContent = "Couldn't load component: " + structured.error;
          panelEl.appendChild(err);
          return;
        }

        var header = document.createElement("div");
        header.className = "pgapp-panel-card-title";
        header.textContent = structured.kind + " #" + idx;
        panelEl.appendChild(header);

        var body = document.createElement("div");
        panelEl.appendChild(body);
        var spec = PGAPP_KIND_RENDERERS[structured.kind];
        var rendered = spec ? spec(body, structured.data || {}, meta || {}) : null;
        if (!rendered) {
          var note = document.createElement("p");
          note.textContent = "No structured editor for kind '" + structured.kind + "' yet — use \"Edit as raw markup\" below.";
          body.appendChild(note);
        }

        var actions = document.createElement("div");
        actions.className = "pgapp-dialog-actions";
        var rawBtn = document.createElement("button");
        rawBtn.type = "button";
        rawBtn.className = "pgapp-btn pgapp-btn-secondary";
        rawBtn.textContent = "Edit as raw markup";
        rawBtn.addEventListener("click", function () {
          var sourceFetch = fetch(pgappAdminPagesUrl(target, "/components/" + encodeURIComponent(idx) + "/source")).then(function (r) {
            return r.json();
          });
          Promise.all([sourceFetch, fetchPagesList(target)])
            .then(function (results2) {
              var data = results2[0];
              var pagesList = results2[1];
              if (!data.ok) {
                pgappAlert("Couldn't load component source: " + data.error);
                return;
              }
              pgappSourceEditor("Edit component (" + structured.kind + ") — raw markup", data.source, pagesList).then(function (edited) {
                if (edited === null) return;
                postComponentEdit(target, idx, "source=" + encodeURIComponent(edited));
              });
            })
            .catch(function (e) {
              pgappAlert("pgapp: " + e);
            });
        });
        actions.appendChild(rawBtn);

        if (rendered) {
          var saveBtn = document.createElement("button");
          saveBtn.type = "button";
          saveBtn.className = "pgapp-btn pgapp-btn-primary";
          saveBtn.textContent = "Save";
          saveBtn.addEventListener("click", function () {
            var text;
            try {
              text = rendered.generate();
            } catch (e) {
              pgappAlert("Couldn't build markup from the form: " + e);
              return;
            }
            postComponentEdit(target, idx, "source=" + encodeURIComponent(text));
          });
          actions.appendChild(saveBtn);
        }
        panelEl.appendChild(actions);
      })
      .catch(function (e) {
        panelEl.textContent = "";
        var err = document.createElement("p");
        err.className = "pgapp-alert pgapp-alert-error";
        err.textContent = "pgapp: " + e;
        panelEl.appendChild(err);
      });
  }

  // Fetches every page name currently in the target app's markup (see
  // `admin_pages_list` in server.rs) — powers the "Target page"
  // dropdown below instead of making the user hand-type a page
  // identifier. Resolves to [] (never rejects) on any failure, so a
  // missing/broken endpoint just means no dropdown shows, not a
  // broken editor.
  function fetchPagesList(target) {
    return fetch("/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin/pages-list")
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        return data.ok ? data.pages : [];
      })
      .catch(function () {
        return [];
      });
  }

  // Finds the one line in `text` that targets another page — either a
  // `link "Label" -> page X` component, or a report's `link: col ->
  // page X (params)` property — and pulls out everything
  // `renderLinkControls` needs to rebuild it. Returns null if neither
  // shape is present (most components don't target a page at all).
  function extractLinkParts(text) {
    var lines = text.split("\n");
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      var reportLink = line.match(/^(\s*link:\s*)(\S+)(\s*->\s*page\s+)([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?\s*$/);
      if (reportLink) {
        return {
          kind: "report-link",
          lineIndex: i,
          prefix: reportLink[1],
          column: reportLink[2],
          arrow: reportLink[3],
          page: reportLink[4],
          params: reportLink[5] || "",
        };
      }
      var linkComponent = line.match(/^(\s*link\s+"(?:[^"\\]|\\.)*"\s*->\s*page\s+)([A-Za-z_][A-Za-z0-9_]*)\s*$/);
      if (linkComponent) {
        return { kind: "link-component", lineIndex: i, prefix: linkComponent[1], page: linkComponent[2] };
      }
      // A button's redirect behavior (its "calls <action>" behavior has
      // no page target, so it never matches this and gets no dropdown).
      var buttonRedirect = line.match(/^(\s*button\s+"(?:[^"\\]|\\.)*"\s*->\s*page\s+)([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(([^)]*)\))?\s*$/);
      if (buttonRedirect) {
        return {
          kind: "button-redirect",
          lineIndex: i,
          prefix: buttonRedirect[1],
          page: buttonRedirect[2],
          params: buttonRedirect[3] || "",
        };
      }
    }
    return null;
  }

  // Structured, GUI-proper editing for the one property that's
  // genuinely error-prone to hand-type: a page target, plus (for a
  // report's `link:` or a button's redirect behavior) its source-field
  // -> page-param mappings. Inserted right before `textarea` inside
  // `container`; rewrites the relevant
  // line in `textarea.value` directly on every change, so Save still
  // just submits the textarea's own text — this is a convenience layer
  // on top of the raw editor, not a replacement for it. Rendered once
  // per call (at editor-open time, or on an explicit re-render trigger
  // like a kind change), never reactively on every keystroke, so it
  // never steals focus out from under someone typing in a param field.
  function renderLinkControls(container, textarea, pagesList) {
    var existing = container.querySelector(".pgapp-link-controls");
    if (existing) existing.remove();
    if (!pagesList || !pagesList.length) return;
    var parsed = extractLinkParts(textarea.value);
    if (!parsed) return;

    var wrap = document.createElement("div");
    wrap.className = "pgapp-link-controls";

    var targetRow = document.createElement("div");
    targetRow.className = "pgapp-link-controls-row";
    var label = document.createElement("label");
    label.textContent = "Target page";
    var select = document.createElement("select");
    select.className = "pgapp-select";
    pagesList.forEach(function (p) {
      var opt = document.createElement("option");
      opt.value = p;
      opt.textContent = p;
      if (p === parsed.page) opt.selected = true;
      select.appendChild(opt);
    });
    label.appendChild(select);
    targetRow.appendChild(label);
    wrap.appendChild(targetRow);

    // Each parenthesized pair is `<source_field>: <target_param_name>`
    // — for a report's `link:`, the CURRENT ROW's own column; for a
    // button's redirect, the CURRENT PAGE's own query-string value
    // (buttons aren't row-bound) — either way forwarded under a
    // (possibly different) name on the target page's own query string
    // (see `render.rs`'s `for (field, param) in extra_params` — `field`
    // is read first, `param` is what shows up in the URL).
    // `rowColumn`/`pageParam` below name the two halves the same way in
    // both cases, so the UI can't get them backwards the way a generic
    // `name`/`value` pair invites.
    var paramRows = [];
    var hasParams = parsed.kind === "report-link" || parsed.kind === "button-redirect";
    if (hasParams) {
      parsed.params.split(",").forEach(function (pair) {
        pair = pair.trim();
        if (!pair) return;
        var sep = pair.indexOf(":");
        if (sep === -1) return;
        paramRows.push({ rowColumn: pair.slice(0, sep).trim(), pageParam: pair.slice(sep + 1).trim() });
      });

      var sourceLabel = parsed.kind === "report-link" ? "row column" : "page field";

      var paramsList = document.createElement("div");
      paramsList.className = "pgapp-link-params-list";

      var rerenderParams = function () {
        paramsList.textContent = "";
        paramRows.forEach(function (row, i) {
          var rowEl = document.createElement("div");
          rowEl.className = "pgapp-link-param-row";
          var columnInput = document.createElement("input");
          columnInput.className = "pgapp-input";
          columnInput.placeholder = sourceLabel;
          columnInput.value = row.rowColumn;
          var paramInput = document.createElement("input");
          paramInput.className = "pgapp-input";
          paramInput.placeholder = "page param";
          paramInput.value = row.pageParam;
          var removeBtn = document.createElement("button");
          removeBtn.type = "button";
          removeBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
          removeBtn.title = "Remove parameter";
          removeBtn.textContent = "✕";
          columnInput.addEventListener("input", function () {
            row.rowColumn = columnInput.value;
            applyChange();
          });
          paramInput.addEventListener("input", function () {
            row.pageParam = paramInput.value;
            applyChange();
          });
          removeBtn.addEventListener("click", function () {
            paramRows.splice(i, 1);
            rerenderParams();
            applyChange();
          });
          rowEl.appendChild(columnInput);
          rowEl.appendChild(document.createTextNode(":"));
          rowEl.appendChild(paramInput);
          rowEl.appendChild(removeBtn);
          paramsList.appendChild(rowEl);
        });
      };
      rerenderParams();

      var addParamBtn = document.createElement("button");
      addParamBtn.type = "button";
      addParamBtn.className = "pgapp-btn pgapp-btn-secondary";
      addParamBtn.textContent = "+ Add parameter";
      addParamBtn.addEventListener("click", function () {
        paramRows.push({ rowColumn: "", pageParam: "" });
        rerenderParams();
        applyChange();
      });

      var paramsRow = document.createElement("div");
      paramsRow.className = "pgapp-link-controls-row";
      var paramsLabel = document.createElement("label");
      paramsLabel.textContent = "Link parameters (" + sourceLabel + " : page param)";
      paramsRow.appendChild(paramsLabel);
      paramsRow.appendChild(paramsList);
      paramsRow.appendChild(addParamBtn);
      wrap.appendChild(paramsRow);
    }

    function applyChange() {
      var lines = textarea.value.split("\n");
      if (parsed.kind === "link-component") {
        lines[parsed.lineIndex] = parsed.prefix + select.value;
      } else {
        var paramsStr = paramRows
          .filter(function (r) {
            return r.rowColumn;
          })
          .map(function (r) {
            return r.rowColumn + ": " + r.pageParam;
          })
          .join(", ");
        var parenthetical = paramsStr ? " (" + paramsStr + ")" : "";
        lines[parsed.lineIndex] =
          parsed.kind === "report-link"
            ? parsed.prefix + parsed.column + parsed.arrow + select.value + parenthetical
            : parsed.prefix + select.value + parenthetical;
      }
      textarea.value = lines.join("\n");
    }
    select.addEventListener("change", applyChange);

    container.insertBefore(wrap, textarea);
  }

  function postComponentEdit(target, idx, body) {
    fetch(pgappAdminPagesUrl(target, "/components/" + encodeURIComponent(idx) + "/edit"), {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: body,
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        if (data.ok) location.reload();
        else pgappAlert("Couldn't edit component: " + data.error);
      })
      .catch(function (e) {
        pgappAlert("pgapp: " + e);
      });
  }

  // Nested nav: a click-to-toggle affordance on the caret button, since
  // CSS-only :hover has no equivalent on touch devices and a submenu is
  // otherwise unreachable there.
  function bindNavToggles() {
    var toggles = document.querySelectorAll(".pgapp-navbar-toggle");
    for (var i = 0; i < toggles.length; i++) {
      toggles[i].addEventListener("click", function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        var li = this.closest(".pgapp-navbar-item");
        if (!li) return;
        var open = !li.classList.contains("pgapp-open");
        var openItems = document.querySelectorAll(".pgapp-navbar-item.pgapp-open");
        for (var j = 0; j < openItems.length; j++) {
          if (openItems[j] !== li) openItems[j].classList.remove("pgapp-open");
        }
        li.classList.toggle("pgapp-open", open);
        this.setAttribute("aria-expanded", open ? "true" : "false");
      });
    }
    document.addEventListener("click", function (ev) {
      if (ev.target.closest(".pgapp-navbar-item")) return;
      var openItems = document.querySelectorAll(".pgapp-navbar-item.pgapp-open");
      for (var j = 0; j < openItems.length; j++) openItems[j].classList.remove("pgapp-open");
    });
  }

  // Mobile nav: a hamburger toggle collapses/expands the nav links +
  // signed-in-user corner into a dropdown. Collapsed by default below
  // the theme's mobile breakpoint; desktop CSS ignores the open/closed
  // class entirely, so this never touches the wide layout.
  function bindMobileNavToggle() {
    var toggle = document.querySelector(".pgapp-nav-toggle");
    var nav = toggle && toggle.closest(".pgapp-nav");
    if (!toggle || !nav) return;
    toggle.addEventListener("click", function (ev) {
      ev.stopPropagation();
      var open = !nav.classList.contains("pgapp-nav-open");
      nav.classList.toggle("pgapp-nav-open", open);
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
    document.addEventListener("click", function (ev) {
      if (nav.contains(ev.target)) return;
      nav.classList.remove("pgapp-nav-open");
      toggle.setAttribute("aria-expanded", "false");
    });
  }

  // A small promise-based dialog used for both alert() and confirm()
  // below — styled via the .pgapp-dialog-* theme classes instead of the
  // browser's native (unstyleable, blocking) alert/confirm popups.
  function showDialog(message, buttons) {
    return new Promise(function (resolve) {
      var overlay = document.createElement("div");
      overlay.className = "pgapp-dialog-overlay";
      var box = document.createElement("div");
      box.className = "pgapp-dialog-box";
      box.setAttribute("role", "alertdialog");
      box.setAttribute("aria-modal", "true");
      var p = document.createElement("p");
      p.className = "pgapp-dialog-message";
      p.textContent = message;
      box.appendChild(p);
      var actions = document.createElement("div");
      actions.className = "pgapp-dialog-actions";
      var focusTarget = null;
      buttons.forEach(function (b) {
        var btn = document.createElement("button");
        btn.type = "button";
        btn.className = "pgapp-btn " + b.cls;
        btn.textContent = b.label;
        btn.addEventListener("click", function () {
          cleanup();
          resolve(b.value);
        });
        actions.appendChild(btn);
        if (b.primary) focusTarget = btn;
      });
      box.appendChild(actions);
      overlay.appendChild(box);
      document.body.appendChild(overlay);

      function onKey(ev) {
        if (ev.key === "Escape") {
          cleanup();
          resolve(false);
        }
      }
      function cleanup() {
        document.removeEventListener("keydown", onKey);
        overlay.remove();
      }
      document.addEventListener("keydown", onKey);
      if (focusTarget) focusTarget.focus();
    });
  }

  // Drop-in, non-blocking replacements for window.alert/window.confirm —
  // both return a Promise instead of blocking the thread, and render as
  // a themed dialog instead of a browser-chrome popup.
  function pgappAlert(message) {
    return showDialog(message, [{ label: "OK", cls: "pgapp-btn-primary", value: true, primary: true }]);
  }
  function pgappConfirm(message) {
    return showDialog(message, [
      { label: "Cancel", cls: "pgapp-btn-secondary", value: false },
      { label: "OK", cls: "pgapp-btn-destructive", value: true, primary: true },
    ]);
  }

  // Same non-blocking, themed idea as showDialog, but with a single
  // text input — the App Builder's "Rename page" button uses this in
  // place of window.prompt(). Resolves the input's value on OK/Enter,
  // or null on Cancel/Escape.
  function pgappPrompt(message, defaultValue) {
    return new Promise(function (resolve) {
      var overlay = document.createElement("div");
      overlay.className = "pgapp-dialog-overlay";
      var box = document.createElement("div");
      box.className = "pgapp-dialog-box";
      box.setAttribute("role", "alertdialog");
      box.setAttribute("aria-modal", "true");
      var p = document.createElement("p");
      p.className = "pgapp-dialog-message";
      p.textContent = message;
      box.appendChild(p);
      var input = document.createElement("input");
      input.className = "pgapp-input";
      input.type = "text";
      input.value = defaultValue || "";
      box.appendChild(input);
      var actions = document.createElement("div");
      actions.className = "pgapp-dialog-actions";
      var cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "pgapp-btn pgapp-btn-secondary";
      cancelBtn.textContent = "Cancel";
      var okBtn = document.createElement("button");
      okBtn.type = "button";
      okBtn.className = "pgapp-btn pgapp-btn-primary";
      okBtn.textContent = "OK";

      function cleanup() {
        document.removeEventListener("keydown", onKey);
        overlay.remove();
      }
      function onKey(ev) {
        if (ev.key === "Escape") {
          cleanup();
          resolve(null);
        } else if (ev.key === "Enter") {
          cleanup();
          resolve(input.value);
        }
      }
      cancelBtn.addEventListener("click", function () {
        cleanup();
        resolve(null);
      });
      okBtn.addEventListener("click", function () {
        cleanup();
        resolve(input.value);
      });
      actions.appendChild(cancelBtn);
      actions.appendChild(okBtn);
      box.appendChild(actions);
      overlay.appendChild(box);
      document.body.appendChild(overlay);
      document.addEventListener("keydown", onKey);
      input.focus();
      input.select();
    });
  }

  // Raw-markup editing: same shell as pgappPrompt, but a multi-line,
  // monospace `<textarea>` instead of a single-line `<input>` — the
  // App Builder's original component editor, still reachable today via
  // the "{ }" button next to a component's structured-editor pencil
  // (see `pgappStructuredEditor`, the primary editor since it renders a
  // real per-attribute property form instead of this raw text box) and
  // via its "Advanced: edit full app source" link's inline variant.
  // Resolves the textarea's value on Save, or null on Cancel/Escape
  // (Enter does *not* submit, unlike pgappPrompt, since newlines are
  // meaningful here).
  function pgappSourceEditor(title, initialText, pagesList) {
    return new Promise(function (resolve) {
      var overlay = document.createElement("div");
      overlay.className = "pgapp-dialog-overlay";
      var box = document.createElement("div");
      box.className = "pgapp-dialog-box pgapp-dialog-box-wide";
      box.setAttribute("role", "alertdialog");
      box.setAttribute("aria-modal", "true");
      var p = document.createElement("p");
      p.className = "pgapp-dialog-message";
      p.textContent = title;
      box.appendChild(p);
      var textarea = document.createElement("textarea");
      textarea.className = "pgapp-input pgapp-source-textarea";
      textarea.rows = 10;
      textarea.value = initialText || "";
      box.appendChild(textarea);
      renderLinkControls(box, textarea, pagesList);
      var actions = document.createElement("div");
      actions.className = "pgapp-dialog-actions";
      var cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "pgapp-btn pgapp-btn-secondary";
      cancelBtn.textContent = "Cancel";
      var saveBtn = document.createElement("button");
      saveBtn.type = "button";
      saveBtn.className = "pgapp-btn pgapp-btn-primary";
      saveBtn.textContent = "Save";

      function cleanup() {
        document.removeEventListener("keydown", onKey);
        overlay.remove();
      }
      function onKey(ev) {
        if (ev.key === "Escape") {
          cleanup();
          resolve(null);
        }
      }
      cancelBtn.addEventListener("click", function () {
        cleanup();
        resolve(null);
      });
      saveBtn.addEventListener("click", function () {
        cleanup();
        resolve(textarea.value);
      });
      actions.appendChild(cancelBtn);
      actions.appendChild(saveBtn);
      box.appendChild(actions);
      overlay.appendChild(box);
      document.body.appendChild(overlay);
      document.addEventListener("keydown", onKey);
      textarea.focus();
    });
  }

  // Delete/destructive forms carry `data-pgapp-confirm="<message>"`
  // instead of a native onsubmit="return confirm(...)" — this intercepts
  // the submit, shows the themed confirm dialog, and only actually
  // submits once the user confirms.
  function bindConfirmForms() {
    var forms = document.querySelectorAll("form[data-pgapp-confirm]");
    for (var i = 0; i < forms.length; i++) {
      var form = forms[i];
      if (form.__pgappConfirmBound) continue;
      form.__pgappConfirmBound = true;
      form.addEventListener("submit", function (ev) {
        var f = ev.currentTarget;
        if (f.__pgappConfirmed) return;
        ev.preventDefault();
        pgappConfirm(f.getAttribute("data-pgapp-confirm")).then(function (ok) {
          if (!ok) return;
          f.__pgappConfirmed = true;
          if (f.requestSubmit) f.requestSubmit();
          else f.submit();
        });
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", bindDynamicActions);
    document.addEventListener("DOMContentLoaded", bindNavToggles);
    document.addEventListener("DOMContentLoaded", bindMobileNavToggle);
    document.addEventListener("DOMContentLoaded", bindConfirmForms);
    document.addEventListener("DOMContentLoaded", bindDraggableRows);
    document.addEventListener("DOMContentLoaded", bindPreviewLink);
    document.addEventListener("DOMContentLoaded", bindAddComponentForm);
    document.addEventListener("DOMContentLoaded", bindComponentDesigner);
    document.addEventListener("DOMContentLoaded", bindContextHeader);
    document.addEventListener("DOMContentLoaded", bindAddPageForm);
    document.addEventListener("DOMContentLoaded", bindPageCardActions);
    document.addEventListener("DOMContentLoaded", bindNewAppProcessing);
    document.addEventListener("DOMContentLoaded", bindAdvancedSourceLink);
    document.addEventListener("DOMContentLoaded", bindAppSettingsLink);
    document.addEventListener("DOMContentLoaded", bindEntitiesPanel);
    document.addEventListener("DOMContentLoaded", bindQueriesPanel);
    document.addEventListener("DOMContentLoaded", bindNavPanel);
    document.addEventListener("DOMContentLoaded", bindAppSettingsForm);
    document.addEventListener("DOMContentLoaded", bindSecretsPanel);
    document.addEventListener("DOMContentLoaded", bindDestroyAppPanel);
    document.addEventListener("DOMContentLoaded", bindDestroyWorkspacePanel);
    document.addEventListener("DOMContentLoaded", wireFileBrowseLinks);
  } else {
    bindDynamicActions();
    bindNavToggles();
    bindMobileNavToggle();
    bindConfirmForms();
    bindDraggableRows();
    bindPreviewLink();
    bindAddComponentForm();
    bindComponentDesigner();
    bindContextHeader();
    bindAddPageForm();
    bindPageCardActions();
    bindNewAppProcessing();
    bindAdvancedSourceLink();
    bindAppSettingsLink();
    bindEntitiesPanel();
    bindQueriesPanel();
    bindNavPanel();
    bindAppSettingsForm();
    bindSecretsPanel();
    bindDestroyAppPanel();
    bindDestroyWorkspacePanel();
    wireFileBrowseLinks();
  }

  return {
    getItem: getItem,
    setItem: setItem,
    refreshRegion: refreshRegion,
    openPopup: openPopup,
    filterPopup: filterPopup,
    syncCheckboxGroup: syncCheckboxGroup,
    setStarRating: setStarRating,
    addListManagerItem: addListManagerItem,
    removeListManagerItem: removeListManagerItem,
    shuttleMove: shuttleMove,
    syncRichText: syncRichText,
    uploadFile: uploadFile,
    alert: pgappAlert,
    confirm: pgappConfirm,
  };
})();
