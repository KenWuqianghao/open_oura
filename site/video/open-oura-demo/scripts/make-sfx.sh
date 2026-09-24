#!/usr/bin/env bash
# Synthesize a restrained SFX kit with ffmpeg. Deterministic, no samples.
# Usage: make-sfx.sh <project-dir> [typing_chars=29] [typing_seconds=1.1] [video_seconds=16.4]
# Output: <project-dir>/audio/sfx/{rush,tick,typing,chime,pop,pad}.wav  (48 kHz mono)
set -euo pipefail
PROJ="${1:?project dir}"; NCHARS="${2:-29}"; TDUR="${3:-1.1}"; VDUR="${4:-16.4}"
mkdir -p "$PROJ/audio/sfx"; cd "$PROJ/audio/sfx"
SR=48000
FADE_OUT_START=$(python3 -c "print(max(0.0, $VDUR - 3.0))")

ffmpeg -y -loglevel error -f lavfi -i "anoisesrc=color=pink:seed=7:sample_rate=$SR:duration=1.3" \
  -af "bandpass=f=420:width_type=o:width=1.4,volume='0.18+0.32*t/1.3':eval=frame,afade=t=in:st=0:d=0.05" rush.wav

ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=1800:sample_rate=$SR:duration=0.06" \
  -af "afade=t=out:st=0.005:d=0.055:curve=exp,volume=0.5" tick.wav

python3 - "$NCHARS" "$TDUR" <<'EOF'
import sys, math, struct, wave, random
N=int(sys.argv[1]); DUR=float(sys.argv[2]); SR=48000
rng=random.Random(3); frames=int((DUR+0.15)*SR); buf=[0.0]*frames
for i in range(N):
    n0=int(i/N*DUR*SR); L=int(0.012*SR); amp=0.22+rng.random()*0.08
    for k in range(L):
        buf[n0+k]+=amp*math.exp(-k/(L*0.22))*(rng.random()*2-1)
with wave.open("typing_raw.wav","wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
    w.writeframes(b"".join(struct.pack("<h",int(max(-1,min(1,s))*32767)) for s in buf))
EOF
ffmpeg -y -loglevel error -i typing_raw.wav -af "bandpass=f=3000:width_type=o:width=1.2,volume=1.6" typing.wav && rm typing_raw.wav

ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=1046.5:sample_rate=$SR:duration=1.6" -f lavfi -i "sine=frequency=1568:sample_rate=$SR:duration=1.6" \
  -filter_complex "[0:a]volume=0.35[a];[1:a]volume=0.18,adelay=40|40[b];[a][b]amix=inputs=2:normalize=0,afade=t=in:st=0:d=0.01,afade=t=out:st=0.05:d=1.55:curve=exp" chime.wav

ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=330:sample_rate=$SR:duration=0.18" \
  -af "afade=t=out:st=0.01:d=0.17:curve=exp,lowpass=f=900,volume=0.6" pop.wav

ffmpeg -y -loglevel error \
  -f lavfi -i "sine=frequency=110:sample_rate=$SR:duration=$VDUR" -f lavfi -i "sine=frequency=164.81:sample_rate=$SR:duration=$VDUR" \
  -f lavfi -i "sine=frequency=277.18:sample_rate=$SR:duration=$VDUR" -f lavfi -i "sine=frequency=110.6:sample_rate=$SR:duration=$VDUR" \
  -filter_complex "[0:a][1:a][2:a][3:a]amix=inputs=4:normalize=0,lowpass=f=600,tremolo=f=0.18:d=0.25,afade=t=in:st=0:d=2.5,afade=t=out:st=$FADE_OUT_START:d=3.0,volume=0.22" pad.wav

for f in *.wav; do printf "%-12s %6ss\n" "$f" "$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f")"; done
