"use strict";

(() => {
  const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
  const SEL_VALUE = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value");
  const SEL_INDEX = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "selectedIndex");
  const CHEVRON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6"/></svg>';
  const CHECK = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';
  let openSel = null;
  let uid = 0;

  function el(tag, cls, html) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (html != null) e.innerHTML = html;
    return e;
  }

  function fancySelect(sel) {
    if (sel.dataset.fancy || sel.multiple) return;
    sel.dataset.fancy = "1";
    const wrap = el("span", "fsel");
    const btn = el("button", "fsel-btn");
    btn.type = "button";
    const label = el("span", "fsel-label");
    const chev = el("span", "fsel-chev", CHEVRON);
    btn.append(label, chev);
    const id = "fsel" + ++uid;
    btn.setAttribute("role", "combobox");
    btn.setAttribute("aria-haspopup", "listbox");
    btn.setAttribute("aria-expanded", "false");
    btn.setAttribute("aria-controls", id);
    const aria = sel.getAttribute("aria-label");
    if (aria) btn.setAttribute("aria-label", aria);
    if (sel.title) btn.title = sel.title;
    sel.parentNode.insertBefore(wrap, sel);
    wrap.append(sel, btn);
    sel.tabIndex = -1;
    sel.setAttribute("aria-hidden", "true");

    const refresh = () => {
      const o = sel.options[SEL_INDEX.get.call(sel)];
      label.textContent = o ? o.textContent : "";
      label.classList.toggle("empty", !o || o.value === "");
      btn.disabled = sel.disabled;
      wrap.classList.toggle("disabled", sel.disabled);
      wrap.hidden = sel.hidden;
      if (sel.title && btn.title !== sel.title) btn.title = sel.title;
      if (openSel?.sel === sel) renderList();
    };
    Object.defineProperty(sel, "value", {
      configurable: true,
      get() { return SEL_VALUE.get.call(this); },
      set(v) { SEL_VALUE.set.call(this, v); refresh(); },
    });
    Object.defineProperty(sel, "selectedIndex", {
      configurable: true,
      get() { return SEL_INDEX.get.call(this); },
      set(v) { SEL_INDEX.set.call(this, v); refresh(); },
    });
    new MutationObserver(refresh).observe(sel, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["disabled", "hidden", "title"] });
    sel.addEventListener("change", refresh);

    let panel = null, list = null, search = null, active = -1, typed = "", typedAt = 0;

    function choose(i) {
      const o = sel.options[i];
      if (!o || o.disabled) return;
      const changed = SEL_INDEX.get.call(sel) !== i;
      SEL_INDEX.set.call(sel, i);
      refresh();
      close(true);
      if (changed) {
        sel.dispatchEvent(new Event("input", { bubbles: true }));
        sel.dispatchEvent(new Event("change", { bubbles: true }));
      }
    }

    function setActive(i, scroll = true) {
      const items = list ? [...list.children] : [];
      if (!items.length) return;
      const visible = items.filter((x) => !x.hidden);
      if (!visible.length) return;
      if (!visible.includes(items[i])) i = Number(visible[0].dataset.i);
      active = i;
      for (const it of items) it.classList.toggle("active", Number(it.dataset.i) === i);
      const it = items.find((x) => Number(x.dataset.i) === i);
      if (it) {
        btn.setAttribute("aria-activedescendant", it.id);
        if (scroll) it.scrollIntoView({ block: "nearest" });
      }
    }

    function renderList() {
      if (!list) return;
      list.innerHTML = "";
      const cur = SEL_INDEX.get.call(sel);
      [...sel.options].forEach((o, i) => {
        const it = el("div", "fsel-opt");
        it.id = `${id}-${i}`;
        it.dataset.i = i;
        it.setAttribute("role", "option");
        it.setAttribute("aria-selected", String(i === cur));
        if (i === cur) it.classList.add("selected");
        if (o.disabled) it.classList.add("disabled");
        const tx = el("span", "fsel-text");
        tx.textContent = o.textContent;
        it.append(el("span", "fsel-check", CHECK), tx);
        it.style.setProperty("--d", `${Math.min(i, 10) * 18}ms`);
        it.addEventListener("pointerenter", () => setActive(i, false));
        it.addEventListener("click", (e) => { e.stopPropagation(); choose(i); });
        list.append(it);
      });
      filter();
    }

    function filter() {
      if (!search || !list) return;
      const q = search.value.trim().toLowerCase();
      let first = -1;
      for (const it of list.children) {
        const show = !q || it.textContent.toLowerCase().includes(q);
        it.hidden = !show;
        if (show && first < 0) first = Number(it.dataset.i);
      }
      list.classList.toggle("nothing", first < 0);
      if (first >= 0 && (active < 0 || list.children[active]?.hidden)) setActive(first);
    }

    function place() {
      if (!panel) return;
      const r = btn.getBoundingClientRect();
      const vw = innerWidth, vh = innerHeight;
      const width = Math.min(Math.max(r.width, 200), 380, vw - 16);
      panel.style.width = width + "px";
      const below = vh - r.bottom - 10, above = r.top - 10;
      const want = Math.min(panel.scrollHeight, 340);
      const up = below < Math.min(want, 220) && above > below;
      const max = Math.max(120, Math.min(340, up ? above : below));
      panel.style.maxHeight = max + "px";
      let left = Math.min(Math.max(8, r.left), vw - width - 8);
      panel.style.left = left + "px";
      if (up) {
        panel.style.top = "";
        panel.style.bottom = vh - r.top + 6 + "px";
      } else {
        panel.style.bottom = "";
        panel.style.top = r.bottom + 6 + "px";
      }
      panel.classList.toggle("up", up);
    }

    function open() {
      if (sel.disabled) return;
      if (openSel) openSel.close(false);
      panel = el("div", "fsel-panel");
      panel.setAttribute("role", "listbox");
      panel.id = id;
      if (sel.options.length > 10) {
        search = el("input", "fsel-search");
        search.type = "text";
        search.placeholder = "Поиск…";
        search.setAttribute("aria-label", "Поиск");
        search.addEventListener("input", filter);
        search.addEventListener("keydown", onKey);
        panel.append(search);
      }
      list = el("div", "fsel-list");
      list.dataset.empty = "Ничего не найдено";
      panel.append(list);
      document.body.append(panel);
      renderList();
      place();
      active = SEL_INDEX.get.call(sel);
      setActive(active < 0 ? 0 : active);
      btn.setAttribute("aria-expanded", "true");
      wrap.classList.add("open");
      openSel = { sel, close };
      if (search) setTimeout(() => search?.focus(), 30);
    }

    function close(focus) {
      if (!panel) return;
      const p = panel;
      panel = list = search = null;
      openSel = null;
      btn.setAttribute("aria-expanded", "false");
      btn.removeAttribute("aria-activedescendant");
      wrap.classList.remove("open");
      if (reduced()) p.remove();
      else {
        p.classList.add("closing");
        p.addEventListener("animationend", () => p.remove(), { once: true });
        setTimeout(() => p.remove(), 260);
      }
      if (focus) btn.focus({ preventScroll: true });
    }

    function step(d) {
      const items = [...list.children].filter((x) => !x.hidden && !x.classList.contains("disabled"));
      if (!items.length) return;
      let k = items.findIndex((x) => Number(x.dataset.i) === active);
      k = k < 0 ? 0 : Math.max(0, Math.min(items.length - 1, k + d));
      setActive(Number(items[k].dataset.i));
    }

    function onKey(e) {
      const isOpen = !!panel;
      const k = e.key;
      if (!isOpen) {
        if (k === "ArrowDown" || k === "ArrowUp" || k === "Enter" || k === " ") {
          e.preventDefault();
          open();
        }
        return;
      }
      if (k === "Escape" || k === "Tab") {
        if (k === "Escape") { e.preventDefault(); e.stopPropagation(); }
        close(k === "Escape");
      } else if (k === "ArrowDown") { e.preventDefault(); step(1); }
      else if (k === "ArrowUp") { e.preventDefault(); step(-1); }
      else if (k === "PageDown") { e.preventDefault(); step(6); }
      else if (k === "PageUp") { e.preventDefault(); step(-6); }
      else if (k === "Home" && e.target !== search) { e.preventDefault(); step(-1e6); }
      else if (k === "End" && e.target !== search) { e.preventDefault(); step(1e6); }
      else if (k === "Enter" || (k === " " && e.target !== search)) { e.preventDefault(); choose(active); }
      else if (k.length === 1 && !search && /\S/.test(k)) {
        const now = Date.now();
        typed = now - typedAt > 700 ? k.toLowerCase() : typed + k.toLowerCase();
        typedAt = now;
        const hit = [...sel.options].findIndex((o) => o.textContent.toLowerCase().startsWith(typed));
        if (hit >= 0) setActive(hit);
      }
    }

    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (panel) close(false);
      else open();
    });
    btn.addEventListener("keydown", onKey);
    btn.addEventListener("mousedown", (e) => e.stopPropagation());
    sel.fancyRefresh = refresh;
    sel.fancyPlace = () => place();
    refresh();
  }

  document.addEventListener("pointerdown", (e) => {
    if (openSel && !e.target.closest(".fsel-panel") && !e.target.closest(".fsel.open")) openSel.close(false);
  }, true);
  document.addEventListener("mousedown", (e) => {
    if (e.target.closest(".fsel-panel")) e.stopImmediatePropagation();
  });
  addEventListener("resize", () => openSel?.close(false));
  document.addEventListener("scroll", (e) => {
    if (openSel && !e.target.closest?.(".fsel-panel")) openSel.close(false);
  }, true);
  addEventListener("keydown", (e) => {
    if (openSel && e.key === "Escape") {
      e.stopPropagation();
      openSel.close(true);
    }
  }, true);

  function fancyNumber(inp) {
    if (inp.dataset.fancy) return;
    inp.dataset.fancy = "1";
    const wrap = el("span", "fnum");
    const minus = el("button", "fnum-btn", "<svg viewBox='0 0 24 24' aria-hidden='true'><path d='M6 12h12'/></svg>");
    const plus = el("button", "fnum-btn", "<svg viewBox='0 0 24 24' aria-hidden='true'><path d='M12 6v12M6 12h12'/></svg>");
    minus.type = plus.type = "button";
    minus.tabIndex = plus.tabIndex = -1;
    minus.setAttribute("aria-label", "Меньше");
    plus.setAttribute("aria-label", "Больше");
    inp.parentNode.insertBefore(wrap, inp);
    wrap.append(minus, inp, plus);
    const sync = () => {
      minus.disabled = plus.disabled = inp.disabled || inp.readOnly;
      wrap.classList.toggle("disabled", inp.disabled);
      wrap.hidden = inp.hidden;
    };
    new MutationObserver(sync).observe(inp, { attributes: true, attributeFilter: ["disabled", "readonly", "hidden"] });
    sync();
    const decimals = (x) => {
      const t = String(x);
      return t.includes(".") ? t.split(".")[1].length : 0;
    };
    const bump = (d) => {
      const before = inp.value;
      const st = Number(inp.step) > 0 ? Number(inp.step) : 1;
      const min = inp.min !== "" ? Number(inp.min) : -Infinity;
      const max = inp.max !== "" ? Number(inp.max) : Infinity;
      let v = inp.value === "" ? (Number.isFinite(min) ? min : 0) : Number(inp.value);
      if (!Number.isFinite(v)) v = 0;
      const dec = Math.min(6, Math.max(decimals(inp.step || 1), decimals(inp.value)));
      let nv = inp.value === "" ? v : v + d * st;
      nv = Math.min(max, Math.max(min, nv));
      inp.value = String(Number(nv.toFixed(dec)));
      if (inp.value !== before) {
        inp.dispatchEvent(new Event("input", { bubbles: true }));
        wrap.classList.remove("pulse");
        void wrap.offsetWidth;
        wrap.classList.add("pulse");
        return true;
      }
      return false;
    };
    for (const [b, d] of [[minus, -1], [plus, 1]]) {
      let timer = 0, changed = false;
      const stop = () => {
        clearTimeout(timer);
        timer = 0;
        b.classList.remove("held");
        if (changed) inp.dispatchEvent(new Event("change", { bubbles: true }));
        changed = false;
      };
      b.addEventListener("pointerdown", (e) => {
        if (b.disabled || e.button !== 0) return;
        e.preventDefault();
        b.setPointerCapture(e.pointerId);
        b.classList.add("held");
        changed = bump(d) || changed;
        let delay = 420;
        const rep = () => {
          changed = bump(d) || changed;
          delay = Math.max(40, delay * 0.82);
          timer = setTimeout(rep, delay);
        };
        timer = setTimeout(rep, delay);
      });
      b.addEventListener("pointerup", stop);
      b.addEventListener("pointercancel", stop);
      b.addEventListener("lostpointercapture", () => timer && stop());
    }
  }

  function placeSlider(box, instant) {
    let ind = box.querySelector(":scope > .slider-ind");
    const on = box.querySelector(":scope > .on");
    if (!ind) {
      ind = el("span", "slider-ind");
      box.prepend(ind);
      instant = true;
    }
    if (!on || !on.offsetWidth) {
      ind.style.opacity = "0";
      return;
    }
    if (instant || reduced()) ind.classList.add("instant");
    ind.style.opacity = "1";
    ind.style.transform = `translate(${on.offsetLeft}px, ${on.offsetTop}px)`;
    ind.style.width = on.offsetWidth + "px";
    ind.style.height = on.offsetHeight + "px";
    if (instant) requestAnimationFrame(() => requestAnimationFrame(() => ind.classList.remove("instant")));
  }

  function slider(box) {
    if (box.dataset.slider) return;
    box.dataset.slider = "1";
    box.classList.add("has-slider");
    placeSlider(box, true);
    const own = (n) => n.classList?.contains("slider-ind") || n.classList?.contains("ripple");
    new MutationObserver((ms) => {
      if (ms.some((m) => (m.type === "childList" ? [...m.addedNodes, ...m.removedNodes].some((n) => !own(n)) : !own(m.target)))) placeSlider(box);
    }).observe(box, { attributes: true, attributeFilter: ["class", "hidden"], subtree: true, childList: true });
    new ResizeObserver(() => placeSlider(box, true)).observe(box);
  }

  function ripple(e) {
    const b = e.target.closest(".btn, .icon, .seg button, .tabs button, .cal-card, .fsel-btn");
    if (!b || b.disabled || reduced()) return;
    const r = b.getBoundingClientRect();
    const d = Math.hypot(Math.max(e.clientX - r.left, r.right - e.clientX), Math.max(e.clientY - r.top, r.bottom - e.clientY)) * 2;
    const s = el("span", "ripple");
    s.style.width = s.style.height = d + "px";
    s.style.left = e.clientX - r.left - d / 2 + "px";
    s.style.top = e.clientY - r.top - d / 2 + "px";
    b.append(s);
    s.addEventListener("animationend", () => s.remove(), { once: true });
    setTimeout(() => s.remove(), 900);
  }
  document.addEventListener("pointerdown", ripple, true);

  let tip = null, tipFor = null, tipTimer = 0;
  function hideTip() {
    clearTimeout(tipTimer);
    if (tipFor?.dataset.tip != null && !tipFor.title) tipFor.title = tipFor.dataset.tip;
    tipFor = null;
    if (!tip) return;
    const t = tip;
    tip = null;
    t.classList.add("closing");
    setTimeout(() => t.remove(), 180);
  }
  function showTip(target) {
    const text = target.title || target.dataset.tip;
    if (!text || !document.contains(target)) return;
    tip = el("div", "tip");
    tip.textContent = text;
    document.body.append(tip);
    const r = target.getBoundingClientRect(), w = tip.offsetWidth, hgt = tip.offsetHeight;
    const below = r.bottom + 8 + hgt < innerHeight;
    tip.style.left = Math.min(innerWidth - w - 8, Math.max(8, r.left + r.width / 2 - w / 2)) + "px";
    tip.style.top = (below ? r.bottom + 8 : r.top - hgt - 8) + "px";
    tip.classList.add(below ? "below" : "above");
  }
  document.addEventListener("pointerover", (e) => {
    const t = e.target.closest?.(".icon[title], .btn[title], .icon[data-tip], .btn[data-tip], .cal-steps li[title]");
    if (t === tipFor) return;
    hideTip();
    if (!t || e.pointerType === "touch") return;
    tipFor = t;
    if (t.title) {
      t.dataset.tip = t.title;
      t.removeAttribute("title");
    }
    tipTimer = setTimeout(() => tipFor === t && showTip(t), 420);
  });
  document.addEventListener("pointerdown", hideTip, true);
  addEventListener("blur", hideTip);

  function enhance(root) {
    if (!(root instanceof Element || root instanceof Document)) return;
    const all = (q) => (root instanceof Element && root.matches(q) ? [root] : []).concat([...root.querySelectorAll(q)]);
    for (const s of all("select")) fancySelect(s);
    for (const n of all('input[type="number"]')) fancyNumber(n);
    for (const b of all(".seg, .tabs")) slider(b);
  }

  new MutationObserver((ms) => {
    for (const m of ms) for (const n of m.addedNodes) if (n.nodeType === 1 && !n.closest(".fsel-panel")) enhance(n);
  }).observe(document.documentElement, { childList: true, subtree: true });

  const start = () => {
    enhance(document);
    document.body.classList.add("ready");
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
  window.hwEnhance = enhance;
})();
