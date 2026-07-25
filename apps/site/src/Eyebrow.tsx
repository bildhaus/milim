export function Eyebrow({ index, label }: { index?: string; label: string }) {
  return (
    <span className="eyebrow">
      {index ? <span className="eyebrow-index">{index}</span> : null}
      <span className="eyebrow-label">{label}</span>
    </span>
  );
}
