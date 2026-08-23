import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";

export const webBaseUrl = import.meta.env.VITE_API_URL?.trim() || "/api";

export const webBaseUrlProvider: BaseUrlProvider = {
  async getBaseUrl(): Promise<string> {
    return webBaseUrl;
  },
};
