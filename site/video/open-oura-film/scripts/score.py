#!/usr/bin/env python3
"""The score for the Open Oura film: 49 s, stereo, 48 kHz, written to the picture.

Every cue sits on a time in index.html (the heartbeat trace, the title, the portal,
the phone, the carousel turns, the widgets, the live beats, the finder pings, the
terminal, the finale). Deterministic: no randomness without a fixed seed.
Run from the film folder: python3 scripts/score.py  ->  audio/score.wav
"""
import numpy as np, wave, os

SR = 48000
DUR = 49.0
N = int(DUR * SR)
L = np.zeros(N); R = np.zeros(N)
rng = np.random.default_rng(11)
t_all = np.arange(N) / SR

def midi(m): return 440.0 * 2 ** ((m - 69) / 12)

def put(sig, t0, gain=1.0, pan=0.0):
    s = int(t0 * SR)
    if s >= N: return
    e = min(N, s + len(sig))
    seg = sig[: e - s] * gain
    L[s:e] += seg * np.sqrt(0.5 * (1 - pan)); R[s:e] += seg * np.sqrt(0.5 * (1 + pan))

def env_adsr(n, a, d, s_lvl, r):
    a, d, r = int(a * SR), int(d * SR), int(r * SR)
    sus = max(0, n - a - d - r)
    return np.concatenate([np.linspace(0, 1, a, endpoint=False), np.linspace(1, s_lvl, d, endpoint=False),
                           np.full(sus, s_lvl), np.linspace(s_lvl, 0, r)])[:n]

def onepole(x, a):
    y = np.empty_like(x); acc = 0.0
    for i in range(len(x)):  # small arrays only
        acc += a * (x[i] - acc); y[i] = acc
    return y

def lowpass(x, cutoff):
    # FFT brick-ish low-pass with a soft knee (fast for long pads).
    X = np.fft.rfft(x); f = np.fft.rfftfreq(len(x), 1 / SR)
    X *= 1 / (1 + (f / cutoff) ** 4)
    return np.fft.irfft(X, len(x))

# ── pad: detuned saw stacks, slow attack, low-passed ──
def pad(notes, t0, dur, gain=0.18, cutoff=1400, pan_spread=0.5):
    n = int(dur * SR); t = np.arange(n) / SR
    for j, m in enumerate(notes):
        for det, pan in ((-0.07, -pan_spread), (0.0, 0.0), (0.07, pan_spread)):
            f = midi(m + det)
            ph = (t * f) % 1.0
            saw = 2 * ph - 1
            sig = lowpass(saw, cutoff) * env_adsr(n, min(1.8, dur * 0.35), 0.5, 0.85, min(2.0, dur * 0.35))
            put(sig, t0, gain / len(notes) / 1.6, pan)

def sine_note(m, t0, dur, gain=0.2, pan=0.0, decay=0.35, harm=(1, 0.3, 0.12)):
    n = int(dur * SR); t = np.arange(n) / SR; f = midi(m)
    sig = sum(h * np.sin(2 * np.pi * f * (k + 1) * t) for k, h in enumerate(harm))
    sig *= np.exp(-t / (dur * decay)) * (1 - np.exp(-t / 0.004))
    put(sig, t0, gain, pan)

def thump(t0, gain=0.9, f0=62, f1=38):
    n = int(0.42 * SR); t = np.arange(n) / SR
    f = f1 + (f0 - f1) * np.exp(-t / 0.05)
    ph = 2 * np.pi * np.cumsum(f) / SR
    sig = np.sin(ph) * np.exp(-t / 0.13) * (1 - np.exp(-t / 0.002))
    put(sig, t0, gain)
    put(sig * 0.55, t0 + 0.16, gain * 0.6)  # the second sound of a heartbeat

def kick(t0, gain=0.55):
    n = int(0.3 * SR); t = np.arange(n) / SR
    f = 45 + 110 * np.exp(-t / 0.03)
    sig = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t / 0.12)
    put(sig, t0, gain)

def noise_burst(t0, dur, gain, cutoff_from, cutoff_to, pan=0.0, rise=False):
    n = int(dur * SR); t = np.arange(n) / SR
    x = rng.standard_normal(n)
    # sweep a one-pole by blocks (fast enough)
    y = np.zeros(n); acc = 0.0; block = 256
    for b in range(0, n, block):
        frac = b / n
        c = cutoff_from + (cutoff_to - cutoff_from) * frac
        a = 1 - np.exp(-2 * np.pi * c / SR)
        seg = x[b:b + block]; out = np.empty_like(seg)
        for i, v in enumerate(seg): acc += a * (v - acc); out[i] = acc
        y[b:b + block] = out
    e = (t / dur) ** 2 if rise else np.exp(-t / (dur * 0.35))
    put(y * e / (np.max(np.abs(y)) + 1e-9), t0, gain, pan)

