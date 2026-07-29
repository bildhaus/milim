import { type HTMLAttributes, type KeyboardEvent, type ReactNode } from "react";

export function SettingsPanel({ children, ...props }: { children: ReactNode } & HTMLAttributes<HTMLDivElement>) {
  return <div className="settings-panel" {...props}>{children}</div>;
}

export function SettingsBlock({
  title,
  className = "",
  children,
  ...props
}: { title?: string; className?: string; children: ReactNode } & HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={`settings-block${className ? ` ${className}` : ""}`} {...props}>
      {title ? <div className="settings-block-title">{title}</div> : null}
      {children}
    </div>
  );
}

export function settingsChoiceNextIndex(key: string, index: number, length: number): number | null {
  if (length <= 0) return null;
  if (key === "ArrowRight" || key === "ArrowDown") return (index + 1) % length;
  if (key === "ArrowLeft" || key === "ArrowUp") return (index - 1 + length) % length;
  if (key === "Home") return 0;
  if (key === "End") return length - 1;
  return null;
}

export function SettingsChoiceGroup<T extends string>({
  value,
  options,
  onChange,
  testIdPrefix,
  ariaLabel,
}: {
  value: T;
  options: Array<{ value: T; label: string; detail: string }>;
  onChange: (value: T) => void;
  testIdPrefix: string;
  ariaLabel?: string;
}) {
  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const nextIndex = settingsChoiceNextIndex(event.key, index, options.length);
    if (nextIndex == null) return;
    event.preventDefault();
    onChange(options[nextIndex].value);
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>("[role='radio']")
      .item(nextIndex)
      .focus();
  }

  return (
    <div className={`settings-choice-grid${options.length === 3 ? " three-up" : ""}`} role="radiogroup" aria-label={ariaLabel ?? testIdPrefix.replace(/-/g, " ")}>
      {options.map((option, index) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            className={"settings-choice-button" + (selected ? " active" : "")}
            type="button"
            role="radio"
            data-testid={`${testIdPrefix}-${option.value}`}
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option.value)}
            onKeyDown={(event) => onKeyDown(event, index)}
          >
            <span>{option.label}</span>
            <small>{option.detail}</small>
          </button>
        );
      })}
    </div>
  );
}

export function FieldIssue({ message }: { message?: string | null }) {
  return message ? <span className="setting-field-error">{message}</span> : null;
}
