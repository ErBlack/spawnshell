import { commands, events } from "./backend.ts";

export async function setupWindow(
  pane: HTMLElement,
  hooks: {
    fit(): void;
    focus(): void;
    onShow(): void;
    onHide(): void;
  },
): Promise<void> {
  const slide = async (toVisible: boolean): Promise<boolean> => {
    const keyframes = toVisible
      ? [{ transform: "translateY(-100%)" }, { transform: "translateY(0)" }]
      : [{ transform: "translateY(0)" }, { transform: "translateY(-100%)" }];
    for (const running of document.body.getAnimations()) running.cancel();
    const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const animation = document.body.animate(keyframes, {
      duration: reduced ? 0 : 180,
      easing: "linear",
      fill: "forwards",
    });
    try {
      await animation.finished;
      animation.commitStyles();
      animation.cancel();
      return true;
    } catch {
      return false;
    }
  };

  let shown = false;

  const reveal = () => {
    if (shown) return;
    shown = true;
    hooks.onShow();
    requestAnimationFrame(() => {
      if (!shown) return;
      slide(true);
      hooks.fit();
      hooks.focus();
    });
  };

  new ResizeObserver(() => hooks.fit()).observe(pane);

  await Promise.all([
    events.windowShown.listen(reveal),
    events.windowHidden.listen(async () => {
      shown = false;
      hooks.onHide();
      if (await slide(false)) commands.hideWindow();
    }),
  ]);

  if (await commands.isOpen()) reveal();
  else await commands.hideWindow();
}
