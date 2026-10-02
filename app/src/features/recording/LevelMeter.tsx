import { FLOOR_DB } from "./levels";

interface LevelMeterProps {
  label: string;
  /** dBFS; `null` when the device sent no audio in the last window. */
  db: number | null;
}

/** One live input level (FR-1.5). A stalled device says so instead of looking silent. */
export function LevelMeter({ label, db }: LevelMeterProps) {
  const percent = db === null ? 0 : Math.round(((db - FLOOR_DB) / -FLOOR_DB) * 100);

  return (
    <div className="flex w-full flex-col gap-1">
      <div className="flex justify-between text-xs text-muted-foreground">
        <span>{label}</span>
        {db === null && <span>No audio from this device</span>}
      </div>
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        className="h-2 w-full overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-100"
          style={{ width: `${String(percent)}%` }}
        />
      </div>
    </div>
  );
}
