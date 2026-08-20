export function normalizeServerUrl(value: string): string {
  const url = new URL(value.trim());
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Use an http:// or https:// server URL.");
  }
  if (url.username !== "" || url.password !== "") {
    throw new Error("The server URL must not contain credentials.");
  }
  if (url.search !== "" || url.hash !== "") {
    throw new Error("The server URL must not contain a query or fragment.");
  }
  return url.toString().replace(/\/+$/, "");
}
