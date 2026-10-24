import type { TabMotion } from "./tab-motion.ts";

const DRAG_THRESHOLD = 4;

interface Placement {
  id: string;
  pinned: boolean;
  position: number;
}

interface Gesture {
  root: HTMLElement;
  anchor: number;
  point: number;
  shift: number;
  parent: HTMLElement;
  next: Element | null;
}

export class TabDrag {
  vertical = false;
  private readonly bar: HTMLElement;
  private readonly pinnedGroup: HTMLElement;
  private readonly group: HTMLElement;
  private readonly motion: TabMotion;
  private readonly onEnd: (placement: Placement | undefined) => void;
  private gesture: Gesture | undefined;
  private release: (() => void) | undefined;
  private dropped: Element | undefined;

  constructor(
    bar: HTMLElement,
    pinnedGroup: HTMLElement,
    group: HTMLElement,
    motion: TabMotion,
    onEnd: (placement: Placement | undefined) => void,
  ) {
    this.bar = bar;
    this.pinnedGroup = pinnedGroup;
    this.group = group;
    this.motion = motion;
    this.onEnd = onEnd;
    bar.addEventListener("pointerdown", (event) => this.grab(event));
    bar.addEventListener(
      "click",
      (event) => {
        if (event.detail === 0 || !(event.target instanceof Element)) return;
        if (event.target.closest(".tab") !== this.dropped) return;
        event.stopPropagation();
        event.preventDefault();
      },
      { capture: true },
    );
  }

  get active(): boolean {
    return this.gesture !== undefined;
  }

  cancel() {
    this.release?.();
  }

  private grab(event: PointerEvent) {
    this.dropped = undefined;
    if (event.button !== 0 || !(event.target instanceof Element)) return;
    const root = event.target.closest<HTMLElement>("button.tab");
    const parent = root?.parentElement;
    if (!root || !parent) return;
    const vertical = this.vertical;
    const along = (pointer: PointerEvent) =>
      vertical ? pointer.clientY : pointer.clientX;
    const start = along(event);

    const detach = () => {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", cancel);
      this.release = undefined;
    };
    const cancel = () => {
      detach();
      this.finish(false);
    };
    const move = (moved: PointerEvent) => {
      const point = along(moved);
      const gesture = this.gesture;
      if (gesture) {
        if (root.hasAttribute("data-leaving")) return cancel();
        gesture.point = point;
        this.reorder();
        return this.follow();
      }
      if (Math.abs(point - start) <= DRAG_THRESHOLD) return;
      this.motion.stop(root);
      const rect = root.getBoundingClientRect();
      const anchor = start - (vertical ? rect.top : rect.left);
      const next = root.nextElementSibling;
      this.gesture = { root, anchor, point, shift: 0, parent, next };
      const before = this.motion.snapshot();
      root.classList.add("tab-dragged");
      this.bar.toggleAttribute("data-dragging", true);
      this.motion.settle(before);
      this.reorder();
      this.follow();
    };
    const up = () => {
      detach();
      this.finish(true);
    };

    this.release?.();
    this.release = cancel;
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", cancel);
  }

  private reorder() {
    const gesture = this.gesture;
    if (!gesture) return;
    const { root, point } = gesture;
    const edge = this.pinnedGroup.getBoundingClientRect();
    const group =
      point < (this.vertical ? edge.bottom : edge.right)
        ? this.pinnedGroup
        : this.group;
    const reference =
      [...group.querySelectorAll(":scope > .tab")].find((tab) => {
        if (tab === root) return false;
        const rect = tab.getBoundingClientRect();
        const middle = this.vertical
          ? rect.top + rect.height / 2
          : rect.left + rect.width / 2;
        return point < middle;
      }) ?? null;
    if (root.parentElement === group && root.nextElementSibling === reference)
      return;
    const before = this.motion.snapshot();
    group.insertBefore(root, reference);
    this.motion.settle(before);
  }

  private follow() {
    const gesture = this.gesture;
    if (!gesture) return;
    const rect = gesture.root.getBoundingClientRect();
    const natural = (this.vertical ? rect.top : rect.left) - gesture.shift;
    gesture.shift = gesture.point - gesture.anchor - natural;
    gesture.root.style.translate = this.vertical
      ? `0 ${gesture.shift}px`
      : `${gesture.shift}px 0`;
  }

  private finish(commit: boolean) {
    const gesture = this.gesture;
    if (!gesture) return;
    this.gesture = undefined;
    const { root, parent, next } = gesture;
    if (commit) this.dropped = root;
    const before = this.motion.snapshot();
    root.classList.remove("tab-dragged");
    root.style.translate = "";
    this.bar.toggleAttribute("data-dragging", false);
    const leaving = root.hasAttribute("data-leaving");
    const group = root.parentElement;
    const id = root.dataset.tabId;
    const moved = group !== parent || root.nextElementSibling !== next;
    if (commit && moved && !leaving && group && id) {
      this.motion.settle(before);
      const tabs = [...group.querySelectorAll(":scope > .tab")];
      const pinned = group === this.pinnedGroup;
      return this.onEnd({ id, pinned, position: tabs.indexOf(root) });
    }
    if (moved && !leaving)
      parent.insertBefore(root, next?.parentElement === parent ? next : null);
    this.motion.settle(before);
    this.onEnd(undefined);
  }
}
