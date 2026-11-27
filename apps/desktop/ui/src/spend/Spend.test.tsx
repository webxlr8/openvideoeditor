import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { commands } from "../bridge";

afterEach(() => {
  const m = window.__oveMock!;
  m.spendCap = 5;
  m.spend = { spentUsd: 0, answers: {}, refused: [] };
});

describe("spending by agents", () => {
  it("shows an agent's price card; only the person's click answers it", async () => {
    render(<App />);
    act(() =>
      window.__oveMock!.requestSpend({
        quoteId: "q_1",
        estimateUsd: 0.025,
        text: "stand-in agent asks to spend about $0.025 on your fal.ai account.",
      }),
    );
    const card = await screen.findByTestId("spend-card");
    expect(screen.getByTestId("spend-price").textContent).toBe("$0.025");
    expect(screen.getByTestId("spend-text").textContent).toContain("fal.ai");
    expect(card.getAttribute("data-quote")).toBe("q_1");
    // Decline is the default button: Enter does not spend.
    expect(document.activeElement).toBe(screen.getByTestId("spend-decline"));
    fireEvent.click(screen.getByTestId("spend-approve"));
    await waitFor(() => expect(screen.queryByTestId("spend-card")).toBeNull());
    expect(window.__oveMock!.spend.answers).toEqual({ q_1: true });

    act(() => window.__oveMock!.requestSpend({ quoteId: "q_2", estimateUsd: 0.4 }));
    fireEvent.click(await screen.findByTestId("spend-decline"));
    await waitFor(() => expect(screen.queryByTestId("spend-card")).toBeNull());
    expect(window.__oveMock!.spend.answers.q_2).toBe(false);
    expect(window.__oveMock!.spend.spentUsd).toBe(0.025);
  });

  it("changing the limit in Settings takes effect for the next request", async () => {
    render(<App />);
    fireEvent.click(screen.getByTestId("nav-settings"));
    fireEvent.click(await screen.findByTestId("set-generation"));
    const input = await screen.findByTestId<HTMLInputElement>("spend-cap-input");
    await waitFor(() => expect(input.value).toBe("5.00"));
    fireEvent.change(input, { target: { value: "-1" } });
    fireEvent.click(screen.getByTestId("spend-cap-save"));
    expect(screen.getByTestId("spend-cap").textContent).toContain("$0 or more");
    fireEvent.change(input, { target: { value: "$0.30" } });
    fireEvent.click(screen.getByTestId("spend-cap-save"));
    await waitFor(() => expect(screen.getByTestId("spend-cap-save").textContent).toBe("Saved"));
    expect(await commands.spendCap()).toBe(0.3);
    // Under the new limit a $0.40 request is refused without a card; a $0.25 one shows its card.
    act(() => window.__oveMock!.requestSpend({ quoteId: "q_over", estimateUsd: 0.4 }));
    expect(window.__oveMock!.spend.refused).toEqual(["q_over"]);
    expect(screen.queryByTestId("spend-card")).toBeNull();
    act(() => window.__oveMock!.requestSpend({ quoteId: "q_under", estimateUsd: 0.25 }));
    expect(await screen.findByTestId("spend-card")).toBeTruthy();
    // Reopened, Settings shows the saved limit.
    fireEvent.click(screen.getByTestId("spend-decline"));
    fireEvent.click(screen.getByTestId("set-library"));
    fireEvent.click(screen.getByTestId("set-generation"));
    await waitFor(() =>
      expect(screen.getByTestId<HTMLInputElement>("spend-cap-input").value).toBe("0.30"),
    );
  });
});
