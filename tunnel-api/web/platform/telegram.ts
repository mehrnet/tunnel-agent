interface TelegramWebApp {
  initData?: string;
  colorScheme?: "light" | "dark";
  ready?: () => void;
  expand?: () => void;
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
  return { source: "telegram", colorScheme: app.colorScheme ?? "light", initData: app.initData };
}

export function demoPreviewEnabled(): boolean {
  return typeof window !== "undefined" && new URLSearchParams(window.location.search).get("demo") === "1";
}
