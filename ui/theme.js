(() => {
  const root = document.documentElement;
  const get = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
  const mq = matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    const t = get("hw-theme") || "auto";
    root.dataset.theme = t === "auto" ? (mq.matches ? "dark" : "light") : t;
    dispatchEvent(new Event("themechange"));
  };
  window.hwApplyTheme = apply;
  apply();
  mq.addEventListener("change", apply);
})();
