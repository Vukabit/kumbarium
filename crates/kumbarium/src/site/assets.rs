//! The static half of a doc build: one stylesheet (the terminal
//! palette on paper, four themes) and one script (themes, copy
//! buttons, collapse toggles, the search view, keyboard
//! shortcuts). The script builds every result node with
//! textContent: nothing from the index is ever parsed as HTML.

pub(super) const JS: &str = r##"// kum doc: themes, copy, toggles, search, keys.
(function () {
  "use strict";
  var doc = document;
  var root = doc.body.getAttribute("data-root") || "";
  function $(id) { return doc.getElementById(id); }
  function each(list, fn) { Array.prototype.forEach.call(list, fn); }
  function el(tag, cls, text) {
    var n = doc.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  }

  // ---- theme ------------------------------------------------
  var THEMES = ["system", "paper", "slate", "lamp"];
  function applyTheme(t) {
    if (t === "system" || THEMES.indexOf(t) < 0) {
      doc.documentElement.removeAttribute("data-theme");
    } else {
      doc.documentElement.setAttribute("data-theme", t);
    }
  }
  function savedTheme() {
    try { return localStorage.getItem("kum-theme") || "system"; }
    catch (_) { return "system"; }
  }
  var current = savedTheme();
  each(doc.querySelectorAll("#kum-settings input[name=theme]"), function (r) {
    r.checked = r.value === current;
    r.addEventListener("change", function () {
      applyTheme(r.value);
      try { localStorage.setItem("kum-theme", r.value); } catch (_) {}
    });
  });

  // ---- popovers ---------------------------------------------
  var help = $("kum-help"), settings = $("kum-settings");
  function setPop(pop, show) {
    if (!pop) return;
    pop.hidden = !show;
    if (show) {
      var other = pop === help ? settings : help;
      if (other) other.hidden = true;
    }
  }
  function flip(pop) { if (pop) setPop(pop, pop.hidden); }
  var hb = $("kum-help-btn"), sb = $("kum-settings-btn");
  if (hb) hb.addEventListener("click", function () { flip(help); });
  if (sb) sb.addEventListener("click", function () { flip(settings); });
  doc.addEventListener("click", function (e) {
    if (!e.target.closest(".pop, .pop-btn")) {
      setPop(help, false);
      setPop(settings, false);
    }
  });

  // ---- copy buttons -----------------------------------------
  doc.addEventListener("click", function (e) {
    var b = e.target.closest("button.copy");
    if (!b) return;
    var text = b.getAttribute("data-copy") || "";
    function done() {
      b.classList.add("copied");
      setTimeout(function () { b.classList.remove("copied"); }, 1200);
    }
    function fallback() {
      var t = el("textarea");
      t.value = text;
      t.style.position = "fixed";
      t.style.opacity = "0";
      doc.body.appendChild(t);
      t.select();
      try { doc.execCommand("copy"); done(); } catch (_) {}
      doc.body.removeChild(t);
    }
    if (navigator.clipboard && window.isSecureContext) {
      navigator.clipboard.writeText(text).then(done, fallback);
    } else {
      fallback();
    }
  });

  // ---- collapse toggles -------------------------------------
  var toggleAll = $("kum-toggle-all");
  function flipAll() {
    var secs = doc.querySelectorAll("#main-content details.sec");
    var anyOpen = Array.prototype.some.call(secs, function (d) {
      return d.open;
    });
    each(secs, function (d) { d.open = !anyOpen; });
    if (toggleAll) {
      toggleAll.textContent = anyOpen ? "[+]" : "[−]";
      toggleAll.title = (anyOpen ? "Expand" : "Collapse") + " all sections (+)";
    }
  }
  if (toggleAll) toggleAll.addEventListener("click", flipAll);
  function openTarget() {
    var id = decodeURIComponent(window.location.hash.slice(1));
    var t = id && doc.getElementById(id);
    for (var n = t; n; n = n.parentElement) {
      if (n.tagName === "DETAILS") n.open = true;
    }
  }
  window.addEventListener("hashchange", openTarget);
  openTarget();

  // ---- search -----------------------------------------------
  var input = $("kum-search");
  var view = $("search-view");
  var main = $("main-content");
  var index = window.KUM_INDEX || [];
  var KINDS = {
    decision: "decision", decisions: "decision",
    state: "project_state", project_state: "project_state",
    preference: "preference", pref: "preference",
    reference: "reference", ref: "reference",
    matter: "matter", matters: "matter", task: "matter",
    shelf: "shelf"
  };
  var LABELS = ["In titles", "In content", "Shelves"];
  var groups = [[], [], []], tab = 0, sel = -1, lastQuery = "";

  function parse(q) {
    var f = { kind: null, shelf: null, tag: null, terms: [] };
    q.toLowerCase().split(/\s+/).forEach(function (w) {
      if (!w) return;
      var m = w.match(/^([a-z_]+):(.*)$/);
      if (m) {
        var key = m[1], val = m[2];
        if (key === "kind" || key === "k") {
          if (KINDS[val]) f.kind = KINDS[val];
          return;
        }
        if (key === "shelf" || key === "in") { f.shelf = val; return; }
        if (key === "tag") { f.tag = val; return; }
        if (KINDS[key]) {
          f.kind = KINDS[key];
          if (val) f.terms.push(val);
          return;
        }
      }
      if (w.charAt(0) === "#" && w.length > 1) { f.tag = w.slice(1); return; }
      f.terms.push(w);
    });
    return f;
  }

  function search(q) {
    var f = parse(q);
    groups = [[], [], []];
    if (!f.terms.length && !f.kind && !f.shelf && !f.tag) return false;
    for (var i = 0; i < index.length; i++) {
      var d = index[i];
      if (f.kind && d.k !== f.kind) continue;
      if (f.shelf && d.n.toLowerCase().indexOf(f.shelf) < 0) continue;
      if (f.tag && (" " + d.g).indexOf(" " + f.tag) < 0) continue;
      var title = d.t.toLowerCase();
      var hay = [title, d.i, d.s.toLowerCase(), d.g, d.x].join(" ");
      var ok = true, inTitle = true, score = 0;
      for (var j = 0; j < f.terms.length; j++) {
        var t = f.terms[j];
        if (hay.indexOf(t) < 0) { ok = false; break; }
        var at = title.indexOf(t);
        if (at < 0 && d.i.indexOf(t) !== 0) inTitle = false;
        else score += at === 0 ? 3 : 2;
        if (d.i === t) score += 20;
      }
      if (!ok) continue;
      var g = d.k === "shelf" ? 2 : (inTitle ? 0 : 1);
      groups[g].push([score, d]);
    }
    groups.forEach(function (list) {
      list.sort(function (a, b) {
        return b[0] - a[0] || (a[1].t < b[1].t ? -1 : a[1].t > b[1].t ? 1 : 0);
      });
    });
    return true;
  }

  function draw(q) {
    while (view.firstChild) view.removeChild(view.firstChild);
    view.appendChild(el("h1", "search-title", "Results for “" + q + "”"));
    var tabs = el("div", "tabs");
    LABELS.forEach(function (label, k) {
      var cls = "tab" + (k === tab ? " on" : "");
      var b = el("button", cls, label + " (" + groups[k].length + ")");
      b.type = "button";
      b.addEventListener("click", function () { tab = k; sel = -1; draw(q); });
      tabs.appendChild(b);
    });
    view.appendChild(tabs);
    var list = el("div", "results");
    groups[tab].slice(0, 200).forEach(function (h) {
      var d = h[1];
      var a = el("a", "result");
      a.href = root + d.u;
      var top = el("div", "result-top");
      var kind = d.k === "project_state" ? "state" : d.k;
      top.appendChild(el("span", "kind", kind));
      if (d.i) top.appendChild(el("span", "id", d.i));
      top.appendChild(el("span", "title", d.t));
      top.appendChild(el("span", "where", d.k === "shelf" ? "" : d.n));
      a.appendChild(top);
      if (d.s) a.appendChild(el("div", "summary", d.s));
      list.appendChild(a);
    });
    if (!groups[tab].length) {
      list.appendChild(el("p", "none", "Nothing on the shelves matches."));
    }
    view.appendChild(list);
    var hint =
      "↑ ↓ move · Enter opens · ← → switch tabs · Esc goes back";
    view.appendChild(el("p", "hint", hint));
  }

  function show(q) {
    if (!view || !main) return;
    q = q.trim();
    if (!search(q)) {
      view.hidden = true;
      main.hidden = false;
      return;
    }
    if (q !== lastQuery) { tab = 0; sel = -1; }
    lastQuery = q;
    if (!groups[tab].length) {
      for (var k = 0; k < 3; k++) { if (groups[k].length) { tab = k; break; } }
    }
    main.hidden = true;
    view.hidden = false;
    draw(q);
  }

  function move(delta) {
    var items = view.querySelectorAll("a.result");
    if (!items.length) return;
    sel = Math.max(0, Math.min(items.length - 1, sel + delta));
    each(items, function (a, i) { a.classList.toggle("sel", i === sel); });
    items[sel].scrollIntoView({ block: "nearest" });
  }

  function setUrl(q) {
    try {
      var u = new URL(window.location.href);
      if (q) u.searchParams.set("search", q);
      else u.searchParams.delete("search");
      history.replaceState(null, "", u.href);
    } catch (_) {}
  }

  if (input && view && main) {
    input.addEventListener("input", function () {
      show(input.value);
      setUrl(input.value.trim());
    });
    input.addEventListener("keydown", function (e) {
      if (view.hidden) return;
      if (e.key === "ArrowDown") { e.preventDefault(); move(1); }
      else if (e.key === "ArrowUp") { e.preventDefault(); move(-1); }
      else if (e.key === "Enter") {
        var items = view.querySelectorAll("a.result");
        var t = items[sel < 0 ? 0 : sel];
        if (t) window.location.href = t.href;
      } else if (
        (e.key === "ArrowRight" || e.key === "ArrowLeft") && sel >= 0
      ) {
        e.preventDefault();
        tab = (tab + (e.key === "ArrowRight" ? 1 : 2)) % 3;
        sel = -1;
        draw(input.value.trim());
      }
    });
    var m = window.location.search.match(/[?&]search=([^&]*)/);
    if (m) {
      try {
        input.value = decodeURIComponent(m[1].replace(/\+/g, " "));
      } catch (_) {}
      show(input.value);
    }
  }

  // ---- keys -------------------------------------------------
  doc.addEventListener("keydown", function (e) {
    var typing = e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName);
    if (e.key === "Escape") {
      setPop(help, false);
      setPop(settings, false);
      if (input && (input.value || (view && !view.hidden))) {
        input.value = "";
        show("");
        setUrl("");
      }
      if (input) input.blur();
      return;
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
    if ((e.key === "/" || e.key === "s" || e.key === "S") && input) {
      e.preventDefault();
      input.focus();
      input.select();
    } else if (e.key === "?") {
      e.preventDefault();
      flip(help);
    } else if (e.key === "+") {
      e.preventDefault();
      flipAll();
    }
  });
})();
"##;

