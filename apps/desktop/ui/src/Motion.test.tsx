import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { commands } from "./bridge";
import { readAnimation, resolvedProps, templateAsSource, writeAnimation, type MotionTemplate } from "./motion";

async function editor() {
  render(<App />);
  fireEvent.click(screen.getByTestId("home-open"));
  await screen.findByTestId("editor");
  await waitFor(() => expect(screen.getAllByTestId("clip").length).toBe(1));
}

const clips = () => screen.queryAllByTestId("clip");

function select(text: string) {
  const clip = clips().find((c) => c.textContent?.includes(text))!;
  fireEvent.pointerDown(clip, { button: 0, clientX: 300 });
  fireEvent.pointerUp(window);
}

async function detailOf(text: string): Promise<Record<string, unknown>> {
  const view = await commands.projectView();
  if (view.status !== "ok" || !view.data) throw new Error("no project");
  const item = view.data.items.find((i) => i.name.includes(text) || i.text?.includes(text))!;
  const d = await commands.itemDetail(item.id);
  if (d.status !== "ok") throw new Error("no detail");
  return JSON.parse(d.data) as Record<string, unknown>;
}

describe("motion graphics", () => {
  it("inserts a template from the Motion panel and edits its props in the Code tab", async () => {
    await editor();
    fireEvent.click(screen.getByTestId("rail-motion"));
    fireEvent.click(await screen.findByTestId("motion-template-lower-third"));
    await waitFor(() => expect(clips().length).toBe(2));
    select("Lower third");
    expect(await screen.findByTestId("template-props")).toBeTruthy();
    expect(screen.getByTestId("insp-tab-code").getAttribute("aria-selected")).toBe("true");
    const name = screen.getByTestId("prop-name") as HTMLInputElement;
    expect(name.value).toBe("Alex Morgan");
    fireEvent.change(name, { target: { value: "Sam Lee" } });
    fireEvent.blur(name);
    await waitFor(async () => expect((await detailOf("Lower third")).codeComposition).toEqual({ template: "lower-third", props: { name: "Sam Lee" } }));
    expect((await screen.findByTestId("code-status")).textContent).toContain("No problems");
  });

  it("turns a template into code and reports problems with positions", async () => {
    await editor();
    fireEvent.click(screen.getByTestId("rail-motion"));
    fireEvent.click(await screen.findByTestId("motion-template-counter"));
    await waitFor(() => expect(clips().length).toBe(2));
    select("Counter");
    fireEvent.click(await screen.findByTestId("code-edit-source"));
    const src = (await screen.findByTestId("code-source")) as HTMLTextAreaElement;
    fireEvent.change(src, { target: { value: "import x from 'y';\n" + src.value } });
    fireEvent.click(screen.getByTestId("code-check-draft"));
    await waitFor(() => expect(screen.getByTestId("code-status").textContent).toContain("1 error"));
    expect(screen.getAllByTestId("code-diagnostic")[0].textContent).toContain("1:1");
    fireEvent.click(screen.getByTestId("code-apply"));
    await waitFor(async () => expect(String(((await detailOf("Counter")).codeComposition as { source: string }).source)).toContain("import x"));
  });

  it("adds a blank composition", async () => {
    await editor();
    fireEvent.click(screen.getByTestId("rail-motion"));
    fireEvent.click(await screen.findByTestId("motion-blank"));
    await waitFor(() => expect(clips().length).toBe(2));
    select("Composition");
    expect(((await screen.findByTestId("code-source")) as HTMLTextAreaElement).value).toContain("export default function");
  });
});

describe("text animation and fonts", () => {
  it("picks entrance and exit animations for the selected title", async () => {
    await editor();
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("text-preset-headline"));
    await waitFor(() => expect(clips().length).toBe(2));
    select("Big Story");
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("anim-rise"));
    await waitFor(async () => expect((await detailOf("Big Story")).textAnimation).toMatchObject({ preset: "rise", duration: 0.6 }));
    fireEvent.click(screen.getByTestId("anim-phase-out"));
    fireEvent.click(screen.getByTestId("anim-fade"));
    await waitFor(async () => expect((await detailOf("Big Story")).textAnimation).toMatchObject({ preset: "rise", out: { preset: "fade", easing: "ease-in" } }));
    const duration = screen.getByTestId("anim-duration") as HTMLInputElement;
    fireEvent.change(duration, { target: { value: "1.5" } });
    fireEvent.pointerUp(duration);
    await waitFor(async () => expect((await detailOf("Big Story")).textAnimation).toMatchObject({ preset: "rise", out: { preset: "fade", duration: 1.5 } }));
    fireEvent.click(screen.getByTestId("anim-none"));
    await waitFor(async () => expect((await detailOf("Big Story")).textAnimation).toMatchObject({ preset: "rise" }));
    expect(((await detailOf("Big Story")).textAnimation as Record<string, unknown>).out).toBeUndefined();
  });

  it("chooses a font from the picker with previews", async () => {
    await editor();
    fireEvent.click(screen.getByTestId("rail-text"));
    fireEvent.click(await screen.findByTestId("text-preset-headline"));
    await waitFor(() => expect(clips().length).toBe(2));
    select("Big Story");
    fireEvent.click(await screen.findByTestId("insp-font"));
    const option = await screen.findByTestId("font-option-Playfair Display");
    await waitFor(() => expect(option.querySelector("img")).toBeTruthy());
    expect(screen.getByText("Installed on this computer")).toBeTruthy();
    fireEvent.click(option);
    await waitFor(async () => expect((await detailOf("Big Story")).fontFamily).toBe("Playfair Display"));
  });

  it("reads and writes animation values", () => {
    expect(readAnimation({ preset: "pop", out: "fade" })).toEqual({
      in: { preset: "pop", duration: 0.6, delay: 0, easing: "ease-out" },
      out: { preset: "fade", duration: 0.6, delay: 0, easing: "ease-in" },
    });
    expect(writeAnimation({ in: null, out: null })).toBeNull();
    expect(writeAnimation({ in: null, out: { preset: "fade", duration: 1, delay: 0, easing: "linear" } })).toEqual({
      preset: "none",
      out: { preset: "fade", duration: 1, delay: 0, easing: "linear" },
    });
  });

  it("converts template props to source props and assets", () => {
    const t: MotionTemplate = {
      id: "x",
      name: "X",
      description: "",
      duration: 1,
      fullFrame: false,
      source: "export default () => null;",
      props: [
        { name: "title", label: "Title", kind: "text", default: "Hi" },
        { name: "logo", label: "Logo", kind: "image", default: null },
      ],
    };
    expect(resolvedProps(t, { title: "Yo" })).toEqual({ title: "Yo", logo: null });
    expect(templateAsSource(t, { logo: "m1" })).toEqual({ source: t.source, props: { title: "Hi" }, assets: { logo: "m1" } });
  });
});
