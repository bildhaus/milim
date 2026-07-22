export function isLoopbackProviderEndpoint(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    return ["localhost", "127.0.0.1", "::1", "[::1]"].includes(url.hostname);
  } catch {
    return /^(https?:\/\/)?(localhost|127\.0\.0\.1|\[::1\])/i.test(baseUrl.trim());
  }
}