pub(super) const CSS: &str = r##"/* kum doc: the terminal palette on paper,
   in four themes. */
:root, :root[data-theme="paper"] {
  --bg: #fbfaf7;
  --panel: #f2efe8;
  --raise: #ffffff;
  --ink: #23211d;
  --dim: #77716a;
  --rule: #e2ddd2;
  --accent: #8a5a2b;
  --cyan: #0f7c86;
  --green: #2f7d32;
  --yellow: #946400;
  --red: #b3261e;
  --magenta: #8e3a8e;
  --add: #e6f2e2;
  --del: #f8e3e0;
  --shadow: rgba(40, 30, 10, .12);
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme]) {
    --bg: #17161a;
    --panel: #201f24;
    --raise: #26252b;
    --ink: #e7e3da;
    --dim: #958e82;
    --rule: #34323a;
    --accent: #d9a066;
    --cyan: #56c2cc;
    --green: #7cc47f;
    --yellow: #e0b44c;
    --red: #ef7a72;
    --magenta: #d18ad1;
    --add: #1f3322;
    --del: #3a2220;
    --shadow: rgba(0, 0, 0, .4);
    color-scheme: dark;
  }
}
:root[data-theme="slate"] {
  --bg: #17161a;
  --panel: #201f24;
  --raise: #26252b;
  --ink: #e7e3da;
  --dim: #958e82;
  --rule: #34323a;
  --accent: #d9a066;
  --cyan: #56c2cc;
  --green: #7cc47f;
  --yellow: #e0b44c;
  --red: #ef7a72;
  --magenta: #d18ad1;
  --add: #1f3322;
  --del: #3a2220;
  --shadow: rgba(0, 0, 0, .4);
  color-scheme: dark;
}
:root[data-theme="lamp"] {
  --bg: #0f1419;
  --panel: #151b22;
  --raise: #1a222b;
  --ink: #d9d4c7;
  --dim: #8a8f95;
  --rule: #26303a;
  --accent: #ffb454;
  --cyan: #59c2ff;
  --green: #aad94c;
  --yellow: #e6b450;
  --red: #f07178;
  --magenta: #d2a6ff;
  --add: #1c2b1a;
  --del: #33191c;
  --shadow: rgba(0, 0, 0, .5);
  color-scheme: dark;
}
:root {
  --mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif;
  --top: 54px;
}
* { box-sizing: border-box; }
html, body { margin: 0; }
html { scroll-padding-top: calc(var(--top) + 12px); }
body { background: var(--bg); color: var(--ink); font: 15px/1.6 var(--sans); }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
code, pre, kbd, .id, .brand, .tree, .tag, .kind, .conf, .sev, .badge, .rel,
.crumbs, .oob button, .src {
  font-family: var(--mono);
}
code {
  font-size: .9em;
  background: var(--panel);
  padding: 1px 5px;
  border-radius: 4px;
}
h1 code, h2 code, a.ref code { background: none; padding: 0; }
kbd {
  font-size: 12px; padding: 1px 6px; border-radius: 4px;
  border: 1px solid var(--rule);
  border-bottom-width: 2px;
  background: var(--raise);
}
button { font: inherit; color: inherit; }

