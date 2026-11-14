// The welcome's "pick an assistant" choice. The welcome only knows this component's props: the chosen assistant's
// id, or null for editing by hand. The list of local agents (icons, whether each is installed) replaces the two
// options here; nothing outside this file depends on what is inside.

const OPTIONS: { value: string | null; id: string; label: string; note: string }[] =
  [
    {
      value: null,
      id: "none",
      label: "I'll edit by hand",
      note: "No assistant. Nothing leaves this computer.",
    },
    {
      value: "api-key",
      id: "api-key",
      label: "Use an API key",
      note: "Set up later in Settings.",
    },
  ];

export function AssistantStep({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
}) {
  return (
    <div
      className="assistant-options"
      role="radiogroup"
      aria-label="Assistant"
      data-testid="assistant-step"
    >
      {OPTIONS.map((o) => (
        <button
          key={o.id}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          className={`assistant-option${value === o.value ? " on" : ""}`}
          data-testid={`assistant-option-${o.id}`}
          onClick={() => onChange(o.value)}
        >
          <strong>{o.label}</strong>
          <span className="muted small">{o.note}</span>
        </button>
      ))}
    </div>
  );
}
