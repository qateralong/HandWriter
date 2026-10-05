import json
import os
import threading
import time
import urllib.request
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from handwriter import launcher
from handwriter.launcher import browser_command, find_browser, should_stop


def fake_env(tmp_path):
    return {"ProgramFiles": str(tmp_path / "pf"), "ProgramFiles(x86)": str(tmp_path / "pf86"),
            "LOCALAPPDATA": str(tmp_path / "local")}


def test_chrome_preferred_then_edge_then_none(tmp_path):
    env = fake_env(tmp_path)
    chrome = tmp_path / "local" / "Google" / "Chrome" / "Application" / "chrome.exe"
    edge = tmp_path / "pf86" / "Microsoft" / "Edge" / "Application" / "msedge.exe"
    files = {edge}
    exists = lambda p: Path(p) in files
    assert find_browser(env, exists, registry=False) == ("edge", edge)
    files.add(chrome)
    assert find_browser(env, exists, registry=False) == ("chrome", chrome)
    files.clear()
    assert find_browser(env, exists, registry=False) is None


def test_chrome_in_program_files_x86_and_program_files(tmp_path):
    env = fake_env(tmp_path)
    for root in ("pf", "pf86", "local"):
        p = tmp_path / root / "Google" / "Chrome" / "Application" / "chrome.exe"
        assert find_browser(env, lambda q, p=p: Path(q) == p, registry=False) == ("chrome", p)


def test_browser_command_app_mode(tmp_path):
    cmd = browser_command(Path("chrome.exe"), "http://127.0.0.1:5555/", tmp_path / "prof")
    assert "--app=http://127.0.0.1:5555/" in cmd
    assert "--window-size=1400,900" in cmd
    assert any(a.startswith("--user-data-dir=") for a in cmd)
    assert "--disable-background-timer-throttling" in cmd


def test_should_stop_grace_and_idle():
    assert not should_stop(now=29, started=0, last_ping=None)
    assert should_stop(now=31, started=0, last_ping=None)
    assert not should_stop(now=40, started=0, last_ping=30)
    assert should_stop(now=46, started=0, last_ping=30)
    assert not should_stop(now=20, started=0, last_ping=1)


def test_instance_file_roundtrip(tmp_path):
    p = tmp_path / "instance.json"
    launcher.write_instance(1234, "abc", p)
    assert launcher.read_instance(p)["port"] == 1234
    launcher.remove_instance("чужой", p)
    assert p.exists()
    launcher.remove_instance("abc", p)
    assert not p.exists()
    assert not launcher.instance_alive({"port": 1, "token": "x"}, timeout=0.3)


def _get(url, timeout=2):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.loads(r.read().decode())


def test_single_instance_and_autostop(monkeypatch):
    monkeypatch.setattr(launcher, "PING_IDLE_S", 1.5)
    monkeypatch.setattr(launcher, "STARTUP_GRACE_S", 3.0)
    opened = []
    monkeypatch.setattr(launcher, "open_window", lambda url: opened.append(url) or "test")
    t = threading.Thread(target=launcher.run_app, kwargs={"open_ui": True}, daemon=True)
    t.start()
    for _ in range(100):
        inst = launcher.read_instance()
        if inst and launcher.instance_alive(inst):
            break
        time.sleep(0.1)
    else:
        pytest.fail("сервер не записал instance.json")
    port = inst["port"]
    assert opened == [f"http://127.0.0.1:{port}/"]
    assert _get(f"http://127.0.0.1:{port}/api/instance")["app"] == "HandWriter"

    assert launcher.run_app(open_ui=True) == 0
    assert opened == [f"http://127.0.0.1:{port}/"] * 2

    for _ in range(10):
        _get(f"http://127.0.0.1:{port}/api/ping")
        time.sleep(0.5)
    assert t.is_alive()
    t.join(timeout=10)
    assert not t.is_alive(), "сервер не завершился без пингов"
    assert launcher.read_instance() is None


def test_errors_are_russian_not_tracebacks(monkeypatch):
    from handwriter import server
    client = TestClient(server.app, raise_server_exceptions=False)
    r = client.post("/api/preview", json={"sheet": {"width": "широкий"}})
    assert r.status_code == 422
    assert r.json()["detail"].startswith("Неверные настройки. Лист → width: нужно число")

    def boom(s):
        raise RuntimeError("проверка")
    monkeypatch.setattr(server, "compose", boom)
    r = client.post("/api/preview", json={})
    assert r.status_code == 500
    d = r.json()["detail"]
    assert "Внутренняя ошибка программы" in d and "журнал" in d and "Traceback" not in d
    assert client.get("/nope").json()["detail"].startswith("Не найдено")


def test_ping_endpoint_updates_activity():
    from handwriter import server
    client = TestClient(server.app)
    before = server.Activity.last_ping
    assert client.post("/api/ping").json() == {"ok": True}
    assert server.Activity.last_ping is not None and server.Activity.last_ping != before


def test_user_data_in_appdata(monkeypatch, tmp_path):
    from handwriter import paths
    monkeypatch.delenv("HANDWRITER_HOME", raising=False)
    monkeypatch.setenv("APPDATA", str(tmp_path / "Roaming"))
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "Local"))
    assert paths.settings_path() == tmp_path / "Roaming" / "HandWriter" / "settings.json"
    assert paths.log_path() == tmp_path / "Roaming" / "HandWriter" / "logs" / "handwriter.log"
    assert paths.local_dir() == tmp_path / "Local" / "HandWriter"


@pytest.mark.skipif(os.name == "nt", reason="XDG base dirs are used outside Windows")
def test_user_data_in_xdg_dirs(monkeypatch, tmp_path):
    from handwriter import paths
    for var in ("HANDWRITER_HOME", "APPDATA", "LOCALAPPDATA"):
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "share"))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    assert paths.settings_path() == tmp_path / "share" / "HandWriter" / "settings.json"
    assert paths.log_path() == tmp_path / "share" / "HandWriter" / "logs" / "handwriter.log"
    assert paths.local_dir() == tmp_path / "cache" / "HandWriter"


def test_selftest_passes_from_source(tmp_path):
    from handwriter.selftest import run
    report = tmp_path / "selftest.txt"
    assert run(report) == 0, report.read_text(encoding="utf-8")
    text = report.read_text(encoding="utf-8")
    assert "ИТОГ: всё в порядке" in text and "[FAIL]" not in text
