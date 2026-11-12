import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

let broken = true;

function Fragile() {
  if (broken) throw new Error("bad item data");
  return <p>Timeline content</p>;
}

function Window() {
  const [clicks, setClicks] = useState(0);
  return (
    <>
      <ErrorBoundary region="The timeline" className="timeline">
        <Fragile />
      </ErrorBoundary>
      <button onClick={() => setClicks((c) => c + 1)}>Viewer button</button>
      <span data-testid="clicks">{clicks}</span>
    </>
  );
}

describe("ErrorBoundary", () => {
  afterEach(() => {
    broken = true;
    vi.restoreAllMocks();
  });

  it("contains a render error to its region and reloads it", () => {
    // React reports caught errors on the console; keep the test output clean.
    vi.spyOn(console, "error").mockImplementation(() => {});
    render(<Window />);

    const card = screen.getByTestId("region-error");
    expect(card.textContent).toContain("The timeline ran into a problem.");
    expect(card.textContent).toContain("bad item data");
    expect(card.className).toContain("timeline");

    // The rest of the window still works.
    fireEvent.click(screen.getByText("Viewer button"));
    expect(screen.getByTestId("clicks").textContent).toBe("1");

    broken = false;
    fireEvent.click(screen.getByText("Reload this panel"));
    expect(screen.queryByTestId("region-error")).toBeNull();
    expect(screen.getByText("Timeline content")).toBeTruthy();
  });

  it("tries again when its reset keys change", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { rerender } = render(
      <ErrorBoundary region="The inspector" resetKeys={["a"]}>
        <Fragile />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("region-error")).toBeTruthy();
    broken = false;
    rerender(
      <ErrorBoundary region="The inspector" resetKeys={["b"]}>
        <Fragile />
      </ErrorBoundary>,
    );
    expect(screen.getByText("Timeline content")).toBeTruthy();
  });
});
