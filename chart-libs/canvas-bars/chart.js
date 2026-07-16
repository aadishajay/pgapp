// Example pluggable chart library ("canvas-bars"): a dependency-free
// bar-chart renderer using <canvas>, demonstrating the pgapp chart
// library plug point (see src/chart_lib.rs). Select it with
// PGAPP_CHART_LIB=canvas-bars.
//
// pgapp doesn't know anything about how this file renders — it just
// serves it at /chart-lib.js and, for each chart placeholder, embeds a
// sibling <script type="application/json" class="pgapp-chart-data">
// with the rows/x/y/type this library needs. Any real charting library
// (Chart.js, D3, ...) could replace this file with no server-side
// changes at all.
(function () {
  function renderChart(container, data) {
    var canvas = document.createElement('canvas');
    canvas.width = container.clientWidth || 480;
    canvas.height = 240;
    container.appendChild(canvas);
    var ctx = canvas.getContext('2d');

    var values = data.rows.map(function (r) { return Number(r[data.y]) || 0; });
    var labels = data.rows.map(function (r) { return String(r[data.x]); });
    var max = Math.max.apply(null, values.concat([1]));
    var pad = 24;
    var w = (canvas.width - pad * 2) / Math.max(values.length, 1);

    ctx.strokeStyle = '#888';
    ctx.beginPath();
    ctx.moveTo(pad, canvas.height - pad);
    ctx.lineTo(canvas.width - pad, canvas.height - pad);
    ctx.stroke();

    ctx.fillStyle = '#2563eb';
    ctx.font = '10px system-ui, sans-serif';
    ctx.textAlign = 'center';
    values.forEach(function (v, i) {
      var barH = (v / max) * (canvas.height - pad * 2);
      var x = pad + i * w + 4;
      var y = canvas.height - pad - barH;
      ctx.fillRect(x, y, Math.max(w - 8, 1), barH);
      ctx.fillStyle = '#333';
      ctx.fillText(labels[i], x + (w - 8) / 2, canvas.height - pad + 12);
      ctx.fillStyle = '#2563eb';
    });
  }

  function init() {
    document.querySelectorAll('.pgapp-chart').forEach(function (el) {
      var dataEl = el.querySelector('.pgapp-chart-data');
      if (!dataEl) return;
      try {
        renderChart(el, JSON.parse(dataEl.textContent));
      } catch (e) {
        el.textContent = 'chart render failed: ' + e;
      }
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
