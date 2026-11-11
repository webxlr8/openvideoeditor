// Progress bar: fills across the frame over the item's duration, with an optional label and percentage.
type Props = { label: string; color: string; track: string; position: string; thickness: number; showPercent: boolean };

export default function ProgressBar({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const p = clamp(time / Math.max(duration, 0.001));
  const top = props.position === "top";
  const label = props.label || props.showPercent;
  return (
    <Fill style={{ justifyContent: top ? "flex-start" : "flex-end" }}>
      <div style={{ display: "flex", flexDirection: top ? "column-reverse" : "column" }}>
        {label ? (
          <div style={{ display: "flex", flexDirection: "row", justifyContent: "space-between", padding: "14px 48px", fontSize: 30, fontWeight: 600, color: props.color }}>
            <span>{props.label}</span>
            <span>{props.showPercent ? `${Math.round(p * 100)}%` : ""}</span>
          </div>
        ) : null}
        <div style={{ display: "flex", width: "100%", height: props.thickness, backgroundColor: props.track }}>
          <div style={{ width: `${p * 100}%`, height: "100%", backgroundColor: props.color }} />
        </div>
      </div>
    </Fill>
  );
}