/* ---- header ---- */
.top {
  position: sticky; top: 0; z-index: 10;
  display: flex; align-items: center; gap: 18px;
  height: var(--top); padding: 0 20px;
  background: var(--panel); border-bottom: 1px solid var(--rule);
}
.brand { font-weight: 700; font-size: 17px; color: var(--ink); }
.brand:hover { text-decoration: none; }
.brand span { color: var(--dim); font-weight: 400; }
.search { flex: 1; max-width: 680px; }
#kum-search {
  width: 100%; padding: 7px 12px; font: 14px var(--mono);
  color: var(--ink); background: var(--bg);
  border: 1px solid var(--rule); border-radius: 6px;
}
#kum-search:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
.tools {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
  font-size: 14px;
}
.tools a { color: var(--dim); padding: 4px 8px; }
.pop-btn {
  display: inline-grid; place-items: center; width: 30px; height: 30px;
  background: var(--bg);
  border: 1px solid var(--rule);
  border-radius: 6px;
  cursor: pointer;
  color: var(--dim); font-weight: 700;
}
.pop-btn:hover { color: var(--ink); }
.pop {
  position: absolute; right: 16px; top: calc(var(--top) - 4px); z-index: 20;
  width: min(420px, calc(100vw - 32px)); max-height: 75vh; overflow-y: auto;
  padding: 6px 18px 14px; background: var(--raise);
  border: 1px solid var(--rule);
  border-radius: 8px;
  box-shadow: 0 10px 30px var(--shadow);
  font-size: 14px;
}
.pop h3 {
  font-size: 12px;
  letter-spacing: .08em;
  text-transform: uppercase;
  color: var(--dim);
  margin: 14px 0 8px;
}
.keys {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 6px 14px;
  margin: 0;
}
.keys dt { white-space: nowrap; }
.keys dd { margin: 0; color: var(--dim); }
.themes { display: flex; flex-wrap: wrap; gap: 6px 16px; }
.themes label { cursor: pointer; font-family: var(--mono); }

