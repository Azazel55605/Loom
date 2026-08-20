import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";

export const webBaseUrlProvider: BaseUrlProvider = {
  async getBaseUrl(): Promise<string> {
    const configured = import.meta.env.VITE_API_URL?.trim();
    return configured ? configured : "/api";
  },
};
