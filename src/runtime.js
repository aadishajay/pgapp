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
  function saveDraggedOrder(tbody) {
    var params = new URLSearchParams(location.search);
    var workspace = params.get("target_workspace");
    var app = params.get("target_app");
    var page = params.get("target_page");
    if (!workspace || !app || !page) {
      console.error("pgapp: draggable rows need ?target_workspace=&target_app=&target_page= on this page's own URL");
      return;
    }
    var ids = [];
    var rows = tbody.querySelectorAll("tr");
    for (var i = 0; i < rows.length; i++) {
      var firstCell = rows[i].querySelector("td");
      if (firstCell) ids.push(firstCell.textContent.trim());
    }
    var url =
      "/" + encodeURIComponent(workspace) + "/" + encodeURIComponent(app) + "/admin/pages/" + encodeURIComponent(page) + "/reorder";
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
  } else {
    bindDynamicActions();
    bindNavToggles();
    bindMobileNavToggle();
    bindConfirmForms();
    bindDraggableRows();
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
