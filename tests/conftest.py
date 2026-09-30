import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from handwriter.glyphs import load_provider
from handwriter.settings import Settings, Travel


@pytest.fixture(autouse=True)
def _isolated_home(tmp_path, monkeypatch):
    monkeypatch.setenv("HANDWRITER_HOME", str(tmp_path / "home"))


@pytest.fixture(scope="session")
def font():
    return load_provider("builtin:hershey_cyrillic.svg")


@pytest.fixture
def settings():
    s = Settings()
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    s.randomness.enabled = False
    return s
