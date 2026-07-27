import { forwardRef, useMemo, type MouseEvent } from "react";
import { highlightedCodeLines } from "../lib/syntaxHighlight";

export const SourceCodeView = forwardRef<HTMLDivElement, {
  source: string;
  language?: string;
  className?: string;
  testId?: string;
  ariaLabel: string;
  selectedLine?: number | null;
  onLineClick?: (lineNumber: number, event: MouseEvent<HTMLButtonElement>) => void;
  onLineDoubleClick?: (lineNumber: number) => void;
}>(function SourceCodeView({
  source,
  language,
  className,
  testId,
  ariaLabel,
  selectedLine,
  onLineClick,
  onLineDoubleClick,
}, ref) {
  const lines = useMemo(() => highlightedCodeLines(source, language), [language, source]);
  return (
    <div ref={ref} className={className} data-testid={testId} role="region" aria-label={ariaLabel} tabIndex={0}>
      {lines.map((tokens, index) => {
        const lineNumber = index + 1;
        const content = (
          <>
            <span className="preview-code-line-number" data-testid="preview-code-line-number" aria-hidden="true">{lineNumber}</span>
            <code className="preview-code-text hljs">
              {tokens.length ? tokens.map((token, tokenIndex) => (
                token.className
                  ? <span className={token.className} key={tokenIndex}>{token.text}</span>
                  : token.text
              )) : " "}
            </code>
          </>
        );
        return onLineClick ? (
          <button
            type="button"
            className={`preview-code-line${selectedLine === lineNumber ? " selected" : ""}`}
            key={lineNumber}
            onClick={(event) => onLineClick(lineNumber, event)}
            onDoubleClick={() => onLineDoubleClick?.(lineNumber)}
          >
            {content}
          </button>
        ) : (
          <div className="preview-code-line" key={lineNumber}>{content}</div>
        );
      })}
    </div>
  );
});
