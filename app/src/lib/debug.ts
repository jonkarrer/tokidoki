// What a test script drives the page with (src-tauri/src/debug.rs): development only.
import { invoke } from "@tauri-apps/api/core";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** The first element matching `selector`, or whose text is `selector` when it matches none. */
function find(selector: string): HTMLElement | null {
  try {
    const found = document.querySelector<HTMLElement>(selector);
    if (found) return found;
  } catch {
    // Not a selector: text.
  }
  const clickable = document.querySelectorAll<HTMLElement>("button, label, a");
  return [...clickable].find((element) => element.textContent?.trim().includes(selector)) ?? null;
}

const t = {
  sleep,
  log: (message: string) => invoke("debug_log", { message }),
  /** A picture of the window, once the page has painted. */
  shot: async (name: string) => {
    await sleep(350);
    return invoke<string>("debug_screenshot", { name });
  },
  /** Waits until `check` holds, or fails after `ms`. */
  until: async (what: string, check: () => unknown, ms = 20_000) => {
    const end = Date.now() + ms;
    while (!check()) {
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await sleep(50);
    }
  },
  /** Waits for an element (a selector, or a button's text) and returns it. */
  see: async (selector: string, ms?: number) => {
    await t.until(selector, () => find(selector), ms);
    return find(selector)!;
  },
  click: async (selector: string) => (await t.see(selector)).click(),
  /** Types into a field as a user would: the page hears `input`. */
  type: async (selector: string, text: string) => {
    const field = (await t.see(selector)) as HTMLInputElement | HTMLTextAreaElement;
    field.focus();
    const proto = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  },
  /** Presses Enter in a field. */
  enter: async (selector: string) => {
    const field = await t.see(selector);
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  },
  text: () => document.body.innerText,
  check: async (what: string, ok: unknown) => t.log(`${ok ? "ok  " : "FAIL"} ${what}`),
};

export type Driver = typeof t;

export function startDebug() {
  (window as unknown as { __tokidoki: Driver }).__tokidoki = t;
  invoke("debug_ready");
}
