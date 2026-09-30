"use strict";

(function () {
  let failures = 0;
  let banner = null;

  function showDown() {
    if (banner) return;
    banner = document.createElement("div");
    banner.setAttribute("role", "alert");
    banner.style.cssText = "position:fixed;left:0;right:0;top:0;z-index:9999;padding:12px 16px;" +
      "background:#b3261e;color:#fff;font:15px system-ui,'Segoe UI',sans-serif;text-align:center";
    banner.textContent = "Программа HandWriter остановлена. Закройте это окно и запустите её снова.";
    document.body.appendChild(banner);
  }
  function hideDown() {
    if (banner) { banner.remove(); banner = null; }
  }
  function ping() {
    fetch("/api/ping", { method: "POST", cache: "no-store" })
      .then((r) => { if (!r.ok) throw new Error(r.status); failures = 0; hideDown(); })
      .catch(() => { if (++failures >= 2) showDown(); });
  }

  let worker = null;
  try {
    const src = "setInterval(function(){postMessage(0)}, 3000);";
    worker = new Worker(URL.createObjectURL(new Blob([src], { type: "text/javascript" })));
    worker.onmessage = ping;
  } catch (e) {
    setInterval(ping, 3000);
  }
  ping();
  document.addEventListener("visibilitychange", () => { if (!document.hidden) ping(); });
})();
