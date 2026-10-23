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

  // Downloads: the visitor's system first, marked. (Linux: Omarchy's card is first.)
  const downloads = $(".downloads");
  const system = (navigator.userAgentData?.platform || navigator.platform || navigator.userAgent).toLowerCase();
  const isPhone = /iphone|ipad|android/.test(navigator.userAgent.toLowerCase());
  const yours = isPhone ? null : system.includes("mac") ? "mac" : system.includes("win") ? "windows" : null;
  const card = yours && downloads && $(`#${yours}`, downloads);
  if (card) {
    downloads.prepend(card);
    card.classList.add("yours");
    const tag = document.createElement("span");
    tag.className = "yours-tag";
    tag.textContent = "Your system";
    $("header", card).append(tag);
  }

  // Phones: the menu button opens the site links under the header.
  const menu = $(".menu-toggle");
  if (menu && header) {
    const setOpen = (open) => {
      header.classList.toggle("open", open);
      document.documentElement.classList.toggle("menu-open", open);
      menu.setAttribute("aria-expanded", String(open));
      menu.setAttribute("aria-label", open ? "Close menu" : "Menu");
    };
    menu.addEventListener("click", () => setOpen(!header.classList.contains("open")));
    for (const a of $$("#site-nav a")) a.addEventListener("click", () => setOpen(false));
    addEventListener("keydown", (e) => e.key === "Escape" && setOpen(false));
    matchMedia("(min-width: 1021px)").addEventListener("change", () => setOpen(false));
  }

  // A link to a smaller update (from the app's What's new) opens the group it was folded into.
  const openHashedDetails = () => {
    const id = decodeURIComponent(location.hash.replace(/^#/, ""));
    if (!id) return;
    const el = document.getElementById(id);
    const details = el?.closest("details");
    if (!details) return;
    details.open = true;
    el.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth" });
  };
  openHashedDetails();
  addEventListener("hashchange", openHashedDetails);

  // Copy buttons.
  for (const btn of $$("[data-copy]")) {
    const label = [...btn.childNodes].map((node) => node.cloneNode(true));
    btn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(btn.dataset.copy);
      } catch {
        return;
      }
      btn.classList.add("done");
      const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      icon.setAttribute("aria-hidden", "true");
      const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
      use.setAttribute("href", "/img/sprite.svg#check");
      icon.append(use);
      btn.replaceChildren(icon, document.createTextNode("Copied"));
      setTimeout(() => {
        btn.classList.remove("done");
        btn.replaceChildren(...label.map((node) => node.cloneNode(true)));
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

  // Docs on phones: a bar names the section in view; tapping it opens the full list.
  const docsBar = $(".docs-nav");
  let docsLabel = null;
  if (docsBar) {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "docs-nav-toggle";
    toggle.setAttribute("aria-expanded", "false");
    const lead = document.createElement("span");
    lead.textContent = "On this page";
    docsLabel = document.createElement("b");
    const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    icon.setAttribute("aria-hidden", "true");
    const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
    use.setAttribute("href", "/img/sprite.svg#chevron");
    icon.append(use);
    toggle.append(lead, docsLabel, icon);
    docsLabel.textContent = $("a", docsBar)?.textContent ?? "";
    docsBar.prepend(toggle);
    const setOpen = (open) => {
      docsBar.classList.toggle("open", open);
      toggle.setAttribute("aria-expanded", String(open));
    };
    toggle.addEventListener("click", () => setOpen(!docsBar.classList.contains("open")));
    for (const a of $$("a", docsBar)) a.addEventListener("click", () => setOpen(false));
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
          if (on && docsLabel) docsLabel.textContent = on.textContent;
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

    side.replaceChildren(...sheets.map((s, i) => {
      const row = document.createElement("div");
      row.className = "sheet";
      row.dataset.i = String(i);
      row.textContent = s.title;
      return row;
    }));

    const tilt = new IntersectionObserver((es) => {
      for (const e of es) if (e.isIntersecting) stageEl.classList.add("in");
    });
    tilt.observe(stageEl);

    let visible = true;
    new IntersectionObserver((es) => (visible = es[0].isIntersecting)).observe(stageEl);
    // Each run of the replay has a number; switching modes starts a new one, and the
    // old run's pending sleeps never wake, so it simply stops.
    let run = 0;
    let current = 0;
    const sleep = (ms, mine = run) =>
      new Promise((resolve) => {
        const tick = () =>
          mine !== run
            ? null
            : visible && !document.hidden
              ? setTimeout(() => mine === run && resolve(), ms)
              : setTimeout(tick, 250);
        tick();
      });

    // Two modes: Watch (the replay) and Try it (the real editor, embedded from
    // /app so a saved page can move before the frame opens app.omacus.com).
    const caption = $("[data-caption]");
    const modes = $$("[data-mode]");
    let frame = null;
    const setMode = (mode) => {
      for (const b of modes) b.setAttribute("aria-pressed", String(b.dataset.mode === mode));
      stageEl.classList.toggle("live", mode === "try");
      caption.textContent =
        mode === "try"
          ? "The real engine, running in your browser. Edit anything; nothing is saved."
          : "A replay with real answers from the Omacus engine.";
    };
    const tryIt = (i = current) => {
      run++;
      current = i;
      setActive(i);
      popup.classList.remove("show");
      setMode("try");
      if (!frame) {
        frame = document.createElement("iframe");
        frame.title = "Omacus, live: edit this page";
        frame.className = "win-frame";
        list.parentElement.append(frame);
        frame.addEventListener("load", () => frame.contentDocument?.querySelector("textarea")?.focus());
      }
      frame.hidden = false;
      frame.src = `/app/?embed#${sheets[i].link}`;
    };
    const watch = () => {
      if (!stageEl.classList.contains("live")) return;
      run++;
      setMode("watch");
      if (frame) frame.hidden = true;
      if (reduceMotion) render(sheets[current], sheets[current].lines.length - 1, null, false);
      else play(current, run);
    };
    for (const b of modes) b.addEventListener("click", () => (b.dataset.mode === "try" ? tryIt() : watch()));
    side.addEventListener("click", (e) => {
      const el = e.target.closest(".sheet");
      if (el) tryIt(Number(el.dataset.i));
    });

    const setActive = (i) => {
      for (const el of $$(".sheet", side)) el.classList.toggle("active", Number(el.dataset.i) === i);
    };

    const render = (sheet, upto, partial, showCaret) => {
      const vars = definedVars(sheet.lines.map((l) => l.text));
      list.replaceChildren();
      sheet.lines.slice(0, upto + 1).forEach((line, idx) => {
        const typing = idx === upto && partial !== null;
        const text = typing ? line.text.slice(0, partial) : line.text;
        const li = document.createElement("li");
        const src = document.createElement("span");
        src.className = "src";
        highlightInto(src, text, vars);
        if (typing && showCaret) {
          const caret = document.createElement("span");
          caret.className = "caret";
          src.append(caret);
        }
        const res = document.createElement("span");
        res.className = "res" + (!typing && line.result ? " show" : "");
        res.textContent = line.result || "";
        li.append(src, res);
        list.append(li);
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
      popup.replaceChildren(...items.map((w, k) => {
        const row = document.createElement("div");
        if (!k) row.className = "sel";
        row.textContent = w;
        return row;
      }));
      popup.style.left = `${c.left - box.left - 12}px`;
      popup.style.top = `${c.bottom - box.top + 4}px`;
      popup.classList.add("show");
    };
    const hidePopup = () => popup.classList.remove("show");

    async function play(from, mine) {
      const pause = (ms) => sleep(ms, mine);
      for (let s = from; ; s = (s + 1) % sheets.length) {
        const sheet = sheets[s];
        const vars = definedVars(sheet.lines.map((l) => l.text));
        current = s;
        setActive(s);
        list.replaceChildren();
        await pause(500);
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
                await pause(650);
                hidePopup();
                n += rest.length;
                render(sheet, i, n, true);
                completedWord = true;
                await pause(120);
                continue;
              }
            }
            if (!word) completedWord = false;
            await pause(text[n - 1] === " " ? 55 : 24 + Math.random() * 38);
          }
          render(sheet, i, null, false);
          await pause(text ? 230 : 120);
        }
        // Keep the caret on a fresh line at the end.
        const li = document.createElement("li");
        const src = document.createElement("span");
        src.className = "src";
        const caret = document.createElement("span");
        caret.className = "caret";
        src.append(caret);
        const res = document.createElement("span");
        res.className = "res";
        li.append(src, res);
        list.append(li);
        await pause(3400);
      }
    }
    play(0, run);
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

  function appendText(parent, cls, text) {
    if (!text) return;
    if (!cls) {
      parent.append(document.createTextNode(text));
      return;
    }
    const span = document.createElement("span");
    span.className = cls;
    span.textContent = text;
    parent.append(span);
  }

  function highlightInto(parent, line, vars) {
    if (/^\s*#/.test(line)) {
      appendText(parent, "t-head", line);
      return;
    }
    let code = line;
    let comment = "";
    const c = line.indexOf("//");
    if (c >= 0) {
      code = line.slice(0, c);
      comment = line.slice(c);
    }
    const label = code.match(/^(\s*[A-Za-z][A-Za-z '\-_&/]*:)(?= )/);
    if (label) {
      appendText(parent, "t-label", label[1]);
      code = code.slice(label[1].length);
    }
    const re = /("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹₩₽₺%])|(\s+)|(.)/gu;
    let m;
    while ((m = re.exec(code))) {
      const [tok, str, num, word, sym, ws] = m;
      if (str) appendText(parent, "t-com", tok);
      else if (num || sym) appendText(parent, "t-num", tok);
      else if (word) {
        const cls = vars.has(word) ? "t-var" : KEYWORDS.has(word.toLowerCase()) ? "t-kw" : "";
        appendText(parent, cls, tok);
      } else if (ws) appendText(parent, "", tok);
      else appendText(parent, "t-op", tok);
    }
    if (comment) appendText(parent, "t-com", comment);
  }

  // Copy saved pages to app.omacus.com in the background. The copy here stays.
  // /app does the same handoff, then opens the app.
  import("/app/migrate-client.js").then((mod) => mod.migrateStoredPages()).catch(() => {});
})();
