// Only sources the phone can load by itself. A desktop file path or a loopback
// URL belongs to the computer and would never resolve here.
export function isLoadableImageSource(source: string): boolean {
  if (/^data:image\/(?:png|jpe?g|gif|webp);base64,/i.test(source)) return true;
  const match = /^https?:\/\/(\[[^\]]+\]|[^/:?#]+)/i.exec(source);
  if (!match) return false;
  const host = match[1].toLowerCase();
  return host !== 'localhost' && host !== '127.0.0.1' && host !== '[::1]' && host !== '0.0.0.0';
}