/* ---- frame ---- */
.frame { display: flex; align-items: flex-start; }
.side {
  position: sticky; top: var(--top);
  flex: 0 0 260px; max-height: calc(100vh - var(--top)); overflow-y: auto;
  padding: 16px 12px 32px 20px; font-size: 13px;
  border-right: 1px solid var(--rule);
}
.side-block { margin-bottom: 20px; }
.side h2 {
  margin: 0 0 6px;
  font-size: 11px;
  letter-spacing: .08em;
  text-transform: uppercase;
  color: var(--dim); font-weight: 600;
}
.side h2 a {
  color: var(--dim);
  text-transform: none;
  letter-spacing: 0;
  font-family: var(--mono);
  font-size: 12px;
}
.side h3 {
  margin: 10px 0 2px;
  font-size: 12px;
  color: var(--dim);
  font-weight: 600;
}
.side ul { list-style: none; margin: 0; padding: 0; }
.side li {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  padding: 2px 0;
}
.side li a {
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.side li a.here { color: var(--accent); font-weight: 700; }
.tree li { padding-left: calc(var(--depth) * 14px); font-family: var(--mono); }
.tree li.group { color: var(--dim); padding-top: 8px; }
.count { color: var(--dim); font-weight: 400; font-size: .85em; }
main { flex: 1; min-width: 0; padding: 20px 36px 72px; }
#main-content, #search-view { max-width: 960px; }

/* ---- page head ---- */
.crumbs { font-size: 13px; color: var(--dim); margin: 0 0 2px; }
.title-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
}
h1 {
  font-size: 26px;
  line-height: 1.25;
  margin: 2px 0 8px;
  overflow-wrap: anywhere;
}
.oob {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  font-size: 13px;
}
.oob button#kum-toggle-all {
  background: none;
  border: 0;
  color: var(--dim);
  cursor: pointer;
  padding: 2px 4px;
}
.oob button#kum-toggle-all:hover { color: var(--ink); }
.src { font-size: 13px; }
button.copy {
  display: inline-grid; place-items: center; width: 26px; height: 24px;
  background: none; border: 1px solid var(--rule); border-radius: 5px;
  color: var(--dim); cursor: pointer;
}
button.copy:hover { color: var(--ink); }
button.copy.copied { color: var(--green); border-color: var(--green); }
.lede { color: var(--dim); margin: 0 0 14px; }
.meta {
  color: var(--dim);
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  align-items: center;
  margin: 4px 0 12px;
}
.note { background: var(--panel); padding: 8px 12px; border-radius: 6px; }
.dim { color: var(--dim); }

