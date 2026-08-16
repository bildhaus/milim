export function createChatMessageId(): string {
  return crypto.randomUUID();
}
