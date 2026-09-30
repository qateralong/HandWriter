from __future__ import annotations

import io
import logging
import logging.handlers
import sys

from .paths import log_path

log = logging.getLogger("handwriter")
_configured = False


class _Null(io.TextIOBase):
    def write(self, s):
        return len(s)


def setup_logging(console: bool = True) -> logging.Logger:
    global _configured
    if sys.stdout is None:
        sys.stdout = _Null()
    if sys.stderr is None:
        sys.stderr = _Null()
    if _configured:
        return log
    fmt = logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s")
    fh = logging.handlers.RotatingFileHandler(log_path(), maxBytes=1_000_000, backupCount=3, encoding="utf-8")
    fh.setFormatter(fmt)
    root = logging.getLogger()
    root.setLevel(logging.INFO)
    root.addHandler(fh)
    if console and not isinstance(sys.stderr, _Null):
        ch = logging.StreamHandler(sys.stderr)
        ch.setFormatter(fmt)
        ch.setLevel(logging.WARNING)
        root.addHandler(ch)
    for name in ("uvicorn", "uvicorn.error"):
        logging.getLogger(name).setLevel(logging.INFO)
    logging.getLogger("uvicorn.access").setLevel(logging.WARNING)
    _configured = True
    return log
