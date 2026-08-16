export function gitRemoteWebUrl(remote: string | null): string | null {
  if (!remote) return null;
  const trimmed = remote.trim().replace(/\.git$/i, "");
  if (/^https?:\/\//i.test(trimmed)) return trimmed;

  if (trimmed.toLowerCase().startsWith("ssh://")) {
    try {
      const url = new URL(trimmed);
      const path = url.pathname.replace(/^\/+/, "");
      return url.hostname && path ? `https://${url.hostname}/${path}` : null;
    } catch {
      return null;
    }
  }

  const separator = trimmed.indexOf(":");
  if (separator <= 0) return null;
  const authority = trimmed.slice(0, separator);
  const path = trimmed.slice(separator + 1);
  const at = authority.lastIndexOf("@");
  const host = authority.slice(at + 1);
  if (!host.includes(".") || !/^[a-z0-9.-]+$/i.test(host)) return null;
  if (!path.includes("/") || /[\\\s]/.test(path)) return null;
  return `https://${host}/${path}`;
}
