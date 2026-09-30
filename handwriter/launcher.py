from __future__ import annotations

import json
import logging
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.request
import webbrowser
from pathlib import Path

from .paths import FROZEN, instance_path, local_dir

log = logging.getLogger("handwriter.launcher")

PING_IDLE_S = 15.0
STARTUP_GRACE_S = 30.0
WINDOW_SIZE = "1400,900"


def browser_candidates(env=None) -> list[tuple[str, Path]]:
    env = os.environ if env is None else env
    roots = {
        "pf": env.get("ProgramFiles", r"C:\Program Files"),
        "pf86": env.get("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        "local": env.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local")),
    }
    chrome = [Path(roots[k]) / "Google" / "Chrome" / "Application" / "chrome.exe" for k in ("pf", "pf86", "local")]
    edge = [Path(roots[k]) / "Microsoft" / "Edge" / "Application" / "msedge.exe" for k in ("pf86", "pf", "local")]
    return [("chrome", p) for p in chrome] + [("edge", p) for p in edge]


def _registry_app_path(exe: str) -> Path | None:
    try:
        import winreg
    except ImportError:
        return None
    for hive in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        try:
            with winreg.OpenKey(hive, rf"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}") as k:
                v, _ = winreg.QueryValueEx(k, None)
                if v:
                    return Path(v.strip('"'))
        except OSError:
            continue
    return None


def find_browser(env=None, exists=None, registry=True) -> tuple[str, Path] | None:
    exists = exists or (lambda p: Path(p).is_file())
    cands = browser_candidates(env)
    for kind in ("chrome", "edge"):
        for k, p in cands:
            if k == kind and exists(p):
                return kind, p
        if registry:
            p = _registry_app_path("chrome.exe" if kind == "chrome" else "msedge.exe")
            if p and exists(p):
                return kind, p
    return None


def browser_command(exe: Path, url: str, profile: Path) -> list[str]:
    return [
        str(exe),
        f"--app={url}",
        f"--window-size={WINDOW_SIZE}",
        f"--user-data-dir={profile}",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-timer-throttling",
        "--disable-renderer-backgrounding",
        "--disable-backgrounding-occluded-windows",
    ]


def open_window(url: str) -> str:
    found = find_browser()
    if found:
        kind, exe = found
        cmd = browser_command(exe, url, local_dir() / "browser")
        flags = 0
        if os.name == "nt":
            flags = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
        try:
            subprocess.Popen(cmd, close_fds=True, creationflags=flags,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            log.info("Окно: %s %s", kind, exe)
            return kind
        except OSError:
            log.exception("Не удалось запустить %s, открываю браузер по умолчанию", exe)
    webbrowser.open(url)
    log.info("Окно: браузер по умолчанию (Chrome и Edge не найдены)")
    return "default"


def read_instance(path: Path | None = None) -> dict | None:
    path = path or instance_path()
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def instance_alive(inst: dict | None, timeout: float = 1.5) -> bool:
    if not inst or "port" not in inst:
        return False
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{int(inst['port'])}/api/instance", timeout=timeout) as r:
            data = json.loads(r.read().decode("utf-8"))
        return data.get("app") == "HandWriter" and data.get("token") == inst.get("token")
    except (OSError, ValueError):
        return False


def write_instance(port: int, token: str, path: Path | None = None) -> None:
    path = path or instance_path()
    path.write_text(json.dumps({"port": port, "token": token, "pid": os.getpid()}), encoding="utf-8")


def remove_instance(token: str, path: Path | None = None) -> None:
    path = path or instance_path()
    inst = read_instance(path)
    if inst and inst.get("token") == token:
        try:
            path.unlink()
        except OSError:
            pass


def should_stop(now: float, started: float, last_ping: float | None,
                idle: float = PING_IDLE_S, grace: float = STARTUP_GRACE_S) -> bool:
    if last_ping is None:
        return now - started > grace
    return now - last_ping > idle and now - started > grace


def free_port(preferred: int | None = None) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("127.0.0.1", preferred or 0))
        except OSError:
            s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _make_server(port: int):
    import uvicorn

    from .server import app
    config = uvicorn.Config(app, host="127.0.0.1", port=port, log_config=None, access_log=False,
                            loop="asyncio", http="h11", ws="none", lifespan="off")
    return uvicorn.Server(config)


def run_app(open_ui: bool = True, watchdog: bool = True, single: bool = True, port: int | None = None,
            browser_tab: bool = False) -> int:
    from .server import Activity

    if single:
        inst = read_instance()
        if instance_alive(inst):
            url = f"http://127.0.0.1:{inst['port']}/"
            log.info("Программа уже запущена (%s), открываю окно", url)
            if open_ui:
                webbrowser.open(url) if browser_tab else open_window(url)
            return 0

    port = free_port(port)
    url = f"http://127.0.0.1:{port}/"
    server = _make_server(port)

    def when_ready():
        for _ in range(300):
            if server.started:
                break
            time.sleep(0.05)
        if not server.started:
            log.error("Сервер не запустился")
            return
        if single:
            write_instance(port, Activity.token)
        log.info("Сервер %s готов", url)
        if not FROZEN:
            print(f"HandWriter: {url}  (Ctrl+C для выхода)", flush=True)
        if open_ui:
            if browser_tab:
                webbrowser.open(url)
            else:
                open_window(url)

    def watch():
        while not server.should_exit:
            time.sleep(1.0)
            if should_stop(time.monotonic(), Activity.started, Activity.last_ping, PING_IDLE_S, STARTUP_GRACE_S):
                log.info("Окно закрыто (нет пингов %s с), завершаю сервер", PING_IDLE_S)
                server.should_exit = True

    Activity.started = time.monotonic()
    threading.Thread(target=when_ready, daemon=True, name="ready").start()
    if watchdog:
        threading.Thread(target=watch, daemon=True, name="watchdog").start()
    try:
        server.run()
    finally:
        if single:
            remove_instance(Activity.token)
        log.info("Сервер остановлен")
    return 0


def fatal_message(text: str) -> None:
    log.error(text)
    if os.name == "nt" and FROZEN:
        try:
            import ctypes
            ctypes.windll.user32.MessageBoxW(None, text, "HandWriter", 0x10)
            return
        except Exception:
            pass
    print(text, file=sys.stderr)
