import { describe, expect, it } from "vitest";

import {
  addToast,
  MAX_TOASTS,
  removeToast,
  TOAST_DURATION_MS,
  type Toast,
  useToastStore,
} from "./toastStore";

const info = { tone: "info", text: "Saved" } as const;
const error = { tone: "error", text: "Failed", detail: "Disk", action: "openLogs" } as const;

describe("addToast", () => {
  it("appends new messages, newest last", () => {
    const toasts = addToast(addToast([], info, 1), error, 2);
    expect(toasts.map((toast) => [toast.id, toast.text, toast.count])).toEqual([
      [1, "Saved", 1],
      [2, "Failed", 1],
    ]);
  });

  it("moves a repeated message to the end with a new id and a higher count", () => {
    let toasts: Toast[] = [];
    toasts = addToast(toasts, error, 1);
    toasts = addToast(toasts, info, 2);
    toasts = addToast(toasts, error, 3);
    expect(toasts.map((toast) => [toast.id, toast.text, toast.count])).toEqual([
      [2, "Saved", 1],
      [3, "Failed", 2],
    ]);
  });

  it("does not merge the same text with another tone or detail", () => {
    const toasts = addToast(addToast([], info, 1), { ...info, tone: "error" }, 2);
    expect(toasts).toHaveLength(2);
    expect(addToast(toasts, { ...info, detail: "x" }, 3)).toHaveLength(3);
  });

  it(`keeps at most ${MAX_TOASTS}, dropping the oldest`, () => {
    let toasts: Toast[] = [];
    for (let id = 1; id <= MAX_TOASTS + 2; id++)
      toasts = addToast(toasts, { tone: "info", text: `${id}` }, id);
    expect(toasts.map((toast) => toast.id)).toEqual([3, 4, 5, 6]);
  });
});

describe("removeToast", () => {
  it("removes by id only", () => {
    const toasts = addToast(addToast([], info, 1), error, 2);
    expect(removeToast(toasts, 1).map((toast) => toast.id)).toEqual([2]);
    expect(removeToast(toasts, 9)).toHaveLength(2);
  });
});

describe("durations", () => {
  it("keeps errors longer than info and success", () => {
    expect(TOAST_DURATION_MS.error).toBeGreaterThan(TOAST_DURATION_MS.info);
    expect(TOAST_DURATION_MS.error).toBeGreaterThan(TOAST_DURATION_MS.success);
  });
});

describe("useToastStore", () => {
  it("gives every toast a fresh id and dismisses by id", () => {
    const { show } = useToastStore.getState();
    show(info);
    show(error);
    const [first, second] = useToastStore.getState().toasts;
    expect(first?.id).not.toBe(second?.id);
    useToastStore.getState().dismiss(first?.id ?? -1);
    expect(useToastStore.getState().toasts).toEqual([second]);
  });
});
