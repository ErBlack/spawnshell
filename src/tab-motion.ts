const MOVE = "move";
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

interface Motion {
  duration: number;
  offset: string;
}

export class TabMotion {
  vertical = false;
  private readonly bar: HTMLElement;

  constructor(bar: HTMLElement) {
    this.bar = bar;
  }

  snapshot(): Map<string, DOMRect> {
    const found = new Map<string, DOMRect>();
    for (const element of this.tabs()) {
      const id = element.dataset.tabId;
      if (id) found.set(id, element.getBoundingClientRect());
    }
    return found;
  }

  settle(before: Map<string, DOMRect>) {
    if (reducedMotion.matches || before.size === 0) return;
    let motion: Motion | undefined;
    for (const element of this.tabs()) {
      const id = element.dataset.tabId;
      if (!id || element.classList.contains("tab-dragged")) continue;
      const old = before.get(id);
      if (!old) {
        motion ??= this.motion();
        element.animate(
          [
            { opacity: 0, transform: motion.offset },
            { opacity: 1, transform: "none" },
          ],
          { id: MOVE, duration: motion.duration },
        );
        continue;
      }
      let now = element.getBoundingClientRect();
      if (
        Math.abs(old.left - now.left) < 0.5 &&
        Math.abs(old.top - now.top) < 0.5
      )
        continue;
      this.stop(element);
      now = element.getBoundingClientRect();
      const x = old.left - now.left;
      const y = old.top - now.top;
      motion ??= this.motion();
      element.animate(
        [{ transform: `translate(${x}px, ${y}px)` }, { transform: "none" }],
        { id: MOVE, duration: motion.duration },
      );
    }
  }

  leave(root: HTMLElement) {
    if (reducedMotion.matches || !root.isConnected) return;
    const { duration, offset } = this.motion();
    const rect = root.getBoundingClientRect();
    const bar = this.bar.getBoundingClientRect();
    this.stop(root);
    root.toggleAttribute("data-leaving", true);
    Object.assign(root.style, {
      boxSizing: "border-box",
      position: "absolute",
      left: `${rect.left - bar.left + this.bar.scrollLeft}px`,
      top: `${rect.top - bar.top + this.bar.scrollTop}px`,
      width: `${rect.width}px`,
      pointerEvents: "none",
    });
    this.bar.append(root);
    const remove = () => root.remove();
    root
      .animate(
        [
          { opacity: 1, transform: "none" },
          { opacity: 0, transform: offset },
        ],
        { duration },
      )
      .finished.then(remove, remove);
  }

  stop(element: Element) {
    for (const running of element.getAnimations())
      if (running.id === MOVE) running.cancel();
  }

  private tabs() {
    return this.bar.querySelectorAll<HTMLElement>(".tab:not([data-leaving])");
  }

  private motion(): Motion {
    const value = getComputedStyle(this.bar)
      .getPropertyValue("--duration")
      .trim();
    const duration = value.endsWith("ms")
      ? parseFloat(value)
      : parseFloat(value) * 1000;
    const offset = this.vertical
      ? "translateX(calc(var(--gap-small) * -1))"
      : "translateY(var(--gap-small))";
    return { duration, offset };
  }
}
