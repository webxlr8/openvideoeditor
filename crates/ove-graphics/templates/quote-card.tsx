// Quote card: a card lifts in, the quote appears word by word, then the attribution.
type Props = { quote: string; author: string; card: string; color: string; accent: string };

export default function QuoteCard({ time, duration, props }: { time: number; duration: number; props: Props }) {
  const words = props.quote.split(/\s+/).filter(Boolean);
  const step = Math.min(0.08, 2 / Math.max(1, words.length));
  const card = easing.out(clamp(time / 0.5));
  const out = 1 - easing.in(clamp((time - (duration - 0.5)) / 0.5));
  const by = easing.out(clamp((time - 0.4 - words.length * step) / 0.5));
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center", opacity: out }}>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          width: 1320,
          padding: "64px 88px 72px",
          borderRadius: 32,
          backgroundColor: props.card,
          boxShadow: "0 30px 80px rgba(0,0,0,0.35)",
          opacity: card,
          transform: `translateY(${(1 - card) * 40}px) scale(${0.96 + 0.04 * card})`,
        }}
      >
        <div style={{ width: 96, height: 10, borderRadius: 5, backgroundColor: props.accent, marginBottom: 40 }} />
        <div style={{ display: "flex", flexDirection: "row", flexWrap: "wrap", fontFamily: "Playfair Display", fontSize: 60, lineHeight: 1.3, color: props.color }}>
          {words.map((w, i) => {
            const p = easing.out(clamp(stagger(time, i, step, 0.3) / 0.4));
            return <span style={{ display: "block", marginRight: 18, opacity: p, transform: `translateY(${(1 - p) * 16}px)` }}>{w}</span>;
          })}
        </div>
        <div style={{ marginTop: 40, fontSize: 34, fontWeight: 600, letterSpacing: 2, color: props.accent, opacity: by }}>{props.author}</div>
      </div>
    </Fill>
  );
}
