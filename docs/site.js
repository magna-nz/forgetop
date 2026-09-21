/* forgetop docs: syntax highlighting, copy buttons, the Terminal/Dashboard
   surface switch, the mobile nav, and active-section tracking in the sidebar.
   Everything here is progressive enhancement. With JavaScript off the page is
   still complete: code is plain text, the nav still links to every anchor, and
   the dashboard copy is the one that shows. */
(function () {
  'use strict';

  /* ---------------- syntax highlighting ---------------- */

  var RULES = {
    bash: [
      { re: /#[^\n]*/y, cls: 'tok-comment' },
      { re: /"(?:[^"\\\n]|\\.)*"/y, cls: 'tok-string' },
      { re: /'[^'\n]*'/y, cls: 'tok-string' },
      { re: /\b(?:forgetop|cargo|export|doctor|clippy|test|run)\b/y, cls: 'tok-keyword' },
      { re: /--?[A-Za-z][\w-]*/y, cls: 'tok-type' },
      { re: /\b\d[\d_.]*\b/y, cls: 'tok-number' }
    ],
    json: [
      { re: /"(?:[^"\\\n]|\\.)*"(?=\s*:)/y, cls: 'tok-type' },
      { re: /"(?:[^"\\\n]|\\.)*"/y, cls: 'tok-string' },
      { re: /\b(?:true|false|null)\b/y, cls: 'tok-keyword' },
      { re: /-?\b\d+(?:\.\d+)?\b/y, cls: 'tok-number' }
    ]
  };
  RULES.sh = RULES.bash;
  RULES.shell = RULES.bash;

  function escapeHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  function highlight(text, rules) {
    var out = '';
    var plain = '';
    var i = 0;
    while (i < text.length) {
      var matched = false;
      for (var r = 0; r < rules.length; r++) {
        var rule = rules[r];
        rule.re.lastIndex = i;
        var m = rule.re.exec(text);
        if (m && m[0].length > 0) {
          if (plain) { out += escapeHtml(plain); plain = ''; }
          out += '<span class="' + rule.cls + '">' + escapeHtml(m[0]) + '</span>';
          i += m[0].length;
          matched = true;
          break;
        }
      }
      if (!matched) { plain += text[i]; i++; }
    }
    if (plain) { out += escapeHtml(plain); }
    return out;
  }

  function languageOf(codeEl) {
    var cls = codeEl.className || '';
    var m = cls.match(/language-([\w-]+)/);
    return m ? m[1] : 'text';
  }

  var codes = document.querySelectorAll('.codeblock pre > code');
  for (var c = 0; c < codes.length; c++) {
    var rules = RULES[languageOf(codes[c])];
    if (!rules) { continue; }
    try {
      codes[c].innerHTML = highlight(codes[c].textContent, rules);
    } catch (err) {
      /* leave the plain text alone */
    }
  }

  /* ---------------- copy buttons ---------------- */

  var blocks = document.querySelectorAll('.codeblock');
  for (var b = 0; b < blocks.length; b++) {
    (function (block) {
      var head = block.querySelector('.cb-head');
      var pre = block.querySelector('pre');
      if (!head || !pre) { return; }
      var button = document.createElement('button');
      button.type = 'button';
      button.className = 'copy';
      button.textContent = 'Copy';
      button.setAttribute('aria-label', 'Copy code to clipboard');
      button.addEventListener('click', function () {
        var text = pre.textContent;
        var done = function () {
          button.textContent = 'Copied';
          window.setTimeout(function () { button.textContent = 'Copy'; }, 1400);
        };
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(text).then(done, function () { button.textContent = 'Press Ctrl+C'; });
        } else {
          var ta = document.createElement('textarea');
          ta.value = text;
          document.body.appendChild(ta);
          ta.select();
          try { document.execCommand('copy'); done(); } catch (e) { button.textContent = 'Press Ctrl+C'; }
          document.body.removeChild(ta);
        }
      });
      head.appendChild(button);
    })(blocks[b]);
  }

  /* ---------------- terminal / dashboard switch ----------------
     The choice applies to the whole page, so a reader picks their surface once
     and every section follows. The initial value is set by a tiny inline script
     in the head, before first paint, so the page never flashes the wrong copy. */

  var SURFACE_KEY = 'forgetop-docs-surface';
  var root = document.documentElement;

  document.addEventListener('click', function (e) {
    var btn = e.target.closest ? e.target.closest('.st-btn') : null;
    if (!btn) { return; }
    var next = btn.getAttribute('data-surface-set');
    if (next !== 'tui' && next !== 'dash') { return; }

    /* Keep the clicked toggle where it is on screen. Switching surfaces changes
       how tall the section is, so without this the page jumps under the cursor. */
    var group = btn.closest('.surface-toggle');
    var before = group ? group.getBoundingClientRect().top : null;

    root.setAttribute('data-surface', next);
    try { localStorage.setItem(SURFACE_KEY, next); } catch (err) { /* private mode */ }

    if (before !== null) {
      var after = group.getBoundingClientRect().top;
      if (after !== before) { window.scrollBy(0, after - before); }
    }
  });

  /* Reflect the current surface for assistive tech. */
  function syncSurfaceButtons() {
    var current = root.getAttribute('data-surface') === 'tui' ? 'tui' : 'dash';
    var btns = document.querySelectorAll('.st-btn');
    for (var i = 0; i < btns.length; i++) {
      btns[i].setAttribute('aria-pressed', String(btns[i].getAttribute('data-surface-set') === current));
    }
  }
  syncSurfaceButtons();
  new MutationObserver(syncSurfaceButtons).observe(root, { attributes: true, attributeFilter: ['data-surface'] });

  /* ---------------- mobile navigation ---------------- */

  var navToggle = document.querySelector('.nav-toggle');
  var sidebar = document.querySelector('.sidebar');
  var NARROW = '(max-width: 900px)';

  function applyNarrow() {
    if (!navToggle || !sidebar) { return; }
    var narrow = window.matchMedia(NARROW).matches;
    /* Wide screens always show the nav; narrow ones start collapsed. */
    sidebar.hidden = narrow;
    navToggle.setAttribute('aria-expanded', String(!narrow));
  }
  applyNarrow();
  window.matchMedia(NARROW).addListener(applyNarrow);

  if (navToggle && sidebar) {
    navToggle.addEventListener('click', function () {
      sidebar.hidden = !sidebar.hidden;
      navToggle.setAttribute('aria-expanded', String(!sidebar.hidden));
    });
    /* Picking a destination closes the menu again. */
    sidebar.addEventListener('click', function (e) {
      if (e.target.tagName === 'A' && window.matchMedia(NARROW).matches) {
        sidebar.hidden = true;
        navToggle.setAttribute('aria-expanded', 'false');
      }
    });
  }

  /* ---------------- active section in the nav ---------------- */

  var links = {};
  var navAnchors = document.querySelectorAll('.sidebar a[href^="#"]');
  for (var n = 0; n < navAnchors.length; n++) {
    links[navAnchors[n].getAttribute('href').slice(1)] = navAnchors[n];
  }

  function setActive(id) {
    for (var key in links) {
      if (Object.prototype.hasOwnProperty.call(links, key)) {
        links[key].classList.toggle('active', key === id);
      }
    }
  }

  var targets = document.querySelectorAll('.section, .section h3[id]');
  if ('IntersectionObserver' in window && targets.length) {
    var visible = {};
    var observer = new IntersectionObserver(function (entries) {
      for (var e = 0; e < entries.length; e++) {
        var id = entries[e].target.id;
        if (entries[e].isIntersecting) { visible[id] = true; }
        else { delete visible[id]; }
      }
      var best = null;
      var bestTop = Infinity;
      for (var key in visible) {
        if (!Object.prototype.hasOwnProperty.call(visible, key)) { continue; }
        if (!links[key]) { continue; }
        var el = document.getElementById(key);
        if (!el) { continue; }
        var top = el.getBoundingClientRect().top;
        if (top < bestTop) { bestTop = top; best = key; }
      }
      if (best) { setActive(best); }
    }, { rootMargin: '-60px 0px -55% 0px', threshold: 0 });

    for (var t = 0; t < targets.length; t++) { observer.observe(targets[t]); }
  }

  if (window.location.hash && links[window.location.hash.slice(1)]) {
    setActive(window.location.hash.slice(1));
  }
})();
