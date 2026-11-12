import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppError } from "./bindings";
import { asAppError, fire, runCommand, setErrorSink } from "./notify";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("notify", () => {
  let off: (() => void) | null = null;
  afterEach(() => {
    off?.();
    vi.restoreAllMocks();
  });

  function sink() {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const seen: AppError[] = [];
    off = setErrorSink((e) => seen.push(e));
    return seen;
  }

  it("shows an error result of a command nobody awaits", async () => {
    const seen = sink();
    const ok = vi.fn();
    runCommand(
      Promise.resolve({
        status: "error" as const,
        error: { code: "io" as const, message: "Disk full" },
      }),
      ok,
    );
    await settle();
    expect(ok).not.toHaveBeenCalled();
    expect(seen).toEqual([{ code: "io", message: "Disk full" }]);
  });

  it("shows a rejected command instead of an unhandled rejection", async () => {
    const seen = sink();
    runCommand(Promise.reject(new Error("IPC closed")));
    fire(Promise.reject(new Error("boom")));
    await settle();
    expect(seen.map((e) => e.message).sort()).toEqual([
      "Something went wrong: IPC closed",
      "Something went wrong: boom",
    ]);
  });

  it("passes data to onOk and reports a throwing callback", async () => {
    const seen = sink();
    const got: number[] = [];
    runCommand(Promise.resolve({ status: "ok" as const, data: 7 }), (n) =>
      got.push(n),
    );
    runCommand(Promise.resolve({ status: "ok" as const, data: 1 }), () => {
      throw new Error("bad JSON");
    });
    await settle();
    expect(got).toEqual([7]);
    expect(seen.map((e) => e.message)).toEqual([
      "Something went wrong: bad JSON",
    ]);
  });

  it("keeps app errors as they are", () => {
    expect(asAppError({ code: "not_found", message: "Gone" })).toEqual({
      code: "not_found",
      message: "Gone",
    });
  });
});
