// Lower third: an accent bar grows, then the name and role slide in beside it; both leave before the end.
type Props = { name: string; role: string; accent: string; color: string; plate: string; side: string };

export default function LowerThird({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const exit = easing.in(clamp((time - (duration - 0.5)) / 0.5));
  const bar = easing.out(clamp(time / 0.45)) * (1 - exit);
  const text = easing.out(clamp((time - 0.2) / 0.5)) * (1 - exit);
  const right = props.side === "right";
  const shift = (1 - text) * (right ? 48 : -48);
  return (
    <Fill style={{ justifyContent: "flex-end", alignItems: right ? "flex-end" : "flex-start", padding: "0 120px 110px" }}>
      <div style={{ display: "flex", flexDirection: right ? "row-reverse" : "row", alignItems: "stretch" }}>
        <div style={{ width: 12, borderRadius: 6, backgroundColor: props.accent, transform: `scaleY(${bar})` }} />
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: right ? "flex-end" : "flex-start",
            margin: right ? "0 22px 0 0" : "0 0 0 22px",
            padding: "18px 30px",
            borderRadius: 14,
            backgroundColor: props.plate,
            opacity: text,
            transform: `translateX(${shift}px)`,
          }}
        >
          <div style={{ fontSize: 62, fontWeight: 800, color: props.color, lineHeight: 1.1 }}>{props.name}</div>
          <div style={{ fontSize: 32, fontWeight: 500, color: props.color, opacity: 0.8, marginTop: 6 }}>{props.role}</div>
        </div>
      </div>
    </Fill>
  );
}
