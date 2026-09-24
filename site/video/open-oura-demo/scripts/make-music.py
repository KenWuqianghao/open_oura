#!/usr/bin/env python3
"""Music bed for the Open Oura demo (32 s, C major, 72 BPM), written to the beat map
in STORYBOARD.md. Adapted from the demo-video skill's make-music.py. Deterministic.
Run from the video directory: python3 scripts/make-music.py
"""
import math, os, wave, struct, subprocess
import numpy as np

SR = 48000
TOTAL = 32.0
BPM = 72
BEAT = 60 / BPM
out = np.zeros(int(TOTAL * SR) + SR)

def midi(n): return 440.0 * 2 ** ((n - 69) / 12)

def note(t0, m, dur, vel=0.5, bright=1.0):
    """Felt-piano-ish tone: harmonics 1..4 with decaying weights, exp envelope."""
    n = int(dur * SR)
    if n <= 0: return
    t = np.arange(n) / SR
    f = midi(m)
    env = np.exp(-t / (dur * 0.35)) * (1 - np.exp(-t / 0.006))
    sig = (np.sin(2*math.pi*f*t)
           + 0.35*bright*np.sin(2*math.pi*2*f*t + 0.3)
           + 0.12*bright*np.sin(2*math.pi*3*f*t + 0.7)
           + 0.05*np.sin(2*math.pi*4*f*t))
    # slight detuned second voice for warmth
    sig += 0.25*np.sin(2*math.pi*f*1.003*t)
    s = int(t0 * SR)
    seg = (sig * env * vel).astype(np.float64)
    end = min(len(out), s + n)
    out[s:end] += seg[:end - s]

def chord(t0, notes, dur, vel=0.4, spread=0.012, bright=1.0):
    for i, m in enumerate(notes):
        note(t0 + i * spread, m, dur, vel, bright)

C4, D4, E4, F4, G4, A4, B4, C5, E5, G5 = 60, 62, 64, 65, 67, 69, 71, 72, 76, 79
C3, E3, F3, G3, A3 = 48, 52, 53, 55, 57
C2, A2, F2, G2 = 36, 45, 41, 43


def arp(t0, notes, step, dur=1.5, vel=0.24, bright=0.95):
    for i, m in enumerate(notes):
        note(t0 + i * step, m, dur, vel=vel if m >= 60 else vel + 0.05, bright=bright)

B4, D5 = 71, 74
# 1 hook 0-3.4: the three rings draw (E4 G4 B4), then Cmaj7 opens under the wordmark
for i, m in enumerate([E4, G4, B4]): note(0.1 + i * 0.15, m, 1.8, vel=0.2)
chord(0.8, [C3, E4, G4, B4], 2.6, vel=0.32, spread=0.02, bright=0.8)
# 2 pair 3.4-7.2: Am7, one note per beat
arp(3.4, [A2, E4, G4, C5, E4], BEAT)
# 3 scores 7.2-11.0: Fmaj7, three high notes on the pills
arp(7.2, [F2, C4, E4, A4], BEAT)
for i, m in enumerate([C5, E5, G5]): note(8.5 + i * 0.25, m, 1.0, vel=0.16)
# 4 hub 11.0-15.2: G, eighth-note pulse builds to the "running" chime (13.6)
arp(11.0, [G2, D4, G4, B4, D4, G4, B4, D5], BEAT / 2, dur=1.0, vel=0.2)
chord(13.6, [C3, G4, C5, E5], 1.6, vel=0.3, spread=0.015, bright=1.05)
# 5 connect 15.2-19.6: Am -> F, lift at the tap (18.0)
arp(15.2, [A2, E4, A4, C5], BEAT)
chord(18.0, [F2, A4, C5, E5], 1.6, vel=0.3, spread=0.018)
# 6 backup 19.6-23.8: C -> G, the arpeggio opens up an octave with the counter
arp(19.6, [C3, E4, G4, C5, E5, G5, E5, C5], BEAT / 2, dur=1.2, vel=0.2, bright=1.05)
chord(22.3, [G2, D4, G4, B4], 1.5, vel=0.3, spread=0.015)
# 7 agent 23.8-28.2: Fmaj7 -> Gsus, the JSON lands at 26.2
arp(23.8, [F2, C4, E4, A4], BEAT)
chord(26.2, [G2, C4, D4, G4], 2.0, vel=0.3, spread=0.02)
# 8 end 28.2-32.0: resolve to Cmaj7, final note with the wordmark, decay into the fade
chord(28.3, [C2, C3, E4, G4, B4], 3.4, vel=0.38, spread=0.025, bright=0.85)
note(28.7, G5, 1.8, vel=0.14)
note(29.0, E5, 2.4, vel=0.22)

# global: soft low-pass via simple one-pole, fade out over the last 2.0s
y = np.zeros_like(out); a = 0.18
for i in range(1, len(out)):
    y[i] = y[i-1] + a * (out[i] - y[i-1])
out = 0.75 * y + 0.25 * out
fade = np.ones(len(out))
fs = int((TOTAL - 2.0) * SR); fade[fs:int(TOTAL*SR)] = np.linspace(1, 0, int(TOTAL*SR) - fs); fade[int(TOTAL*SR):] = 0
out *= fade
out = out[:int(TOTAL * SR)]
out = out / (np.max(np.abs(out)) + 1e-9) * 0.6

os.makedirs("audio/music", exist_ok=True)
with wave.open("audio/music/theme_raw.wav", "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
    w.writeframes((out * 32767).astype(np.int16).tobytes())
# room: short soft echo + final loudness
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", "audio/music/theme_raw.wav",
                "-t", str(TOTAL), "-af", "aecho=0.7:0.35:38|71:0.22|0.14,loudnorm=I=-19:TP=-2:LRA=9,afade=t=out:st=%s:d=0.2" % (TOTAL - 0.2),
                "-ar", str(SR), "audio/music/theme.wav"], check=True)
os.remove("audio/music/theme_raw.wav")
print("audio/music/theme.wav", subprocess.run(["ffprobe","-v","error","-show_entries","format=duration","-of","csv=p=0","audio/music/theme.wav"],capture_output=True,text=True).stdout.strip(), "s")
