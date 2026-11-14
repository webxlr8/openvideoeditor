// The icon shown next to an assistant. Until a vendor's brand terms are confirmed, each agent has an icon drawn for
// this app: its initials on a rounded square in a colour of ours. A vendor's own artwork replaces it by importing one
// file here and adding it to ART under the agent's id; nothing else in the app changes.
import "./agents.css";

const ART: Partial<Record<string, string>> = {};

const LOOK: Record<string, { letters: string; hue: number }> = {
  claude: { letters: "C", hue: 22 },
  codex: { letters: "Cx", hue: 158 },
  opencode: { letters: "Oc", hue: 208 },
  hermes: { letters: "H", hue: 276 },
  kimi: { letters: "K", hue: 336 },
  grok: { letters: "G", hue: 48 },
  "api-key": { letters: "API", hue: 240 },
};

export function AgentIcon({ id, name, size = 28 }: { id: string; name: string; size?: number }) {
  const art = ART[id];
  if (art) return <img className="agent-icon" src={art} width={size} height={size} alt="" />;
  const look = LOOK[id] ?? { letters: name.slice(0, 1).toUpperCase(), hue: 0 };
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