/* ---- banners ---- */
.banner {
  margin: 12px 0; padding: 10px 14px; border-radius: 6px;
  border: 1px solid var(--rule);
  border-left-width: 4px;
  background: var(--panel);
}
.banner ul { margin: 6px 0 0; padding-left: 18px; }
.banner.superseded { border-left-color: var(--yellow); }
.banner.retired { border-left-color: var(--dim); }
.banner.disputed { border-left-color: var(--red); }
.banner.disputed b { color: var(--red); }

/* ---- sections ---- */
details.sec { margin-top: 26px; }
details.sec > summary { list-style: none; cursor: pointer; }
details.sec > summary::-webkit-details-marker { display: none; }
details.sec > summary h2 {
  display: flex; align-items: baseline; gap: 8px;
  font-size: 18px; margin: 0 0 10px; padding-bottom: 6px;
  border-bottom: 1px solid var(--rule);
}
details.sec > summary h2::before {
  content: "\2212"; width: 14px; color: var(--dim); font: 14px var(--mono);
}
details.sec:not([open]) > summary h2::before { content: "+"; }
details.sec:not([open]) > summary h2 { margin-bottom: 0; }
.anchor {
  visibility: hidden;
  color: var(--dim);
  font-weight: 400;
  margin-left: 2px;
}
summary h2:hover .anchor { visibility: visible; }
h3.sub { font-size: 14px; color: var(--dim); margin: 14px 0 4px; }

