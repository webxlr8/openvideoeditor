// Counter: a number counts from `from` to `to` with an ease-out, popping in on a spring, with a caption below.
type Props = { from: number; to: number; seconds: number; decimals: number; prefix: string; suffix: string; label: string; color: string };

const group = (digits: string) => digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",");

export default function Counter({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const run = Math.max(0.3, Math.min(props.seconds, duration));
  const value = mix(props.from, props.to, easing.out(clamp(time / run)));
  const [whole, frac] = Math.abs(value).toFixed(Math.max(0, Math.min(4, props.decimals))).split(".");
  const text = (value < 0 ? "-" : "") + props.prefix + group(whole) + (frac ? "." + frac : "") + props.suffix;
  const pop = spring({ time, from: 0.7, to: 1, stiffness: 220, damping: 14 });
  const out = 1 - easing.in(clamp((time - (duration - 0.4)) / 0.4));
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center", opacity: out }}>
      <div style={{ fontSize: 210, fontWeight: 800, color: props.color, transform: `scale(${pop})`, textShadow: "0 6px 30px rgba(0,0,0,0.35)" }}>
        {text}
      </div>
      <div style={{ marginTop: 8, fontSize: 46, fontWeight: 500, color: props.color, opacity: 0.8 * easing.out(clamp((time - 0.2) / 0.5)) }}>
        {props.label}
      </div>
    </Fill>
  );
}
