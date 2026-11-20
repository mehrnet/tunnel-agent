interface TelegramWebApp {
  initData?: string;
  colorScheme?: "light" | "dark";
  ready?: () => void;
  expand?: () => void;
  disableVerticalSwipes?: () => void;
  viewportStableHeight?: number;
  safeAreaInset?: { top: number; bottom: number; left: number; right: number };
  contentSafeAreaInset?: { top: number; bottom: number; left: number; right: number };
  onEvent?: (name: string, callback: () => void) => void;
}
interface TelegramWindow extends Window {
  Telegram?: { WebApp?: TelegramWebApp };
}


export interface TelegramContext {
  source: "telegram" | "browser";
  colorScheme: "light" | "dark";
  initData: string | undefined;
}

function webApp(): TelegramWebApp | undefined {
  if (typeof window === "undefined") return undefined;
  return (window as TelegramWindow).Telegram?.WebApp;
}

export function telegramContext(): TelegramContext {
  const app = webApp();
  if (!app?.initData) return { source: "browser", colorScheme: "light", initData: undefined };
  app.ready?.();
  app.expand?.();
  app.disableVerticalSwipes?.();
  const updateInsets = () => {
    const style = document.documentElement.style;
    if (app.viewportStableHeight) style.setProperty("--app-height", `${app.viewportStableHeight}px`);
    for (const side of ["top", "bottom", "left", "right"] as const) {
      const inset = (app.safeAreaInset?.[side] ?? 0) + (app.contentSafeAreaInset?.[side] ?? 0);
      style.setProperty(`--app-inset-${side}`, `${inset}px`);
    }
  };
  updateInsets();
  for (const name of ["viewportChanged", "safeAreaChanged", "contentSafeAreaChanged"]) app.onEvent?.(name, updateInsets);
  return { source: "telegram", colorScheme: app.colorScheme ?? "light", initData: app.initData };
}