def whoosh(t_peak, dur=1.2, gain=0.25):
    noise_burst(t_peak - dur * 0.8, dur, gain, 300, 5000, rise=False)

def riser(t_end, dur=1.2, gain=0.3):
    noise_burst(t_end - dur, dur, gain, 200, 8000, rise=True)

def tick(t0, gain=0.25, f=2400, pan=0.0):
    n = int(0.05 * SR); t = np.arange(n) / SR
    put(np.sin(2 * np.pi * f * t) * np.exp(-t / 0.008), t0, gain, pan)

def pop(t0, m=76, gain=0.35):
    sine_note(m, t0, 0.35, gain, 0.0, 0.25, (1, 0.5, 0.2))
    sine_note(m + 7, t0 + 0.03, 0.3, gain * 0.5, 0.2, 0.25)

def ping(t0, m=84, gain=0.28):
    sine_note(m, t0, 1.6, gain, -0.2, 0.45, (1, 0.15))
    sine_note(m, t0 + 0.28, 1.2, gain * 0.35, 0.3, 0.45, (1,))

def impact(t0, gain=0.8):
    thump(t0, gain, 70, 30)
    noise_burst(t0, 1.6, gain * 0.35, 6000, 400)

# ════════════════════ the cue sheet (times match index.html) ════════════════════
D, F, A, C, E, G, B = 50, 53, 57, 60, 64, 67, 71   # D3 F3 A3 C4 E4 G4 B4

# 1 · heartbeat (0–5): low D drone, a thump on every drawn beat
pad([D - 12, A - 12, D], 0.0, 5.4, gain=0.22, cutoff=500)
for b in [0.99, 1.51, 2.04, 2.56, 3.09]: thump(b, 0.75)
riser(6.0, 1.6, 0.22)

# 2 · title (5–10): Fmaj7 swell, impact on the title, soft beats with the LEDs
impact(6.0, 0.85)
pad([F - 12, C, E, A], 5.6, 4.8, gain=0.26, cutoff=1800)
for b in [5.2, 6.15, 7.1, 8.05, 9.0]: thump(b, 0.35)
for i, m in enumerate([A + 12, C + 12, E + 12, G + 12, E + 12, C + 12]):
    sine_note(m, 6.6 + i * 0.42, 1.6, 0.09, (-0.5 + i * 0.2), 0.4)

# 3 · the portal (10–15): Am9, a swarm of tiny blips as the particles lay out
pad([A - 24, E - 12, G, B, C + 12], 9.8, 5.4, gain=0.24, cutoff=2600)
whoosh(10.3, 1.4, 0.22)
bl = np.random.default_rng(3)
for k in range(90):
    tt = 10.4 + (k / 90) ** 0.8 * 2.8
    m = int(bl.choice([69, 72, 76, 79, 81, 84, 88]))
    sine_note(m, tt, 0.18, 0.035, float(bl.uniform(-0.8, 0.8)), 0.3, (1, 0.2))

# 4 · into the phone (15–20): pulse starts, whoosh as the phone arrives
whoosh(15.8, 1.3, 0.28)
pad([C - 12, G - 12, E, G, D + 12], 15.0, 5.2, gain=0.24, cutoff=2200)
beat = 60 / 96
for i in range(8): kick(15.6 + i * beat, 0.42)
for i, m in enumerate([C, E, G, D + 12, E + 12, D + 12, G, E]):
    sine_note(m + 12, 15.6 + i * beat, 0.5, 0.07, (-0.4 if i % 2 else 0.4), 0.3)

# 5 · carousel (20–27): driving plucks, a tick and a swish on every turn
pad([A - 12, E, G, C + 12], 20.0, 3.5, gain=0.2, cutoff=2400)
pad([F - 12, C, E, A], 23.5, 3.6, gain=0.2, cutoff=2400)
for i in range(11): kick(20.0 + i * beat, 0.36)
arp = [A, C + 12, E + 12, A + 12, E + 12, C + 12, F, A, C + 12, F + 12, C + 12, A]
for i in range(22):
    sine_note(arp[i % len(arp)] + 12, 20.0 + i * beat / 2 * 1.0, 0.28, 0.06, (-0.5 if i % 2 else 0.5), 0.25, (1, 0.35, 0.1))
