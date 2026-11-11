// Logo sting: a ring bursts outwards while the logo springs in, then a tagline settles underneath.
type Props = { tagline: string; background: string; accent: string; color: string };
type Input = { time: number; duration: number; width: number; height: number; assets: { logo?: string }; props: Props };

export default function LogoSting({ time, duration, width, height, assets, props }: Input) {
  const s = spring({ time, delay: 0.1, stiffness: 140, damping: 11 });
  const ring = easing.out(clamp((time - 0.15) / 0.9));
  const tag = easing.out(clamp((time - 0.7) / 0.5));
  const out = 1 - easing.in(clamp((time - (duration - 0.4)) / 0.4));
  const size = 320 + ring * 760;
  return (
    <Fill style={{ alignItems: "center", justifyContent: "center", backgroundColor: props.background, opacity: out }}>
      <div
        style={{
          position: "absolute",
          left: width / 2 - size / 2,
          top: height / 2 - size / 2 - 40,
          width: size,
          height: size,
          borderRadius: size / 2,
          border: `${Math.max(1, 16 * (1 - ring))}px solid ${props.accent}`,
          opacity: 1 - ring,
        }}
      />
      {assets.logo ? (
        <img src={assets.logo} style={{ width: 360, height: 360, objectFit: "contain", transform: `scale(${s})` }} />
      ) : (
        <div style={{ width: 220, height: 220, borderRadius: 48, backgroundColor: props.accent, transform: `scale(${s}) rotate(${(1 - s) * -90}deg)` }} />
      )}
      <div style={{ marginTop: 44, fontSize: 52, fontWeight: 700, letterSpacing: 4, color: props.color, opacity: tag, transform: `translateY(${(1 - tag) * 24}px)` }}>
        {props.tagline}
      </div>
    </Fill>
  );
}
