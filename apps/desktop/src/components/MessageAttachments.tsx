import type { ChatAttachment } from "../api";

export function MessageAttachments({
  attachments,
  onOpen,
}: {
  attachments?: ChatAttachment[];
  onOpen?: (attachment: ChatAttachment) => void;
}) {
  if (!attachments?.length) return null;
  const images = attachments.filter(
    (attachment) =>
      Boolean(attachment.dataUrl) &&
      attachment.mime.toLowerCase().startsWith("image/"),
  );
  const files = attachments.filter(
    (attachment) =>
      !attachment.dataUrl ||
      !attachment.mime.toLowerCase().startsWith("image/"),
  );
  return (
    <div className="message-attachments">
      {images.length > 0 && (
        <div className="message-image-attachments">
          {images.length > 1 && (
            <div className="message-image-count">{images.length} images</div>
          )}
          <div
            className={`message-image-grid message-image-grid-${Math.min(images.length, 3)}`}
            role="group"
            aria-label={`${images.length} attached image${images.length === 1 ? "" : "s"}`}
          >
            {images.map((attachment, index) => (
              <button
                key={attachment.id}
                type="button"
                className="message-image-button"
                data-testid={`message-image-${attachment.id}`}
                aria-label={`Open pasted image ${index + 1} of ${images.length}`}
                title={attachment.name}
                onClick={() => onOpen?.(attachment)}
              >
                <img
                  className="message-image-preview"
                  src={attachment.dataUrl}
                  alt=""
                />
              </button>
            ))}
          </div>
        </div>
      )}
      {files.length > 0 && (
        <div className="message-file-attachments">
          {files.map((attachment) => (
            <div
              key={attachment.id}
              className="message-attachment"
              data-testid={`message-attachment-${attachment.id}`}
            >
              <span className="message-attachment-name">{attachment.name}</span>
              <span className="message-attachment-meta">
                {attachment.mime}
                {attachment.truncated ? " clipped" : ""}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
