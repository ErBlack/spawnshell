import type { TabBar } from "./backend.ts";
import { TabDrag } from "./tab-drag.ts";
import type { TabEntry } from "./tab-manager.ts";
import { TabMotion } from "./tab-motion.ts";

interface Button {
  root: HTMLButtonElement;
  caption: HTMLSpanElement;
  dot: HTMLSpanElement;
}

export interface TabsActions {
  select(id: string): void;
  close(id: string): void;
  menu(id: string | undefined): void;
  place(id: string, pinned: boolean, position: number): void;
  rename(id: string, title: string): void;
  focus(): void;
}

interface Editor {
  id: string;
  input: HTMLInputElement;
  finish(save: boolean): void;
}

function fill(group: HTMLElement, buttons: HTMLElement[]) {
  const same =
    group.children.length === buttons.length &&
    buttons.every((button, index) => group.children[index] === button);
  if (!same) group.replaceChildren(...buttons);
}

export class Tabs {
  private readonly bar: HTMLElement;
  private readonly pinnedGroup: HTMLElement;
  private readonly group: HTMLElement;
  private readonly actions: TabsActions;
  private readonly buttons = new Map<string, Button>();
  private readonly motion: TabMotion;
  private readonly drag: TabDrag;
  private entries: readonly TabEntry[] = [];
  private active: string | undefined;
  private editor: Editor | undefined;

  constructor(
    bar: HTMLElement,
    pinnedGroup: HTMLElement,
    group: HTMLElement,
    actions: TabsActions,
  ) {
    this.bar = bar;
    this.pinnedGroup = pinnedGroup;
    this.group = group;
    this.actions = actions;
    this.motion = new TabMotion(bar);
    this.drag = new TabDrag(bar, pinnedGroup, group, this.motion, (placed) => {
      if (placed) actions.place(placed.id, placed.pinned, placed.position);
      else this.draw();
    });
    bar.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      const tab =
        event.target instanceof Element
          ? event.target.closest<HTMLElement>(".tab")
          : null;
      actions.menu(tab?.dataset.tabId);
    });
  }

  setSide(side: TabBar) {
    if (this.bar.dataset.side === side) return;
    this.drag.cancel();
    this.bar.dataset.side = side;
    const vertical = side !== "bottom";
    this.motion.vertical = vertical;
    this.drag.vertical = vertical;
  }

  render(entries: readonly TabEntry[], active: string | undefined) {
    const before = this.motion.snapshot();
    const switched = active !== this.active;
    this.entries = entries;
    this.active = active;
    for (const [id, button] of this.buttons)
      if (!entries.some((entry) => entry.info.id === id)) {
        this.buttons.delete(id);
        this.motion.leave(button.root);
      }
    const editor = this.editor;
    if (editor && !entries.some((entry) => entry.info.id === editor.id))
      editor.finish(false);
    this.draw(before);
    if (switched && active)
      this.buttons.get(active)?.root.scrollIntoView({
        inline: "nearest",
        block: "nearest",
      });
  }

  rename(id: string) {
    const entry = this.entries.find(({ info }) => info.id === id);
    if (!entry || this.editor?.id === id) return;
    this.editor?.finish(true);
    const initial = entry.label;
    const input = document.createElement("input");
    input.className = "tab tab-rename";
    input.dataset.tabId = id;
    input.value = initial;
    input.spellcheck = false;

    let finished = false;
    const finish = (save: boolean) => {
      if (finished) return;
      finished = true;
      if (this.editor?.input === input) this.editor = undefined;
      this.draw();
      const title = input.value.trim();
      const current = this.entries.find(({ info }) => info.id === id)?.title;
      if (save && title !== initial && (title || null) !== current)
        this.actions.rename(id, title);
      this.actions.focus();
    };
    input.addEventListener("keydown", (event) => {
      if (event.isComposing) return;
      if (event.key === "Enter") finish(true);
      else if (event.key === "Escape") finish(false);
    });
    input.addEventListener("blur", () => finish(true));

    this.editor = { id, input, finish };
    this.draw();
    input.focus();
    input.select();
  }

  private draw(before = this.motion.snapshot()) {
    for (const { info, label } of this.entries) {
      const { root, caption, dot } = this.button(info.id);
      if (info.id === this.active) root.setAttribute("aria-current", "true");
      else root.removeAttribute("aria-current");
      root.toggleAttribute("data-stopped", !info.alive);
      root.title = [info.cwd, info.process].filter(Boolean).join(" · ");
      if (caption.textContent !== label) caption.textContent = label;
      dot.hidden = !info.notified || info.id === this.active;
    }
    if (this.drag.active) return;

    const roots = this.entries.map(({ info }) =>
      this.editor?.id === info.id
        ? this.editor.input
        : this.button(info.id).root,
    );
    const pinned = this.entries.filter(({ info }) => info.pinned).length;
    fill(this.pinnedGroup, roots.slice(0, pinned));
    fill(this.group, roots.slice(pinned));
    this.motion.settle(before);
  }

  private button(id: string): Button {
    const existing = this.buttons.get(id);
    if (existing) return existing;

    const root = document.createElement("button");
    root.className = "tab";
    root.type = "button";
    root.dataset.tabId = id;
    const caption = document.createElement("span");
    caption.className = "tab-title";
    const dot = document.createElement("span");
    dot.className = "tab-notified";
    dot.hidden = true;
    root.append(caption, dot);
    root.addEventListener("mousedown", (event) => event.preventDefault());
    root.addEventListener("click", () => this.actions.select(id));
    root.addEventListener("mouseup", (event) => {
      if (event.button === 1) this.actions.close(id);
    });
    root.addEventListener("dblclick", () => this.rename(id));

    const button = { root, caption, dot };
    this.buttons.set(id, button);
    return button;
  }
}
