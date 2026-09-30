import threading

from handwriter.paths import settings_path
from handwriter.settings import Settings, load_settings, save_settings


def test_roundtrip():
    s = Settings()
    s.text = "Проба\tпера"
    s.typography.size_mm = 2.7
    save_settings(s)
    assert load_settings() == s


def test_concurrent_reads_and_writes_do_not_lose_settings():
    s = Settings()
    s.text = "важный текст"
    save_settings(s)
    errors = []

    def writer():
        for i in range(150):
            s2 = s.model_copy(deep=True)
            s2.typography.size_mm = 2 + i / 1000
            try:
                save_settings(s2)
            except Exception as e:
                errors.append(e)

    def reader():
        for _ in range(300):
            if load_settings().text != "важный текст":
                errors.append("настройки потерялись")

    ts = [threading.Thread(target=writer), threading.Thread(target=reader), threading.Thread(target=reader)]
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    assert errors == []
    assert not settings_path().with_suffix(".broken.json").exists()


def test_really_broken_file_is_kept_aside():
    p = settings_path()
    p.write_text("{ не json", encoding="utf-8")
    assert load_settings() == Settings()
    assert p.with_suffix(".broken.json").read_text(encoding="utf-8") == "{ не json"
