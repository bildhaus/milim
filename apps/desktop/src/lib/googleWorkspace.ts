export type GoogleWorkspaceUrl = {
  fileId: string;
  kind: "sheet" | "document" | "presentation" | "folder" | "drive-file";
};

export type GoogleRevocationStatus = "confirmed" | "unconfirmed" | "not_needed";

export const GOOGLE_ACCOUNT_CONNECTIONS_URL = "https://myaccount.google.com/connections";
export const GOOGLE_CONNECT_DISCLOSURE =
  "Google files stay local until you use them in a chat or external tool. If you use a remote model, selected file content may be sent to that provider to fulfill your request. That provider’s terms apply.";
export const GOOGLE_REMOVE_MESSAGE =
  "Removed from Milim’s selected-file list. The Drive file and Google authorization were not changed.";

export function googleDisconnectMessage(revocation: GoogleRevocationStatus): string {
  if (revocation === "confirmed") {
    return "Google Workspace disconnected. Google confirmed revocation and local authorization was removed.";
  }
  if (revocation === "unconfirmed") {
    return "Disconnected locally, but Google did not confirm revocation.";
  }
  return "Google Workspace local authorization was removed.";
}

const GOOGLE_ID = /^[A-Za-z0-9_-]{1,256}$/;

export function googleWorkspaceUrl(value: string | null | undefined): GoogleWorkspaceUrl | null {
  if (!value) return null;
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  const host = url.hostname.toLowerCase();
  const segments = url.pathname.split("/").filter(Boolean);
  const candidate = (kind: GoogleWorkspaceUrl["kind"], id: string | undefined) =>
    id && GOOGLE_ID.test(id) ? { fileId: id, kind } : null;

  if (host === "docs.google.com") {
    const idIndex = segments.indexOf("d");
    const id = idIndex >= 0 ? segments[idIndex + 1] : undefined;
    if (segments[0] === "spreadsheets") return candidate("sheet", id);
    if (segments[0] === "document") return candidate("document", id);
    if (segments[0] === "presentation") return candidate("presentation", id);
    return null;
  }
  if (host === "drive.google.com") {
    if (segments[0] === "drive" && segments[1] === "folders") {
      return candidate("folder", segments[2]);
    }
    if (segments[0] === "file" && segments[1] === "d") {
      return candidate("drive-file", segments[2]);
    }
    if (segments[0] === "open") {
      return candidate("drive-file", url.searchParams.get("id") ?? undefined);
    }
  }
  return null;
}

export function googleWorkspaceFileUrl(file: {
  id: string;
  mime_type: string;
  web_view_link?: string | null;
}): string {
  if (file.web_view_link) return file.web_view_link;
  switch (file.mime_type) {
    case "application/vnd.google-apps.spreadsheet":
      return `https://docs.google.com/spreadsheets/d/${file.id}/edit`;
    case "application/vnd.google-apps.document":
      return `https://docs.google.com/document/d/${file.id}/edit`;
    case "application/vnd.google-apps.presentation":
      return `https://docs.google.com/presentation/d/${file.id}/edit`;
    case "application/vnd.google-apps.folder":
      return `https://drive.google.com/drive/folders/${file.id}`;
    default:
      return `https://drive.google.com/file/d/${file.id}/view`;
  }
}

export function googleSheetCellRange(sheetTitle: string, address: string): string {
  const escapedTitle = sheetTitle.split("'").join("''");
  return sheetTitle ? `'${escapedTitle}'!${address}` : address;
}

export function parseGoogleSheetClipboard(text: string): string[][] {
  const rows = text.replace(/\r\n?/g, "\n").split("\n");
  if (rows[rows.length - 1] === "") rows.pop();
  return rows.map((row) => row.split("\t"));
}

export type GoogleDocEditableParagraph = {
  start: number;
  end: number;
  text: string;
};

export function googleDocEditableParagraph(value: unknown): GoogleDocEditableParagraph | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const item = value as Record<string, unknown>;
  const paragraph = item.paragraph;
  if (!paragraph || typeof paragraph !== "object" || Array.isArray(paragraph)) return null;
  const elements = (paragraph as Record<string, unknown>).elements;
  if (!Array.isArray(elements)) return null;
  const runs = elements.map((element) => {
    if (!element || typeof element !== "object" || Array.isArray(element)) return null;
    const textRun = (element as Record<string, unknown>).textRun;
    if (!textRun || typeof textRun !== "object" || Array.isArray(textRun)) return null;
    const content = (textRun as Record<string, unknown>).content;
    return typeof content === "string" ? content : null;
  });
  if (runs.some((run) => run === null)) return null;
  const start = item.startIndex;
  const end = item.endIndex;
  if (typeof start !== "number" || typeof end !== "number" || start < 1 || end <= start) return null;
  const text = (runs as string[]).join("").replace(/\n$/, "");
  return { start, end: Math.max(start, end - 1), text };
}
