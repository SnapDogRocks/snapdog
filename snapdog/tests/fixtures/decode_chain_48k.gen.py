#!/usr/bin/env python3
"""Generate a deterministic 48kHz/16-bit/stereo WAV for the decode-chain golden."""
import math, struct, sys, wave

RATE = 48000
FRAMES = 256  # ~5.3 ms
out_wav = sys.argv[1]

frames = bytearray()
for n in range(FRAMES):
    l = int(round(8000 * math.sin(2 * math.pi * 1000.0 * n / RATE)))
    r = int(round(6000 * math.sin(2 * math.pi * 1500.0 * n / RATE)))
    frames += struct.pack("<hh", l, r)

with wave.open(out_wav, "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)   # 16-bit
    w.setframerate(RATE)
    w.writeframes(bytes(frames))
print(f"wrote {out_wav}: {FRAMES} frames @ {RATE}Hz 16-bit stereo")
