// Title card: a gradient field, the title rising letter by letter with a slight overshoot, then the subtitle.
type Props = { title: string; subtitle: string; color: string; from: string; to: string };

export default function TitleCard({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const letters = Array.from(props.title);
  const step = Math.min(0.05, 1.2 / Math.max(1, letters.length));
  const out = 1 - easing.in(clamp((time - (duration - 0.6)) / 0.6));
  const sub = easing.out(clamp((time - 0.3 - letters.length * step) / 0.6));
  return (
    <Fill
      style={{
        alignItems: "center",
        justifyContent: "center",
        backgroundImage: `linear-gradient(135deg, ${props.from}, ${props.to})`,
        opacity: out,
      }}
    >
      <div style={{ display: "flex", flexDirection: "row", fontSize: 150, fontWeight: 800, color: props.color, letterSpacing: -3 }}>
        {letters.map((ch, i) => {
          const p = easing.back(clamp(stagger(time, i, step, 0.1) / 0.55));
          return (
            <span style={{ display: "block", whiteSpace: "pre", opacity: clamp(p * 1.6), transform: `translateY(${(1 - p) * 70}px)` }}>
              {ch}
            </span>
          );
        })}
      </div>
      <div style={{ marginTop: 28, fontSize: 40, fontWeight: 500, letterSpacing: 8, color: props.color, opacity: sub * 0.85 }}>
        {props.subtitle.toUpperCase()}
      </div>
    </Fill>
  );
}
