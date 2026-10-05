#!/usr/bin/env python3
"""Export SleepECG's `wrn-gru-mesa` sleep classifier for `oura-analysis`.

SleepECG (BSD-3-Clause, https://github.com/cbrnr/sleepecg) ships a small Keras
model that stages sleep as wake / REM / non-REM from heartbeat times. It was
trained on 1971 MESA nights. `oura-analysis::insights::open_sleep` runs the same
model in pure Rust, so a client without the Oura models still gets a hypnogram.

This script writes two files next to the Rust module:

* `wrn_gru_mesa.bin`: every weight as little-endian f32, in the order that
  `open_sleep::net` reads them (see `LAYOUT` below).
* `golden.json`: synthetic nights with SleepECG's own features and class
  probabilities. The Rust tests compare against them.

Usage (any Python with `sleepecg`, `keras` and `torch`):
    KERAS_BACKEND=torch python tools/export_sleepecg.py
"""
import datetime
import json
import os
from pathlib import Path

os.environ.setdefault("KERAS_BACKEND", "torch")

import numpy as np
import sleepecg
from sleepecg import SleepRecord, extract_features, load_classifier, stage
from sleepecg.io.sleep_readers import SubjectData

OUT = Path(__file__).resolve().parent.parent / "crates/oura-analysis/src/insights/open_sleep"
NAME = "wrn-gru-mesa"

# (layer index in the Keras model, weight names) in file order.
LAYOUT = [
    ("batch_normalization", ["gamma", "beta", "moving_mean", "moving_variance"]),
    ("dense", ["kernel", "bias"]),
    ("bidirectional", ["fwd kernel", "fwd recurrent", "fwd bias", "bwd kernel", "bwd recurrent", "bwd bias"]),
    ("bidirectional_1", ["fwd kernel", "fwd recurrent", "fwd bias", "bwd kernel", "bwd recurrent", "bwd bias"]),
    ("dense_1", ["kernel", "bias"]),
]


def export_weights(clf) -> int:
    layers = [l for l in clf.model.layers if l.get_weights()]
    assert [type(l).__name__ for l in layers] == [
        "BatchNormalization", "Dense", "Bidirectional", "Bidirectional", "Dense"
    ], [type(l).__name__ for l in layers]
    flat = []
    for layer in layers:
        for w in layer.get_weights():
            flat.append(np.asarray(w, dtype="<f4").ravel(order="C"))
    data = np.concatenate(flat)
    (OUT / "wrn_gru_mesa.bin").write_bytes(data.tobytes())
    return data.size


def synthetic_night(seed: int, minutes: int, gaps: bool) -> np.ndarray:
    """Heartbeat times (s) with breathing, slow drift, wake bursts and artifacts."""
    rng = np.random.RandomState(seed)
    t, times = 0.0, []
    while t < minutes * 60 + 30:
        phase = t / 60.0
        base = 1.0 + 0.08 * np.sin(2 * np.pi * phase / 90.0)  # slow cycle
        if 20 <= phase % 45 < 24:  # wake-like burst: faster, noisier
            base -= 0.25
        rsa = 0.05 * np.sin(2 * np.pi * 0.25 * t)
        rri = base + rsa + rng.normal(0, 0.03)
        t += rri
        times.append(t)
    times = np.array(times)
    if gaps:
        # a 3-minute drop-out and a few artifacts the RRI bounds must reject
        times = times[(times < 600) | (times > 780)]
        times = np.insert(times, 50, times[49] + 0.1)
        times = np.insert(times, 400, times[399] + 0.15)
    # millisecond resolution, like the ring's IBI stream
    return np.round(times, 3)


def golden(clf) -> list[dict]:
    cases = [
        dict(seed=1, minutes=40, gaps=False, start=(23, 30, 0), age=35, gender=1),
        dict(seed=2, minutes=60, gaps=True, start=(1, 5, 30), age=62, gender=0),
        dict(seed=3, minutes=12, gaps=True, start=(22, 0, 0), age=None, gender=None),
    ]
    out = []
    params = clf.feature_extraction_params
    for case in cases:
        beats = synthetic_night(case["seed"], case["minutes"], case["gaps"])
        record = SleepRecord(
            heartbeat_times=beats,
            recording_start_time=datetime.time(*case["start"]),
            subject_data=SubjectData(gender=case["gender"], age=case["age"]),
        )
        features, _, ids = extract_features([record], **params)
        features = features[0]
        probs = stage(clf, record, return_mode="prob")
        h, m, s = case["start"]
        out.append(
            {
                "heartbeat_times": beats.tolist(),
                "epochs": int(features.shape[0]),
                "start_sec_of_day": h * 3600 + m * 60 + s,
                "age": case["age"],
                "gender": case["gender"],
                "feature_ids": ids,
                "features": [[float(f"{v:.12g}") if np.isfinite(v) else None for v in row] for row in features],
                "probs": [[float(f"{p:.9g}") for p in row] for row in np.asarray(probs, dtype=float)],
            }
        )
    return out


def main():
    clf = load_classifier(NAME, "SleepECG")
    assert clf.stages_mode == "wake-rem-nrem" and clf.mask_value == -1
    print(f"sleepecg {sleepecg.__version__}: {clf}")
    print(f"weights: {export_weights(clf)} f32 values")
    cases = golden(clf)
    (OUT / "golden.json").write_text(json.dumps({"sleepecg": sleepecg.__version__, "cases": cases}))
    print(f"golden: {len(cases)} nights, {sum(c['epochs'] for c in cases)} epochs")


if __name__ == "__main__":
    main()