/* ---- listings ---- */
.id { color: var(--cyan); font-size: .9em; }
.kind { color: var(--magenta); font-size: .85em; }
.conf {
  font-size: .78em;
  padding: 0 5px;
  border-radius: 4px;
  border: 1px solid currentColor;
}
.conf.hi { color: var(--green); }
.conf.mid { color: var(--yellow); }
.conf.lo { color: var(--red); }
.badge {
  font-size: .74em;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--panel);
  color: var(--dim);
  border: 1px solid var(--rule);
}
.badge.now { color: var(--green); border-color: var(--green); }
.tags { display: inline-flex; flex-wrap: wrap; gap: 2px 8px; }
.tag { font-size: .78em; color: var(--dim); }
.tag::before { content: "#"; }
.sev {
  font-size: .78em;
  padding: 0 6px;
  border-radius: 4px;
  background: var(--panel);
  border: 1px solid var(--rule);
}
.sev.urgent { color: var(--red); border-color: var(--red); }
.sev.high { color: var(--yellow); }
.sev.low { color: var(--dim); }
.rel { color: var(--dim); font-size: .85em; margin-right: 6px; }
.rel.warn { color: var(--red); font-weight: 700; }
.external { color: var(--dim); font-style: italic; font-size: .85em; }
.table-wrap { overflow-x: auto; }
table.items { width: 100%; border-collapse: collapse; }
table.items td {
  padding: 8px 8px;
  border-bottom: 1px solid var(--rule);
  vertical-align: top;
}
td.item-id { width: 1%; white-space: nowrap; padding-top: 10px !important; }
.item-title { font-weight: 600; color: var(--ink); }
.item-title:hover { color: var(--accent); }
.summary { color: var(--dim); font-size: 14px; margin-top: 2px; }
td.item-conf { width: 1%; text-align: right; }
table.list { width: 100%; border-collapse: collapse; font-size: 14px; }
table.list th {
  text-align: left;
  font-size: 11px;
  letter-spacing: .08em;
  text-transform: uppercase;
  color: var(--dim);
  font-weight: 600;
  border-bottom: 1px solid var(--rule);
  padding: 6px 8px;
}
table.list td {
  padding: 6px 8px;
  border-bottom: 1px solid var(--rule);
  vertical-align: top;
}
table.list .n { text-align: right; font-variant-numeric: tabular-nums; }
td.tree { padding-left: calc(8px + var(--depth) * 16px); white-space: nowrap; }
ul.compact, ul.matters, ul.edges { list-style: none; padding: 0; margin: 0; }
ul.compact li, ul.matters li, ul.edges li {
  padding: 5px 0;
  border-bottom: 1px dashed var(--rule);
  overflow-wrap: anywhere;
}
ol.chain { margin: 0; padding-left: 22px; }
ol.chain li { padding: 3px 0; }
ol.chain li.here { font-weight: 600; }
dl.about {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 5px 18px;
  margin: 0;
  font-size: 14px;
}
dl.about dt { color: var(--dim); }
dl.about dd { margin: 0; overflow-wrap: anywhere; }

