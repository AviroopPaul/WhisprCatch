/* Hero ribbon: messy speech flows into the Catcher pill, clean text flows out.
   Pure SVG + rAF. Geometry is rebuilt from the stage size, so type stays at its
   real pixel size at every width. Styles live in site.css (.ribbon). */
(function () {
  "use strict";

  var svg = document.getElementById("ribbon-svg");
  if (!svg) return;
  var stage = svg.parentNode;
  var NS = "http://www.w3.org/2000/svg";
  var XL = "http://www.w3.org/1999/xlink";

  var RAW = "so um I was thinking we could, like, move the sync to Thursday, no wait, Friday, and uh send the notes to Priya before lunch";
  var CLEAN = "Let's move the sync to Friday and send the notes to Priya before lunch.";
  var GAP_RAW = "      ";
  var GAP_CLEAN = "     ";

  var SPEED = 58;              // px per second, both strands
  var PILL_W = 124, PILL_H = 44;
  var BARS = 13, BAR_W = 3, BAR_GAP = 3.4;

  var reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  function el(name, attrs, parent) {
    var n = document.createElementNS(NS, name);
    for (var k in attrs) n.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(n);
    return n;
  }

  /* seeded PRNG so the waveform is smooth and reproducible */
  function mulberry(a) {
    return function () {
      a |= 0; a = (a + 0x6d2b79f5) | 0;
      var t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  var rnd = mulberry(20260807);
  var voices = [];
  for (var i = 0; i < BARS; i++) {
    voices.push({
      f1: 1.3 + rnd() * 2.2, p1: rnd() * 6.28,
      f2: 3.1 + rnd() * 3.4, p2: rnd() * 6.28,
      w: 0.55 + rnd() * 0.45
    });
  }
  function level(i, t) {
    // slow "phrase" envelope gives the pauses between words
    var env = 0.5 + 0.5 * Math.sin(t * 1.7) * Math.sin(t * 0.63 + 1.1);
    env = 0.28 + 0.72 * Math.max(0, env);
    var v = voices[i];
    var m = 0.5 + 0.5 * (0.6 * Math.sin(t * v.f1 + v.p1) + 0.4 * Math.sin(t * v.f2 + v.p2));
    var shape = 1 - Math.pow((i - (BARS - 1) / 2) / (BARS / 2 + 1), 2) * 0.7;
    return Math.max(0.08, Math.min(1, m * v.w * env * shape * 1.25));
  }

  /* Catmull-Rom through points -> cubic bezier path string */
  function smooth(pts) {
    var p = [pts[0]].concat(pts, [pts[pts.length - 1]]);
    var d = "M" + p[1][0].toFixed(1) + " " + p[1][1].toFixed(1);
    for (var i = 1; i < p.length - 2; i++) {
      var a = p[i - 1], b = p[i], c = p[i + 1], e = p[i + 2];
      d += "C" + (b[0] + (c[0] - a[0]) / 6).toFixed(1) + " " + (b[1] + (c[1] - a[1]) / 6).toFixed(1) +
           " " + (c[0] - (e[0] - b[0]) / 6).toFixed(1) + " " + (c[1] - (e[1] - b[1]) / 6).toFixed(1) +
           " " + c[0].toFixed(1) + " " + c[1].toFixed(1);
    }
    return d;
  }

  var S = null;   // current scene

  function build() {
    var W = Math.round(stage.clientWidth), H = Math.round(stage.clientHeight);
    if (!W || !H) return;
    while (svg.firstChild) svg.removeChild(svg.firstChild);
    svg.setAttribute("viewBox", "0 0 " + W + " " + H);

    var narrow = W < 560;
    var cx = W / 2, cy = H - 46;
    var R = Math.min(H * 0.22, W * 0.14);
    var Lx = Math.max(cx - Math.min(W * 0.26, 340), 1.45 * R + 8);
    var Ly = 12 + 1.15 * R;
    var pxL = cx - PILL_W / 2 + 8, pxR = cx + PILL_W / 2 - 8;

    // raw speech: enters low on the left, curls once, drops into the pill
    var u = function (x, y) { return [Lx + x * R, Ly + y * R]; };
    var raw = [
      [Math.min(Lx - 5 * R, -80), Ly + 1.55 * R],
      u(-3.0, 1.75), u(-1.2, 1.72), u(0.5, 1.3),
      u(1.3, 0.2), u(0.65, -1.0), u(-0.6, -1.15), u(-1.4, -0.2),
      u(-1.05, 0.95), u(-0.35, 1.5), u(0.7, 2.0),
      [pxL, cy]
    ];
    var dRaw = smooth(raw);

    // clean text: leaves the pill on a thick band that climbs to the right edge
    var y1 = cy - H * 0.36, x1 = W + 90;
    var dBand = "M" + pxR + " " + cy +
      "C" + (pxR + (x1 - pxR) * 0.38) + " " + cy + " " + (pxR + (x1 - pxR) * 0.62) + " " + (y1 + (cy - y1) * 0.1) +
      " " + x1 + " " + y1;

    var defs = el("defs", {}, svg);
    var g = el("radialGradient", { id: "rib-glow" }, defs);
    el("stop", { offset: "0", style: "stop-color:var(--accent);stop-opacity:.38" }, g);
    el("stop", { offset: "1", style: "stop-color:var(--accent);stop-opacity:0" }, g);

    el("ellipse", { cx: cx, cy: cy, rx: 180, ry: 62, fill: "url(#rib-glow)" }, svg);

    var pathRaw = el("path", { id: "rib-raw", d: dRaw, "class": "guide" }, svg);
    var tRaw = el("text", { "class": "raw" }, svg);
    var tpRaw = el("textPath", { startOffset: 0 }, tRaw);
    tpRaw.setAttributeNS(XL, "href", "#rib-raw");
    tpRaw.setAttribute("href", "#rib-raw");

    var bandW = narrow ? 34 : 42;
    var pathBand = el("path", {
      id: "rib-band", d: dBand, "class": "rib-band", fill: "none",
      "stroke-width": bandW, "stroke-linecap": "round"
    }, svg);
    var tClean = el("text", { "class": "clean", dy: ".34em" }, svg);
    var tpClean = el("textPath", { startOffset: 0 }, tClean);
    tpClean.setAttributeNS(XL, "href", "#rib-band");
    tpClean.setAttribute("href", "#rib-band");

    // pill, above both strands
    var pill = el("g", { transform: "translate(" + cx + " " + cy + ")" }, svg);
    el("rect", {
      x: -PILL_W / 2, y: -PILL_H / 2, width: PILL_W, height: PILL_H, rx: PILL_H / 2, "class": "pill-body"
    }, pill);
    var bars = [];
    var total = BARS * BAR_W + (BARS - 1) * BAR_GAP;
    for (var i = 0; i < BARS; i++) {
      bars.push(el("rect", {
        x: -total / 2 + i * (BAR_W + BAR_GAP), width: BAR_W, rx: BAR_W / 2, "class": "bar"
      }, pill));
    }

    // fill each strand with whole repeats of its sentence, measured in place
    function fill(tp, text, gap, textEl, pathEl) {
      tp.textContent = text + gap;
      var T = textEl.getComputedTextLength();
      var L = pathEl.getTotalLength();
      var n = Math.ceil(L / T) + 2, s = "";
      for (var k = 0; k < n; k++) s += text + gap;
      tp.textContent = s;
      return T;
    }
    var Tr = fill(tpRaw, RAW, GAP_RAW, tRaw, pathRaw);
    var Tc = fill(tpClean, CLEAN, GAP_CLEAN, tClean, pathBand);

    S = { tpRaw: tpRaw, tpClean: tpClean, Tr: Tr, Tc: Tc, bars: bars, total: total };
  }

  function render(t) {
    if (!S) return;
    var d = t * SPEED;
    // text moves along its path: raw toward the pill, clean away from it
    S.tpRaw.setAttribute("startOffset", (-S.Tr + (d % S.Tr)).toFixed(1));
    S.tpClean.setAttribute("startOffset", (-S.Tc + (d % S.Tc)).toFixed(1));
    var maxH = PILL_H - 12;
    for (var i = 0; i < S.bars.length; i++) {
      var h = 3 + (maxH - 3) * level(i, t);
      S.bars[i].setAttribute("y", (-h / 2).toFixed(2));
      S.bars[i].setAttribute("height", h.toFixed(2));
    }
  }

  /* loop control */
  var raf = 0, t = 0, last = 0, onscreen = true;
  function frame(now) {
    raf = 0;
    if (!onscreen || document.hidden) return;
    t += Math.min(0.05, (now - last) / 1000);
    last = now;
    render(t);
    raf = requestAnimationFrame(frame);
  }
  function start() {
    if (reduced || raf || !onscreen || document.hidden) return;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  }

  var built = false;
  function rebuild() {
    build();
    render(reduced ? 2.4 : t);
    built = true;
  }

  function init() {
    rebuild();
    if (window.ResizeObserver) {
      var pending = 0;
      new ResizeObserver(function () {
        if (pending) return;
        pending = requestAnimationFrame(function () { pending = 0; rebuild(); });
      }).observe(stage);
    }
    if (reduced) return;
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (es) {
        onscreen = es[0].isIntersecting;
        if (onscreen) start();
      }).observe(stage);
    }
    document.addEventListener("visibilitychange", function () { if (!document.hidden) start(); });
    start();
  }

  // measure after the web fonts have loaded, or the loop period is wrong
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(init);
  else init();
})();
