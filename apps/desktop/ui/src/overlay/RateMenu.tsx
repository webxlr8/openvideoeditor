// The playback rate menu (add-viewer-interaction-and-text-fills task 3.4): 0.5× to 2×, in the transport bar and the
// full-screen controls. Audio plays resampled at rates other than 1×.

import type { TransportCommand } from "../bindings";
import { Dropdown } from "../Menu";
import { PLAYBACK_RATES, rateLabel } from "./rates";

export function RateMenu({
  rate,
  onTransport,
  testId = "rate",
  direction = "up",
}: {
  rate: number;
  onTransport: (c: TransportCommand) => void;
  testId?: string;
  direction?: "down" | "up";
}) {
  return (
    <Dropdown
      testId={testId}
      label={rateLabel(rate)}
      direction={direction}
      align="left"
      items={PLAYBACK_RATES.map((r) => ({
        id: String(r),
        label: r === rate ? `✓ ${rateLabel(r)}` : rateLabel(r),
        onSelect: () => onTransport({ kind: "setRate", rate: r }),
      }))}
    />
  );
}