for tt in [21.0, 22.35, 23.7, 25.05]:
    tick(tt, 0.2, 3200); noise_burst(tt - 0.1, 0.5, 0.12, 800, 6000)

# 6 · widgets (27–31.5): G brightens, pops as each widget lands, rings shimmer up
pad([G - 12, D, G, B, D + 12], 26.9, 4.8, gain=0.26, cutoff=3200)
impact(27.2, 0.5)
pop(27.3, 79); pop(28.05, 76); pop(28.25, 83)
for i, m in enumerate([67, 71, 74, 79, 83, 86]): sine_note(m + 12, 27.6 + i * 0.1, 0.5, 0.05, 0.3 - i * 0.12, 0.3)

# 7 · live heart rate (31.5–35.5): the music drops away to the heartbeat itself
pad([E - 24, B - 12, E, G], 31.4, 4.3, gain=0.16, cutoff=900)
for b in [31.9, 32.54, 33.18, 33.82, 34.46, 35.1]: thump(b, 0.95)

# 8 · ring page (35.5–39.5): sonar pings with the finder ripples
pad([D - 12, A - 12, F, A, E + 12], 35.4, 4.3, gain=0.2, cutoff=2000)
for k in range(3): ping(36.0 + k * 0.6, 84 + (k == 2) * 3, 0.24)

# 9 · hub + agent (39.5–44): typing, a chime when the tool answers
pad([F - 12, C, A, C + 12, E + 12], 39.4, 4.8, gain=0.2, cutoff=2400)
whoosh(39.9, 1.2, 0.2)
q = 'claude mcp add --transport http health https://…/mcp/••••'
for i in range(len(q)): tick(41.3 + i * 0.022, 0.06, 1800 + (i * 37 % 900), (-0.3 if i % 2 else 0.3))
sine_note(84, 42.7, 1.4, 0.14, 0.0, 0.4); sine_note(91, 42.74, 1.2, 0.08, 0.2, 0.4)
riser(44.3, 1.4, 0.26)

# 10 · finale (44–49): the resolve, wide C add9, a last shimmer, fade
impact(44.3, 0.9)
pad([C - 24, C - 12, G - 12, E, G, D + 12, E + 12], 44.1, 4.9, gain=0.34, cutoff=3800, pan_spread=0.8)
for i, m in enumerate([72, 76, 79, 84, 86, 88]): sine_note(m + 12, 45.2 + i * 0.16, 2.2, 0.05, -0.6 + i * 0.24, 0.45)

# ── reverb: a synthetic hall impulse (seeded), convolved by FFT ──
ir_len = int(2.6 * SR); ti = np.arange(ir_len) / SR
irr = np.random.default_rng(5)
irL = irr.standard_normal(ir_len) * np.exp(-ti / 0.7); irR = irr.standard_normal(ir_len) * np.exp(-ti / 0.72)
irL = lowpass(irL, 5000); irR = lowpass(irR, 5000)
def conv(x, h):
    n = 1 << int(np.ceil(np.log2(len(x) + len(h))))
    return np.fft.irfft(np.fft.rfft(x, n) * np.fft.rfft(h, n), n)[: len(x)]
wetL, wetR = conv(L, irL), conv(R, irR)
wetL /= np.max(np.abs(wetL)) + 1e-9; wetR /= np.max(np.abs(wetR)) + 1e-9
dryL, dryR = L / (np.max(np.abs(L)) + 1e-9), R / (np.max(np.abs(R)) + 1e-9)
outL = 0.78 * dryL + 0.34 * wetL; outR = 0.78 * dryR + 0.34 * wetR

# fade the last 1.5 s, soft-clip, normalize to -1 dBFS
fade = np.ones(N); fs = int((DUR - 1.5) * SR); fade[fs:] = np.linspace(1, 0, N - fs)
outL *= fade; outR *= fade
outL, outR = np.tanh(outL * 1.2), np.tanh(outR * 1.2)
peak = max(np.max(np.abs(outL)), np.max(np.abs(outR)))
outL, outR = outL / peak * 0.89, outR / peak * 0.89

os.makedirs('audio', exist_ok=True)
pcm = (np.stack([outL, outR], axis=1) * 32767).astype(np.int16)
with wave.open('audio/score.wav', 'wb') as w:
    w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR); w.writeframes(pcm.tobytes())
print('audio/score.wav', f'{DUR:.1f} s')
