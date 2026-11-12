/** Copies text; `navigator.clipboard` is missing outside secure contexts (and in tests). */
export function copyText(text: string): Promise<void> {
  const clip = (navigator as Partial<Navigator>).clipboard;
  return clip
    ? clip.writeText(text)
    : Promise.reject(new Error("the clipboard is not available"));
}
