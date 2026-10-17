// omacus.com

(() => {
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Header gets a backdrop once the page scrolls.
  const header = $(".site-header");
  const onScroll = () => header?.classList.toggle("scrolled", scrollY > 8);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // Mac downloads: browsers all say "Intel" in their user agent, but the graphics
  // renderer often names the chip. Highlight the likely .dmg; both stay one click away.
  const chips = $$("[data-arch]");
  if (chips.length) {
    let gpu = "";
    try {
      const gl = document.createElement("canvas").getContext("webgl");
      const info = gl && gl.getExtension("WEBGL_debug_renderer_info");
      gpu = info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "";
    } catch {}
    const arch = /Intel|AMD|Radeon/.test(gpu) ? "x86_64" : "arm64";
    for (const a of chips) a.classList.toggle("primary", a.dataset.arch === arch);
  }

  // Copy buttons.
  for (const btn of $$("[data-copy]")) {
    const label = btn.innerHTML;
    btn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(btn.dataset.copy);
      } catch {
        return;
      }
      btn.classList.add("done");
      btn.innerHTML = '<svg aria-hidden="true"><use href="/img/sprite.svg#check"/></svg>Copied';
      setTimeout(() => {
        btn.classList.remove("done");
        btn.innerHTML = label;
      }, 1600);
    });
  }

  // Install tabs, preselected for the visitor's OS.
  const install = $("[data-install]");
  if (install) {
    const tabs = $$("[data-os]", install);
    const select = (os) => {
      for (const t of tabs) t.setAttribute("aria-selected", String(t.dataset.os === os));
      for (const p of $$("[data-panel]", install)) p.hidden = p.dataset.panel !== os;
    };
    for (const t of tabs) t.addEventListener("click", () => select(t.dataset.os));
    const platform = (navigator.userAgentData?.platform || navigator.platform || navigator.userAgent).toLowerCase();
    if (platform.includes("mac")) select("mac");
    else if (platform.includes("win")) select("windows");
  }

  // Reveal on scroll.
  const revealer = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          revealer.unobserve(e.target);
        }
      }
    },
    { rootMargin: "0px 0px -8% 0px" },
  );
  for (const el of $$(".reveal")) revealer.observe(el);

  // Theme gallery.
  const stage = $("[data-theme-stage]");
  if (stage) {
    const buttons = $$("[data-pick]");
    const show = (name) => {
      for (const img of $$("img", stage)) img.classList.toggle("on", img.dataset.theme === name);
      for (const b of buttons) b.setAttribute("aria-pressed", String(b.dataset.pick === name));
    };
    for (const b of buttons) {
      b.addEventListener("click", () => {
        show(b.dataset.pick);
        clearInterval(cycle);
      });
    }
    // Gently cycle until someone picks one.
    let i = 0;
    const cycle = reduceMotion
      ? null
      : setInterval(() => {
          if (stage.getBoundingClientRect().top > innerHeight) return;
          i = (i + 1) % buttons.length;
          show(buttons[i].dataset.pick);
        }, 3200);
  }

  // Docs: highlight the section in view.
  const docsNav = $(".docs-nav") || $(".toc");
  if (docsNav) {
    const links = new Map($$("a[href^='#']", docsNav).map((a) => [a.getAttribute("href").slice(1), a]));
    const spy = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          for (const a of links.values()) a.classList.remove("on");
          const on = links.get(e.target.id);
          on?.classList.add("on");
          // On phones the nav is one swipeable row; keep the current section's chip in view.
          if (on && docsNav.scrollWidth > docsNav.clientWidth) {
            docsNav.scrollTo({ left: on.offsetLeft - 16, behavior: reduceMotion ? "auto" : "smooth" });
          }
        }
      },
      { rootMargin: "-20% 0px -70% 0px" },
    );
    for (const id of links.keys()) {
      const el = document.getElementById(id);
      if (el) spy.observe(el);
    }
  }

  demo();

  // ---------------------------------------------------------------------------------
  // The hero: a replay of real Omacus sheets, typed out with results appearing.

  function demo() {
    const sheets = window.OMACUS_DEMO;
    const stageEl = $("[data-stage]");
    const list = $("[data-lines]");
    const side = $("[data-sheets]");
    const popup = $("[data-popup]");
    if (!sheets || !list) return;

    side.innerHTML = sheets.map((s, i) => `<div class="sheet" data-i="${i}">${esc(s.title)}</div>`).join("");

    const tilt = new IntersectionObserver((es) => {
      for (const e of es) if (e.isIntersecting) stageEl.classList.add("in");
    });
    tilt.observe(stageEl);

    let visible = true;
    new IntersectionObserver((es) => (visible = es[0].isIntersecting)).observe(stageEl);
    const sleep = (ms) =>
      new Promise((resolve) => {
        const tick = () => (visible && !document.hidden ? setTimeout(resolve, ms) : setTimeout(tick, 250));
        tick();
      });

    const setActive = (i) => {
      for (const el of $$(".sheet", side)) el.classList.toggle("active", Number(el.dataset.i) === i);
    };

    const render = (sheet, upto, partial, showCaret) => {
      const vars = definedVars(sheet.lines.map((l) => l.text));
      list.innerHTML = "";
      sheet.lines.slice(0, upto + 1).forEach((line, idx) => {
        const typing = idx === upto && partial !== null;
        const text = typing ? line.text.slice(0, partial) : line.text;
        const li = document.createElement("li");
        li.innerHTML =
          `<span class="src">${highlight(text, vars)}${typing && showCaret ? '<span class="caret"></span>' : ""}</span>` +
          `<span class="res${!typing && line.result ? " show" : ""}">${esc(line.result || "")}</span>`;
        list.appendChild(li);
      });
      return list.lastElementChild;
    };

    // Final state only, for reduced motion.
    if (reduceMotion) {
      setActive(0);
      render(sheets[0], sheets[0].lines.length - 1, null, false);
      return;
    }

    const COMPLETIONS = {
      ra: ["rate", "radian", "rankine"],
      ea: ["each"],
      su: ["subtotal", "sum", "sunday"],
      ta: ["tax", "tablespoon", "tan"],
      in: null,
    };

    const showPopup = (li, items) => {
      const caret = $(".caret", li);
      if (!caret) return;
      const box = list.parentElement.getBoundingClientRect();
      const c = caret.getBoundingClientRect();
      popup.innerHTML = items.map((w, k) => `<div class="${k ? "" : "sel"}">${esc(w)}</div>`).join("");
      popup.style.left = `${c.left - box.left - 12}px`;
      popup.style.top = `${c.bottom - box.top + 4}px`;
      popup.classList.add("show");
    };
    const hidePopup = () => popup.classList.remove("show");

    (async function loop() {
      for (let s = 0; ; s = (s + 1) % sheets.length) {
        const sheet = sheets[s];
        const vars = definedVars(sheet.lines.map((l) => l.text));
        setActive(s);
        list.innerHTML = "";
        await sleep(500);
        for (let i = 0; i < sheet.lines.length; i++) {
          const text = sheet.lines[i].text;
          let completedWord = false;
          for (let n = 0; n <= text.length; n++) {
            const li = render(sheet, i, n, true);
            // Autocomplete: after two letters of a variable, show the popup and accept.
            const word = text.slice(0, n).match(/[A-Za-z_]+$/)?.[0];
            const rest = text.slice(n).match(/^[A-Za-z_]*/)[0];
            if (!completedWord && word?.length === 2 && rest.length > 0 && vars.has(word + rest)) {
              const items = COMPLETIONS[word.toLowerCase()];
              if (items) {
                showPopup(li, items);
                await sleep(650);
                hidePopup();
                n += rest.length;
                render(sheet, i, n, true);
                completedWord = true;
                await sleep(120);
                continue;
              }
            }
            if (!word) completedWord = false;
            await sleep(text[n - 1] === " " ? 55 : 24 + Math.random() * 38);
          }
          render(sheet, i, null, false);
          await sleep(text ? 230 : 120);
        }
        // Keep the caret on a fresh line at the end.
        const li = document.createElement("li");
        li.innerHTML = '<span class="src"><span class="caret"></span></span><span class="res"></span>';
        list.appendChild(li);
        await sleep(3400);
      }
    })();
  }

  // Same rules as the app's highlighter (src/highlight.rs), simplified.
  const KEYWORDS = new Set(
    "in to as of on off what is prev sum total avg average mean today now tomorrow yesterday plus minus times mod".split(" "),
  );

  function definedVars(lines) {
    const vars = new Set();
    for (const l of lines) {
      const m = l.match(/^\s*([A-Za-z_]\w*)\s*=[^=]/);
      if (m) vars.add(m[1]);
    }
    return vars;
  }

  function highlight(line, vars) {
    if (/^\s*#/.test(line)) return `<span class="t-head">${esc(line)}</span>`;
    let code = line;
    let comment = "";
    const c = line.indexOf("//");
    if (c >= 0) {
      code = line.slice(0, c);
      comment = line.slice(c);
    }
    let out = "";
    const label = code.match(/^(\s*[A-Za-z][A-Za-z '\-_&/]*:)(?= )/);
    if (label) {
      out += `<span class="t-label">${esc(label[1])}</span>`;
      code = code.slice(label[1].length);
    }
    const re = /("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹₩₽₺%])|(\s+)|(.)/gu;
    let m;
    while ((m = re.exec(code))) {
      const [tok, str, num, word, sym, ws] = m;
      if (str) out += `<span class="t-com">${esc(tok)}</span>`;
      else if (num || sym) out += `<span class="t-num">${esc(tok)}</span>`;
      else if (word) {
        const cls = vars.has(word) ? "t-var" : KEYWORDS.has(word.toLowerCase()) ? "t-kw" : "";
        out += cls ? `<span class="${cls}">${esc(tok)}</span>` : esc(tok);
      } else if (ws) out += tok;
      else out += `<span class="t-op">${esc(tok)}</span>`;
    }
    if (comment) out += `<span class="t-com">${esc(comment)}</span>`;
    return out;
  }

  function esc(s) {
    return s.replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);
  }
})();
