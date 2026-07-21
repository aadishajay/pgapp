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

  function runOps(ops) {
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
      }
    } finally {
      daDepth--;
    }
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
          runOps(da.ops);
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

  // A "Delete" button per page card on the Pages page (`.pgapp-cards`
  // rows there) — the page's own name is the card's visible link text,
  // so no hidden id column is needed the way the component list needs
  // one for its ordinal-based `idx`.
  function bindPageCardActions() {
    var rows = document.querySelectorAll(".pgapp-page-cards tbody tr");
    if (!rows.length) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid2(target)) return;
    for (var i = 0; i < rows.length; i++) {
      (function (row) {
        var link = row.querySelector("a.pgapp-link");
        var pageName = link ? link.textContent.trim() : row.textContent.trim();
        var pageUrl =
          "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/admin/pages/" + encodeURIComponent(pageName);

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
        row.appendChild(renameBtn);

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
        row.appendChild(btn);
      })(rows[i]);
    }
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
    action: '    action "Run action" runs <action_name>',
    link: '    link "Go" -> page <PageName>',
  };

  // The App Builder's "Add Component" panel: a `text ... attrs (id:
  // "pgapp-add-component-slot")` placeholder gets a kind picker plus a
  // raw markup textarea appended into it. Picking a kind just seeds the
  // textarea with a starter template (COMPONENT_TEMPLATES) — the
  // textarea's own content, not the kind picker, is what's actually
  // submitted, so any of the 8 component kinds and any of their
  // attributes can be added, not a fixed structured-fields subset. If
  // the textarea's text targets a page (a `link` component, or a
  // report's `link:` property), `renderLinkControls` also renders a
  // proper "Target page" dropdown (+ parameter rows for a report link)
  // above it — a real GUI control for the one property that's
  // otherwise easy to typo, re-rendered whenever the kind changes.
  // POSTs the raw text to the target app's own
  // `/admin/pages/:page/components/add`.
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

    var form = document.createElement("form");
    form.className = "pgapp-add-component-form";

    var kindSel = document.createElement("select");
    kindSel.className = "pgapp-select";
    Object.keys(COMPONENT_TEMPLATES).forEach(function (k) {
      var opt = document.createElement("option");
      opt.value = k;
      opt.textContent = k;
      kindSel.appendChild(opt);
    });

    var sourceArea = document.createElement("textarea");
    sourceArea.className = "pgapp-input pgapp-source-textarea";
    sourceArea.rows = 4;
    sourceArea.value = COMPONENT_TEMPLATES[kindSel.value];

    var pagesListCache = [];
    fetchPagesList(target).then(function (pages) {
      pagesListCache = pages;
      renderLinkControls(form, sourceArea, pagesListCache);
    });

    kindSel.addEventListener("change", function () {
      sourceArea.value = COMPONENT_TEMPLATES[kindSel.value];
      renderLinkControls(form, sourceArea, pagesListCache);
    });

    var addBtn = document.createElement("button");
    addBtn.type = "submit";
    addBtn.className = "pgapp-btn pgapp-btn-primary";
    addBtn.textContent = "Add";

    [kindSel, sourceArea, addBtn].forEach(function (el) {
      form.appendChild(el);
    });
    slot.appendChild(form);

    form.addEventListener("submit", function (ev) {
      ev.preventDefault();
      fetch(pgappAdminPagesUrl(target, "/components/add"), {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: "source=" + encodeURIComponent(sourceArea.value),
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
  function bindComponentRowActions() {
    var tbodies = document.querySelectorAll(".pgapp-draggable-rows tbody");
    for (var t = 0; t < tbodies.length; t++) {
      var target = pgappEditTarget();
      if (!pgappEditTargetValid(target)) continue;
      var rows = tbodies[t].querySelectorAll("tr");
      for (var i = 0; i < rows.length; i++) {
        (function (row) {
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

          var actionsTd = document.createElement("td");
          actionsTd.className = "pgapp-component-actions";

          var editBtn = document.createElement("button");
          editBtn.type = "button";
          editBtn.className = "pgapp-icon-btn";
          editBtn.title = "Edit";
          editBtn.setAttribute("aria-label", "Edit component");
          editBtn.textContent = "✎";
          editBtn.addEventListener("click", function () {
            var sourceFetch = fetch(pgappAdminPagesUrl(target, "/components/" + encodeURIComponent(idx) + "/source")).then(function (r) {
              return r.json();
            });
            Promise.all([sourceFetch, fetchPagesList(target)])
              .then(function (results) {
                var data = results[0];
                var pagesList = results[1];
                if (!data.ok) {
                  pgappAlert("Couldn't load component source: " + data.error);
                  return;
                }
                pgappSourceEditor("Edit component (" + kind + ")", data.source, pagesList).then(function (edited) {
                  if (edited === null) return;
                  postComponentEdit(target, idx, "source=" + encodeURIComponent(edited));
                });
              })
              .catch(function (e) {
                pgappAlert("pgapp: " + e);
              });
          });
          actionsTd.appendChild(editBtn);

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
        })(rows[i]);
      }
    }
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
    }
    return null;
  }

  // Structured, GUI-proper editing for the one property that's
  // genuinely error-prone to hand-type: a page target, plus (for a
  // report's `link:`) its row-column -> page-param mappings. Inserted
  // right before `textarea` inside `container`; rewrites the relevant
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

    var paramRows = [];
    if (parsed.kind === "report-link") {
      parsed.params.split(",").forEach(function (pair) {
        pair = pair.trim();
        if (!pair) return;
        var sep = pair.indexOf(":");
        if (sep === -1) return;
        paramRows.push({ name: pair.slice(0, sep).trim(), value: pair.slice(sep + 1).trim() });
      });

      var paramsList = document.createElement("div");
      paramsList.className = "pgapp-link-params-list";

      var rerenderParams = function () {
        paramsList.textContent = "";
        paramRows.forEach(function (row, i) {
          var rowEl = document.createElement("div");
          rowEl.className = "pgapp-link-param-row";
          var nameInput = document.createElement("input");
          nameInput.className = "pgapp-input";
          nameInput.placeholder = "page param";
          nameInput.value = row.name;
          var valueInput = document.createElement("input");
          valueInput.className = "pgapp-input";
          valueInput.placeholder = "row column";
          valueInput.value = row.value;
          var removeBtn = document.createElement("button");
          removeBtn.type = "button";
          removeBtn.className = "pgapp-icon-btn pgapp-icon-btn-destructive";
          removeBtn.title = "Remove parameter";
          removeBtn.textContent = "✕";
          nameInput.addEventListener("input", function () {
            row.name = nameInput.value;
            applyChange();
          });
          valueInput.addEventListener("input", function () {
            row.value = valueInput.value;
            applyChange();
          });
          removeBtn.addEventListener("click", function () {
            paramRows.splice(i, 1);
            rerenderParams();
            applyChange();
          });
          rowEl.appendChild(nameInput);
          rowEl.appendChild(document.createTextNode(":"));
          rowEl.appendChild(valueInput);
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
        paramRows.push({ name: "", value: "" });
        rerenderParams();
        applyChange();
      });

      var paramsRow = document.createElement("div");
      paramsRow.className = "pgapp-link-controls-row";
      var paramsLabel = document.createElement("label");
      paramsLabel.textContent = "Link parameters";
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
            return r.name;
          })
          .map(function (r) {
            return r.name + ": " + r.value;
          })
          .join(", ");
        lines[parsed.lineIndex] =
          parsed.prefix + parsed.column + parsed.arrow + select.value + (paramsStr ? " (" + paramsStr + ")" : "");
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

  // Full-property editing, APEX-Page-Designer-style but as a raw text
  // box instead of a property sheet: same shell as pgappPrompt, but a
  // multi-line, monospace `<textarea>` instead of a single-line
  // `<input>` — used by the App Builder's component "Edit" button
  // (prefilled with that component's exact current markup) and its
  // "Advanced: edit full app source" link's inline variant. Resolves
  // the textarea's value on Save, or null on Cancel/Escape (Enter does
  // *not* submit, unlike pgappPrompt, since newlines are meaningful
  // here).
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
    document.addEventListener("DOMContentLoaded", bindComponentRowActions);
    document.addEventListener("DOMContentLoaded", bindContextHeader);
    document.addEventListener("DOMContentLoaded", bindAddPageForm);
    document.addEventListener("DOMContentLoaded", bindPageCardActions);
    document.addEventListener("DOMContentLoaded", bindNewAppProcessing);
    document.addEventListener("DOMContentLoaded", bindAdvancedSourceLink);
  } else {
    bindDynamicActions();
    bindNavToggles();
    bindMobileNavToggle();
    bindConfirmForms();
    bindDraggableRows();
    bindPreviewLink();
    bindAddComponentForm();
    bindComponentRowActions();
    bindContextHeader();
    bindAddPageForm();
    bindPageCardActions();
    bindNewAppProcessing();
    bindAdvancedSourceLink();
  }

  return {
    getItem: getItem,
    setItem: setItem,
    refreshRegion: refreshRegion,
    openPopup: openPopup,
    filterPopup: filterPopup,
    alert: pgappAlert,
    confirm: pgappConfirm,
  };
})();
