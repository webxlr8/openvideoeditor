#!/usr/bin/env bash
# Makes the sample project's clip (sample.mp4, next to this script). Everything in it is generated here: moving
# colour gradients and plain shapes drawn by FFmpeg filters, and two spoken lines from the espeak-ng synthesiser.
# No footage, music or recorded voice is used, so the file can ship with the app.
#
# Needs: ffmpeg with libx264 and the native AAC encoder, espeak-ng. Run it again only to change the clip; the
# times below are also written into ../src/sample.rs (the cut, the title and the captions), so keep them in step.
#
#   0.0 – 3.4 s   scene one (warm), "Welcome to Open Video Editor." spoken from 0.5 s
#   3.4 – 7.0 s   scene two (cool), "Cut it, add a title, then export." spoken from 3.6 s
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${1:-$here/sample.mp4}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

espeak-ng -v en-us -s 150 -w "$work/line1.wav" "Welcome to Open Video Editor."
espeak-ng -v en-us -s 150 -w "$work/line2.wav" "Cut it, add a title, then export."

# A two-colour diagonal wave that drifts, tinted towards a third colour at the bottom. Drawn small and scaled up:
# the picture is smooth, and it encodes to very little.
wave="0.5+0.5*sin(2*PI*(X/W*0.55+Y/H*0.45)+T*0.9)"
tint="(Y/H)*(0.6+0.4*sin(T*0.7+X/W*3))"
mix() { echo "(($1)*(1-($wave))+($2)*($wave))*(1-0.55*($tint))+($3)*0.55*($tint)"; }
gradient() { # seconds, then three colours as r g b triples
  echo "color=c=black:s=96x54:r=30:d=$1,format=gbrp,geq=r='$(mix "$2" "$5" "$8")':g='$(mix "$3" "$6" "$9")':b='$(mix "$4" "$7" "${10}")',scale=960:540:flags=bicubic"
}

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "$(gradient 3.4 255 107 90 242 193 78 90 24 150)" \
  -f lavfi -i "$(gradient 3.6 11 19 43 58 134 255 6 214 160)" \
  -i "$work/line1.wav" -i "$work/line2.wav" \
  -filter_complex "
    color=c=white@0.0:s=144x144:r=30,format=rgba,geq=r=255:g=255:b=255:a='255*clip(66-hypot(X-72,Y-72),0,1)*0.93'[dot];
    color=c=white@0.0:s=240x240:r=30,format=rgba,geq=r=255:g=255:b=255:a='255*clip(114-hypot(X-120,Y-120),0,1)*clip(hypot(X-120,Y-120)-98,0,1)*0.8'[ring];
    [0:v][ring]overlay=x='360+90*sin(t*1.4)':y='150+30*cos(t*1.1)':shortest=1[a1];
    [a1][dot]overlay=x='120+t*195':y='315-135*abs(sin(t*2.6))':shortest=1[one];
    color=c=white@0.0:s=60x330:r=30,format=rgba,geq=r=255:g=255:b=255:a=215,split=3[b1][b2][b3];
    [1:v][b1]overlay=x=300:y='105+75*sin(t*2.2)':shortest=1[c1];
    [c1][b2]overlay=x=450:y='105+75*sin(t*2.2+1.2)':shortest=1[c2];
    [c2][b3]overlay=x=600:y='105+75*sin(t*2.2+2.4)':shortest=1[two];
    [one][two]concat=n=2:v=1:a=0,format=yuv420p[v];
    [2:a]adelay=500:all=1[l1];
    [3:a]adelay=3600:all=1[l2];
    [l1][l2]amix=inputs=2:normalize=0,aresample=48000,apad=whole_dur=7[a]" \
  -map "[v]" -map "[a]" -t 7 \
  -c:v libx264 -preset veryslow -crf 24 -profile:v high -pix_fmt yuv420p -g 30 \
  -c:a aac -b:a 64k -ac 1 \
  -movflags +faststart -map_metadata -1 -fflags +bitexact -flags:v +bitexact -flags:a +bitexact \
  "$out"

echo "$out: $(wc -c <"$out") bytes"
