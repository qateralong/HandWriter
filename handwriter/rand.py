from __future__ import annotations

import hashlib
import math
import struct
from functools import lru_cache


@lru_cache(maxsize=200_000)
def _hash01(text: str) -> float:
    h = hashlib.blake2b(text.encode("utf-8"), digest_size=8).digest()
    return struct.unpack("<Q", h)[0] / 2.0**64


def rnd(seed: int, channel: str, *keys) -> float:
    return _hash01(f"{seed}|{channel}|" + "|".join(str(k) for k in keys))


def urnd(seed: int, channel: str, *keys) -> float:
    return 2.0 * rnd(seed, channel, *keys) - 1.0


def pick(seed: int, channel: str, n: int, *keys) -> int:
    return min(int(rnd(seed, channel, *keys) * n), n - 1)


def vnoise(seed: int, channel: str, t: float, *keys) -> float:
    i = math.floor(t)
    f = t - i
    a = urnd(seed, channel, *keys, i)
    b = urnd(seed, channel, *keys, i + 1)
    s = f * f * f * (f * (f * 6 - 15) + 10)
    return a + (b - a) * s
