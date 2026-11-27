// Stereo level meters next to the viewer: bar level, peak marker (held 1.5 s), clip indicator (lit until clicked),
// true peak and momentary / short-term loudness. Readings come from the engine in batches; each display frame shows
// the readings under the heard position. Drawing touches only the bars' styles, not React state, so a playing
// timeline costs one small style update per frame.

import { useEffect, useRef, useState } from "react";
import "./audio.css";
import { events } from "./bridge";
import { HeardReadings, METER_FLOOR_DB, type MeterState, idleMeters, resetClip, stepMeters } from "./meterBallistics";

/** Bar position (0–1) for a level in dB. */
export function meterFraction(db: number): number {
  return Math.min(1, Math.max(0, (db - METER_FLOOR_DB) / -METER_FLOOR_DB));
}

function fmt(v: number | null, unit: string): string {
  return v === null || v <= METER_FLOOR_DB ? `– ${unit}` : `${v.toFixed(1)} ${unit}`;
}

export function Meters() {
  const state = useRef<MeterState>(idleMeters());
  const heard = useRef(new HeardReadings());
  const bars = useRef<(HTMLDivElement | null)[]>([]);
  const peaks = useRef<(HTMLDivElement | null)[]>([]);
  const [clip, setClip] = useState<[boolean, boolean]>([false, false]);
  const [numbers, setNumbers] = useState<{ tp: number; m: number | null; s: number | null }>({ tp: METER_FLOOR_DB, m: null, s: null });

  useEffect(() => {
    let raf = 0;
    let last = performance.now();
    let lastText = 0;
    let unlisten: (() => void) | null = null;
    let live = true;
    // The frame loop runs only while there is something to show: from a batch until the bars and markers are back at
    // the floor, so idle meters cost nothing.
    const idle = (s: MeterState) => heard.current.heard(performance.now()) === null && s.channels.every((c) => c.level <= METER_FLOOR_DB && c.peak <= METER_FLOOR_DB);
    const frame = () => {
      // The same clock the batches are stamped with.
      const now = performance.now();
      const s = stepMeters(state.current, heard.current.take(now), now - last, now);
      last = now;
      state.current = s;
      for (let c = 0; c < 2; c++) {
        const bar = bars.current[c];
        const pk = peaks.current[c];
        if (bar) bar.style.transform = `scaleY(${meterFraction(s.channels[c].level)})`;
        if (pk) pk.style.bottom = `${meterFraction(s.channels[c].peak) * 100}%`;
      }
      setClip((old) => (old[0] === s.channels[0].clip && old[1] === s.channels[1].clip ? old : [s.channels[0].clip, s.channels[1].clip]));
      // Numbers change at most 4 times a second, so they stay readable.
      if (now - lastText > 250 || idle(s)) {
        lastText = now;
        setNumbers((o) => (o.tp === s.truePeakMax && o.m === s.momentary && o.s === s.shortTerm ? o : { tp: s.truePeakMax, m: s.momentary, s: s.shortTerm }));
      }
      raf = live && !idle(s) ? requestAnimationFrame(frame) : 0;
    };
    events.audioMeter
      .listen((e) => {
        heard.current.receive(e.payload, performance.now());
        if (live && raf === 0) {
          last = performance.now();
          raf = requestAnimationFrame(frame);
        }
      })
      .then(
        (u) => {
          if (live) unlisten = u;
          else u();
        },
        () => undefined,
      );
    return () => {
      live = false;
      cancelAnimationFrame(raf);
      unlisten?.();
    };
  }, []);

  const clear = () => {
    state.current = resetClip(state.current);
    setClip([false, false]);
    setNumbers((o) => ({ ...o, tp: METER_FLOOR_DB }));
  };

  return (
    <div className="meters" data-testid="meters" title="Levels (dBFS), true peak and loudness">
      <div className="meter-bars">
        {[0, 1].map((c) => (
          <div className="meter-channel" key={c}>
            <button
              type="button"
              className={`meter-clip${clip[c] ? " lit" : ""}`}
              data-testid={`meter-clip-${c}`}
              aria-label={clip[c] ? "Clipped: click to reset" : "No clipping"}
              onClick={clear}
            />
            <div className="meter-track">
              <div className="meter-bar" data-testid={`meter-bar-${c}`} ref={(el) => void (bars.current[c] = el)} />
              <div className="meter-peak" data-testid={`meter-peak-${c}`} ref={(el) => void (peaks.current[c] = el)} />
            </div>
          </div>
        ))}
      </div>
      <div className="meter-numbers small">
        <span data-testid="meter-tp" title="Highest true peak">{fmt(numbers.tp, "dBTP")}</span>
        <span data-testid="meter-m" title="Momentary loudness (400 ms)">{fmt(numbers.m, "M")}</span>
        <span data-testid="meter-s" title="Short-term loudness (3 s)">{fmt(numbers.s, "S")}</span>
      </div>
    </div>
  );
}
