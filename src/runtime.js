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
    var a = document.createElement("a");
    a.className = "pgapp-link pgapp-btn pgapp-btn-secondary";
    a.href = "/" + encodeURIComponent(target.workspace) + "/" + encodeURIComponent(target.app) + "/" + encodeURIComponent(target.page);
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = "Run this page ↗";
    slot.appendChild(document.createTextNode(" "));
    slot.appendChild(a);
  }

  // The App Builder's "Add Component" panel: another `text ... attrs
  // (id: "pgapp-add-component-slot")` placeholder gets real form
  // controls appended into it (kind/label/source/columns), POSTing to
  // the target app's own `/admin/pages/:page/components/add` on submit
  // — see `render_new_component` in server.rs for what each field means
  // per kind, and why only text/report/region are offered here.
  function bindAddComponentForm() {
    var slot = document.getElementById("pgapp-add-component-slot");
    if (!slot) return;
    var target = pgappEditTarget();
    if (!pgappEditTargetValid(target)) return;

    var form = document.createElement("form");
    form.className = "pgapp-add-component-form";

    var kindSel = document.createElement("select");
    kindSel.className = "pgapp-select";
    ["text", "report", "region"].forEach(function (k) {
      var opt = document.createElement("option");
      opt.value = k;
      opt.textContent = k;
      kindSel.appendChild(opt);
    });

    var labelInput = document.createElement("input");
    labelInput.className = "pgapp-input";
    labelInput.placeholder = "Title (report/region) or content (text)";

    var sourceInput = document.createElement("input");
    sourceInput.className = "pgapp-input";
    sourceInput.placeholder = "Entity name (report) or query name (region)";

    var columnsInput = document.createElement("input");
    columnsInput.className = "pgapp-input";
    columnsInput.placeholder = "Columns, comma-separated (report/region)";

    var addBtn = document.createElement("button");
    addBtn.type = "submit";
    addBtn.className = "pgapp-btn pgapp-btn-primary";
    addBtn.textContent = "Add Component";

    [kindSel, labelInput, sourceInput, columnsInput, addBtn].forEach(function (el) {
      form.appendChild(el);
    });
    slot.appendChild(form);

    form.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var body =
        "kind=" +
        encodeURIComponent(kindSel.value) +
        "&label=" +
        encodeURIComponent(labelInput.value) +
        "&source=" +
        encodeURIComponent(sourceInput.value) +
        "&columns=" +
        encodeURIComponent(columnsInput.value);
      fetch(pgappAdminPagesUrl(target, "/components/add"), {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: body,
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

  // Per-row Edit label / Edit columns / Delete buttons on the App
  // Builder's draggable component list — appended as an extra <td>
  // alongside the id/kind/ordinal columns already in the markup
  // (`columns: id, kind, ordinal`, see examples/app_builder.pgapp). The
  // ordinal column doubles as the `idx` the edit/delete routes expect,
  // since `meta::sync_app` always re-derives ordinal from file order on
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
          var actionsTd = document.createElement("td");

          var editLabelBtn = document.createElement("button");
          editLabelBtn.type = "button";
          editLabelBtn.className = "pgapp-btn pgapp-btn-secondary";
          editLabelBtn.textContent = "Edit label";
          editLabelBtn.addEventListener("click", function () {
            pgappPrompt("New title/content for this component:", "").then(function (label) {
              if (label === null || label === "") return;
              postComponentEdit(target, idx, "label=" + encodeURIComponent(label));
            });
          });
          actionsTd.appendChild(editLabelBtn);

          if (kind === "report" || kind === "region" || kind === "editable_table") {
            var editColsBtn = document.createElement("button");
            editColsBtn.type = "button";
            editColsBtn.className = "pgapp-btn pgapp-btn-secondary";
            editColsBtn.textContent = "Edit columns";
            editColsBtn.addEventListener("click", function () {
              pgappPrompt("New columns for this component, comma-separated:", "").then(function (columns) {
                if (columns === null || columns === "") return;
                postComponentEdit(target, idx, "columns=" + encodeURIComponent(columns));
              });
            });
            actionsTd.appendChild(editColsBtn);
          }

          var deleteBtn = document.createElement("button");
          deleteBtn.type = "button";
          deleteBtn.className = "pgapp-btn pgapp-btn-destructive";
          deleteBtn.textContent = "Delete";
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
  // text input — the App Builder's "Edit label"/"Edit columns" buttons
  // use this in place of window.prompt(). Resolves the input's value on
  // OK/Enter, or null on Cancel/Escape.
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
  } else {
    bindDynamicActions();
    bindNavToggles();
    bindMobileNavToggle();
    bindConfirmForms();
    bindDraggableRows();
    bindPreviewLink();
    bindAddComponentForm();
    bindComponentRowActions();
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
