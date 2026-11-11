// Kinetic text: words spring up one after another; every Nth word takes the highlight colour.
type Props = { text: string; color: string; highlight: string; every: number; size: number; uppercase: boolean };

export default function KineticText({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const words = props.text.split(/\s+/).filter(Boolean);
  const step = Math.min(0.3, (duration * 0.6) / Math.max(1, words.length));
  const out = 1 - easing.in(clamp((time - (duration - 0.4)) / 0.4));
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center", padding: 120, opacity: out }}>
      <div style={{ display: "flex", flexDirection: "row", flexWrap: "wrap", justifyContent: "center", alignItems: "baseline", width: 1500 }}>
        {words.map((w, i) => {
          const local = stagger(time, i, step);
          const p = spring({ time: local, stiffness: 260, damping: 16 });
          const hot = props.every > 0 && (i + 1) % props.every === 0;
          return (
            <span
              style={{
                display: "block",
                margin: "6px 16px",
                fontSize: props.size,
                fontWeight: 900,
                color: hot ? props.highlight : props.color,
                textShadow: "0 4px 24px rgba(0,0,0,0.35)",
                opacity: local > 0 ? clamp(p * 2) : 0,
                transform: `translateY(${(1 - p) * 50}px) scale(${0.6 + 0.4 * p})`,
              }}
            >
              {props.uppercase ? w.toUpperCase() : w}
            </span>
          );
        })}
      </div>
    </Fill>
  );
}
