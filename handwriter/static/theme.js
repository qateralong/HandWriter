(() => {
  const root = document.documentElement;
  const get = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
  const put = (k, v) => { try { localStorage.setItem(k, v); } catch {} };
  const mq = matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    const t = get("hw-theme") || "auto";
    root.dataset.theme = t === "auto" ? (mq.matches ? "dark" : "light") : t;
    root.dataset.paper = get("hw-paper") || "white";
    dispatchEvent(new Event("themechange"));
  };
  apply();
  mq.addEventListener("change", apply);
  addEventListener("DOMContentLoaded", () => {
    const b = document.getElementById("themeBtn"), box = document.getElementById("prefs");
    if (!b || !box) return;
    const th = document.getElementById("prefTheme"), pp = document.getElementById("prefPaper");
    th.value = get("hw-theme") || "auto";
    pp.value = get("hw-paper") || "white";
    b.onclick = (e) => { e.stopPropagation(); box.hidden = !box.hidden; };
    document.addEventListener("click", (e) => { if (!box.hidden && !box.contains(e.target)) box.hidden = true; });
    th.onchange = () => { put("hw-theme", th.value); apply(); };
    pp.onchange = () => { put("hw-paper", pp.value); apply(); };
  });
})();
