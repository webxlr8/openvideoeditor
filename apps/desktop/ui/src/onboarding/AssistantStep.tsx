// The welcome's "pick an assistant" screen: the assistants found on this computer, each with its logo and whether
// it is ready, the built-in assistant on an API key, and editing by hand. It never asks for a key. The welcome only
// knows this component's props: the chosen assistant's id, or null for editing by hand.
import { AssistantPicker } from "../agents/AssistantPicker";

export function AssistantStep({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
}) {
  return (
    <div data-testid="assistant-step">
      <AssistantPicker value={value} onChange={onChange} />
    </div>
  );
}
