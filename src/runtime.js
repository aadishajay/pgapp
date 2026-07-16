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

  return { getItem: getItem, setItem: setItem };
})();
