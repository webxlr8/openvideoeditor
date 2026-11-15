// The icon shown next to an assistant: its own logo (logos.tsx) on a neutral tile, so a one-colour logo reads on the
// dark window. An entry without a logo (the built-in assistant on an API key, or an agent whose logo was dropped)
// gets a lettered tile drawn for this app.
import "./agents.css";
import { LOGOS } from "./logos";

const LETTERS: Record<string, { letters: string; hue: number }> = {
  claude: { letters: "C", hue: 22 },
  codex: { letters: "Cx", hue: 158 },
  opencode: { letters: "Oc", hue: 208 },
  hermes: { letters: "H", hue: 276 },
  kimi: { letters: "K", hue: 336 },
  grok: { letters: "G", hue: 48 },
  "api-key": { letters: "API", hue: 240 },
};

export function AgentIcon({ id, name, size = 28 }: { id: string; name: string; size?: number }) {
  const logo = LOGOS[id];
  if (logo) {
    return (
      <span className="agent-icon logo" data-testid={`agent-icon-${id}`} style={{ width: size, height: size }}>
        {logo(Math.round(size * 0.68))}
      </span>
    );
  }
  const look = LETTERS[id] ?? { letters: name.slice(0, 1).toUpperCase(), hue: 0 };
  return (
    <span
      className="agent-icon"
      data-testid={`agent-icon-${id}`}
      aria-hidden
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * (look.letters.length > 2 ? 0.32 : 0.42)),
        background: `hsl(${look.hue} 42% 30%)`,
        borderColor: `hsl(${look.hue} 55% 46%)`,
      }}
    >
      {look.letters}
    </span>
  );
}
