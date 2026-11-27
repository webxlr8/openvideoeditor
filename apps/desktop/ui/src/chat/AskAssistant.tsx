// "Ask the assistant" in a panel: a menu of the panel's prepared requests. Picking one opens the chat with the
// request and its mentions in the input; nothing is sent until the person sends it.

import type { ItemView_Serialize, ProjectView_Serialize } from "../bindings";
import { Icon } from "../icons";
import { Dropdown } from "../Menu";
import { askAssistant } from "./handoff";
import { type HandoffPanel, PREPARED } from "./prompts";

export function AskAssistant({
  panel,
  project,
  selection,
}: {
  panel: HandoffPanel;
  project: ProjectView_Serialize | null | undefined;
  selection: ItemView_Serialize[];
}) {
  return (
    <div className="ask-assistant">
      <Dropdown
        testId={`ask-${panel}`}
        align="left"
        icon={<Icon name="spark" size={14} />}
        label="Ask the assistant"
        items={PREPARED[panel].map((r) => ({
          id: r.id,
          label: r.label,
          onSelect: () => askAssistant({ prompt: r.prompt, mentions: r.mentions({ project, selection }) }),
        }))}
      />
    </div>
  );
}
