// End card: heading, two placeholder frames for follow-up videos, and a call-to-action pill.
type Props = { heading: string; first: string; second: string; cta: string; from: string; to: string; accent: string; color: string; ctaColor: string };

export default function EndCard({ time, width, height, props }: { time: number; width: number; height: number; props: Props }) {
  // Side by side on a wide frame, stacked on a tall one.
  const tall = height > width;
  const frameW = tall ? Math.min(760, width - 160) : 560;
  const bg = easing.out(clamp(time / 0.6));
  const head = easing.out(clamp((time - 0.2) / 0.6));
  const frames = [props.first, props.second];
  const cta = spring({ time, delay: 1.0, stiffness: 180, damping: 12 });
  return (
    <Fill
      style={{
        alignItems: "center",
        justifyContent: "center",
        backgroundImage: `linear-gradient(160deg, ${props.from}, ${props.to})`,
        opacity: bg,
      }}
    >
      <div style={{ fontSize: 92, fontWeight: 800, color: props.color, opacity: head, transform: `translateY(${(1 - head) * 30}px)` }}>
        {props.heading}
      </div>
      <div style={{ display: "flex", flexDirection: tall ? "column" : "row", margin: tall ? "64px 0" : "56px 0" }}>
        {frames.map((label, i) => {
          const p = easing.back(clamp(stagger(time, i, 0.15, 0.5) / 0.6));
          return (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                width: frameW,
                height: (frameW * 9) / 16,
                margin: tall ? "24px 0" : "0 24px",
                borderRadius: 24,
                border: `4px solid ${props.accent}`,
                backgroundColor: "rgba(255,255,255,0.06)",
                fontSize: 36,
                fontWeight: 600,
                color: props.color,
                opacity: clamp(p),
                transform: `scale(${0.8 + 0.2 * p})`,
              }}
            >
              {label}
            </div>
          );
        })}
      </div>
      <div style={{ padding: "22px 60px", borderRadius: 60, backgroundColor: props.accent, color: props.ctaColor, fontSize: 44, fontWeight: 800, transform: `scale(${cta})` }}>
        {props.cta}
      </div>
    </Fill>
  );
}
