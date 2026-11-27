/**
 * Shared inspector controls (clip-speed-audio design §6; task 0350's Transform section uses them
 * too): a slider with a number field, a number field alone and a check box. Every value comes from
 * the model; a drag shows Rust's in-progress state (ADR-0006), never a local copy.
 */
import { useEffect, useId, useRef, useState } from "react";

import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { t } from "@/i18n";
import { beginGesture, type GestureSender } from "@/ipc/gesture";

import { createSliderGesture } from "./sliderGesture";

const identity = (value: number) => value;

export type SliderFieldProps = {
  label: string;
  /** The model value (the first edited clip's when `mixed`). */
  value: number;
  /** The edited clips have different values: the field shows "Mixed". */
  mixed?: boolean;
  disabled?: boolean;
  /** Slider range and step, in slider positions. */
  min: number;
  max: number;
  step?: number;
  /** Model value ↔ slider position, e.g. a logarithmic scale. Identity by default. */
  toSlider?: (value: number) => number;
  fromSlider?: (position: number) => number;
  /** The number field's text for a model value, and typed text → model value (`null`: invalid). */
  format: (value: number) => string;
  parse: (text: string) => number | null;
  unit: string;
  /** Shown right of the label, e.g. the dB readout of a volume. */
  readout?: string;
  /** Sends one absolute model value with a phase: dragging is a gesture, typing one commit. */
  send: GestureSender<number>;
};

export function SliderField({
  label,
  value,
  mixed = false,
  disabled = false,
  min,
  max,
  step = 1,
  toSlider = identity,
  fromSlider = identity,
  format,
  parse,
  unit,
  readout,
  send,
}: SliderFieldProps) {
  const id = useId();
  const [gesture] = useState(createSliderGesture);
  // A drag still open when the section goes away (selection changed) is cancelled.
  useEffect(() => () => gesture.cancel(), [gesture]);
  const position = Math.min(max, Math.max(min, toSlider(value)));

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-2">
        <label htmlFor={id} className="text-xs text-muted-foreground">
          {label}
        </label>
        {readout && <span className="text-xs text-muted-foreground tabular-nums">{readout}</span>}
      </div>
      <div className="flex items-center gap-3">
        <Slider
          aria-label={label}
          min={min}
          max={max}
          step={step}
          value={[position]}
          disabled={disabled}
          onPointerDown={gesture.pointerDown}
          onKeyDown={gesture.keyDown}
          onPointerUp={gesture.pointerUp}
          onLostPointerCapture={gesture.cancel}
          onValueChange={([next]) => {
            if (next !== undefined) gesture.change(fromSlider(next), send);
          }}
          onValueCommit={([next]) => {
            if (next !== undefined) gesture.commit(fromSlider(next), send);
          }}
        />
        <NumberField
          id={id}
          value={value}
          mixed={mixed}
          disabled={disabled}
          format={format}
          parse={parse}
          unit={unit}
          onCommit={(next) => void beginGesture(send).commit(next)}
        />
      </div>
    </div>
  );
}

export type NumberFieldProps = {
  id?: string;
  /** Accessible name when no `<label htmlFor>` names the field. */
  "aria-label"?: string;
  value: number;
  mixed?: boolean;
  disabled?: boolean;
  format: (value: number) => string;
  parse: (text: string) => number | null;
  unit: string;
  /** A typed value, sent on Enter or when the field loses focus; not sent when it is unchanged. */
  onCommit: (value: number) => void;
};

/**
 * A number typed by hand. Shows the model value until the user types; Enter or leaving the field
 * sends it, Escape restores the model value. Text that is not a number is dropped.
 */
export function NumberField({
  id,
  "aria-label": ariaLabel,
  value,
  mixed = false,
  disabled = false,
  format,
  parse,
  unit,
  onCommit,
}: NumberFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  // Escape blurs the field; its blur must not send the text Escape threw away.
  const discard = useRef(false);

  const commit = () => {
    const text = draft;
    setDraft(null);
    if (discard.current || text === null) {
      discard.current = false;
      return;
    }
    const next = parse(text);
    if (next !== null && (mixed || next !== value)) onCommit(next);
  };

  return (
    <div className="relative w-24 shrink-0">
      <Input
        id={id}
        aria-label={ariaLabel}
        value={draft ?? (mixed ? "" : format(value))}
        placeholder={mixed ? t.inspector.mixed : undefined}
        inputMode="decimal"
        disabled={disabled}
        className="h-7 pr-6 text-right tabular-nums"
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") {
            discard.current = true;
            event.currentTarget.blur();
          }
        }}
      />
      <span
        aria-hidden
        className="pointer-events-none absolute inset-y-0 right-2 flex items-center text-xs text-muted-foreground"
      >
        {unit}
      </span>
    </div>
  );
}

export type CheckFieldProps = {
  label: string;
  checked: boolean;
  /** The edited clips differ: the box shows the indeterminate state. */
  mixed?: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
};

/** An on/off property; one change is one command (a single history entry). */
export function CheckField({ label, checked, mixed = false, disabled = false, onChange }: CheckFieldProps) {
  return (
    <label className="flex items-center gap-2 has-disabled:opacity-50">
      <input
        type="checkbox"
        className="size-3.5 accent-primary focus-visible:outline-2 focus-visible:outline-ring"
        checked={checked && !mixed}
        ref={(element) => {
          if (element) element.indeterminate = mixed;
        }}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}
