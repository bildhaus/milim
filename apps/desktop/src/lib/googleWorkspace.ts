export type GoogleWorkspaceUrl = {
  fileId: string;
  kind: "sheet" | "document" | "presentation" | "folder" | "drive-file";
};

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
