import { mountDemoFrame } from "../demo-frame/mount-demo-frame.ts";
import { mountLogo } from "../logo/mount-logo.ts";

const keys = document.querySelector(".keys") as HTMLButtonElement;
if (!/Mac|iPhone|iPad/.test(navigator.platform)) {
  const control = keys.querySelector('[data-key="Control"]');
  const legend = control?.querySelector(".key__legend");
  if (legend) legend.textContent = "Ctrl";
  control?.querySelector(".key__name")?.remove();
}
mountDemoFrame(keys);
await mountLogo(
  document.querySelector(".logo") as HTMLElement,
  document.querySelector(".ground-glow") as HTMLElement,
);