/* ---- rendered content ---- */
.docblock { overflow-wrap: anywhere; }
.docblock > :first-child { margin-top: 0; }
.docblock p, .docblock ul, .docblock ol, .docblock blockquote, .docblock pre {
  margin: 0 0 12px;
}
.docblock ul, .docblock ol { padding-left: 24px; }
.docblock li { margin: 3px 0; }
.docblock h3, .docblock h4, .docblock h5, .docblock h6 {
  margin: 20px 0 8px;
  font-size: 16px;
}
.docblock h5, .docblock h6 { font-size: 14px; color: var(--dim); }
.docblock blockquote {
  border-left: 3px solid var(--rule);
  padding: 2px 14px;
  color: var(--dim);
}
.docblock hr { border: 0; border-top: 1px solid var(--rule); margin: 18px 0; }
pre.code, pre.diff {
  background: var(--panel); border: 1px solid var(--rule); border-radius: 6px;
  padding: 12px 14px; overflow-x: auto; font-size: 13px; line-height: 1.5;
}
pre.code code { background: none; padding: 0; }
table.md { border-collapse: collapse; margin: 0 0 12px; font-size: 14px; }
table.md th, table.md td {
  border: 1px solid var(--rule);
  padding: 5px 10px;
  text-align: left;
}
table.md th { background: var(--panel); }
a.ref, a.wiki { border-bottom: 1px dotted currentColor; }
a.ref:hover, a.wiki:hover { text-decoration: none; border-bottom-style: solid; }
.wiki.missing {
  color: var(--dim);
  border-bottom: 1px dashed var(--dim);
  cursor: help;
}
.check { color: var(--dim); }
.check.done { color: var(--green); }
details.fold { margin: 6px 0 4px; }
details.fold > summary { cursor: pointer; color: var(--dim); font-size: 14px; }
pre.diff span { display: block; white-space: pre-wrap; }
pre.diff .add { background: var(--add); }
pre.diff .del { background: var(--del); }
pre.diff .same { color: var(--dim); }

/* ---- search view ---- */
.search-title { font-size: 22px; }
.tabs {
  display: flex;
  gap: 4px;
  border-bottom: 1px solid var(--rule);
  margin: 10px 0 8px;
  flex-wrap: wrap;
}
.tab {
  background: none; border: 0; border-bottom: 2px solid transparent;
  padding: 6px 12px; color: var(--dim); cursor: pointer; font-size: 14px;
}
.tab.on {
  color: var(--ink);
  border-bottom-color: var(--accent);
  font-weight: 600;
}
.results { display: flex; flex-direction: column; }
a.result {
  display: block;
  padding: 8px 10px;
  color: var(--ink);
  border-radius: 6px;
}
a.result:hover, a.result.sel {
  background: var(--panel);
  text-decoration: none;
}
a.result.sel { outline: 1px solid var(--accent); }
.result-top { display: flex; align-items: baseline; gap: 10px; }
.result-top .title {
  flex: 1;
  min-width: 0;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.result-top .where {
  color: var(--dim);
  font-size: 13px;
  font-family: var(--mono);
}
.results .none, .hint { color: var(--dim); }
.hint { font-size: 13px; margin-top: 14px; }

/* ---- narrow screens ---- */
@media (max-width: 800px) {
  .top { gap: 10px; padding: 0 12px; }
  .brand span, .tools a { display: none; }
  .frame { flex-direction: column; }
  main { order: 1; width: 100%; padding: 16px 16px 40px; }
  .side {
    order: 2; position: static; width: 100%; max-height: none; flex-basis: auto;
    border-right: 0; border-top: 1px solid var(--rule); padding: 16px;
  }
  .side .siblings { display: none; }
  .title-row { flex-wrap: wrap; }
  table.list th:nth-child(2), table.list td:nth-child(2) { display: none; }
  dl.about { grid-template-columns: 1fr; gap: 0; }
  dl.about dd { margin-bottom: 8px; }
  .result-top .where { display: none; }
}
"##;
