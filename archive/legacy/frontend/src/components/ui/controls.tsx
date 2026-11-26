import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";

import { cn } from "@/lib/utils.ts";
import { recipes } from "@/styles/recipes.ts";
import { focusPreview, type Preview } from "@/components/ui/types.ts";

const control = "h-[var(--control-h)] w-full px-2 text-body";

export function Label({ htmlFor, children }: { htmlFor?: string; children: ReactNode }) {
  return (
    <label htmlFor={htmlFor} data-token="label" className="text-small text-foreground">
      {children}
    </label>
  );
}

export function Field({
  label,
  htmlFor,
  help,
  error,
  required,
  children,
}: {
  label: string;
  htmlFor: string;
  help?: string;
  error?: string;
  required?: boolean;
  children: ReactNode;
}) {
  return (
    <div data-token="field" className="flex flex-col gap-1">
      <Label htmlFor={htmlFor}>
        {label}
        {required ? <span aria-hidden> *</span> : null}
      </Label>
      {children}
      {help && !error ? <p className={recipes.helpText}>{help}</p> : null}
      {error ? (
        <p className={recipes.errorText} role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

export function Input({ preview, className, ...props }: InputHTMLAttributes<HTMLInputElement> & { preview?: Preview }) {
  return (
    <input
      data-token="input"
      className={cn(recipes.control, control, preview === "hover" && "bg-muted", focusPreview(preview), className)}
      {...props}
    />
  );
}

export function Textarea({
  preview,
  className,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { preview?: Preview }) {
  return (
    <textarea
      data-token="textarea"
      className={cn(recipes.control, "min-h-16 w-full px-2 py-1 text-body", preview === "hover" && "bg-muted", focusPreview(preview), className)}
      {...props}
    />
  );
}

export function Select({
  preview,
  className,
  children,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement> & { preview?: Preview }) {
  return (
    <select
      data-token="select"
      className={cn(recipes.control, control, preview === "hover" && "bg-muted", focusPreview(preview), className)}
      {...props}
    >
      {children}
    </select>
  );
}

export function Checkbox({ label, id, ...props }: InputHTMLAttributes<HTMLInputElement> & { label: string; id: string }) {
  return (
    <label htmlFor={id} data-token="checkbox" className="flex items-center gap-2 text-body">
      <input id={id} type="checkbox" className="accent-primary size-3.5" {...props} />
      {label}
    </label>
  );
}

export function Switch({
  label,
  checked,
  onCheckedChange,
  disabled,
  id,
}: {
  label: string;
  checked: boolean;
  onCheckedChange?: (checked: boolean) => void;
  disabled?: boolean;
  id: string;
}) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      data-token="switch"
      className={cn(
        "relative h-5 w-8 rounded-pill border",
        checked ? "border-azure-500 bg-azure-500" : "border-border bg-muted",
        "disabled:opacity-50",
      )}
      onClick={() => onCheckedChange?.(!checked)}
    >
      <span
        className={cn("bg-card absolute top-0.5 size-3.5 rounded-pill", checked ? "left-3.5" : "left-0.5")}
        aria-hidden
      />
    </button>
  );
}

export function RadioGroup({
  name,
  label,
  value,
  options,
  onChange,
}: {
  name: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange?: (value: string) => void;
}) {
  return (
    <fieldset data-token="radio-group" className="flex flex-col gap-1">
      <legend className="text-small">{label}</legend>
      {options.map((option) => (
        <label key={option.value} className="border-border flex items-center gap-2 rounded-md border px-2 py-1 text-body">
          <input
            type="radio"
            name={name}
            value={option.value}
            checked={value === option.value}
            onChange={() => onChange?.(option.value)}
          />
          {option.label}
        </label>
      ))}
    </fieldset>
  );
}

export function Slider({
  id,
  label,
  value,
  min = 0,
  max = 100,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  value: number;
  min?: number;
  max?: number;
  onChange?: (value: number) => void;
  disabled?: boolean;
}) {
  return (
    <input
      id={id}
      aria-label={label}
      data-token="slider"
      type="range"
      min={min}
      max={max}
      value={value}
      disabled={disabled}
      className="accent-primary w-full"
      onChange={(event) => onChange?.(Number(event.target.value))}
    />
  );
}

export function InputGroup({ children, label }: { children: ReactNode; label: string }) {
  return (
    <div data-token="input-group" role="group" aria-label={label} className="border-input flex overflow-hidden rounded-md border">
      {children}
    </div>
  );
}

export function NumberStepper({
  id,
  value,
  onChange,
  unit,
  disabled,
}: {
  id: string;
  value: number;
  onChange?: (value: number) => void;
  unit?: string;
  disabled?: boolean;
}) {
  return (
    <div data-token="number-stepper" className="flex items-center gap-1">
      <button type="button" className="border-border size-7 rounded-md border" aria-label="Decrease" disabled={disabled} onClick={() => onChange?.(value - 1)}>
        −
      </button>
      <input
        id={id}
        aria-label="Number"
        inputMode="numeric"
        className={cn(recipes.control, "h-7 w-16 px-2 text-center font-mono text-value")}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange?.(Number(event.target.value))}
      />
      {unit ? <span className="text-small text-muted-foreground">{unit}</span> : null}
      <button type="button" className="border-border size-7 rounded-md border" aria-label="Increase" disabled={disabled} onClick={() => onChange?.(value + 1)}>
        +
      </button>
    </div>
  );
}

export function OtpInput({ id, value, onChange }: { id: string; value: string; onChange?: (value: string) => void }) {
  const cells = Array.from({ length: 6 }, (_, index) => value[index] ?? "");
  return (
    <div data-token="otp" className="flex gap-1" role="group" aria-label="One-time code">
      {cells.map((cell, index) => (
        <input
          key={index}
          id={index === 0 ? id : undefined}
          aria-label={`Digit ${index + 1}`}
          inputMode="numeric"
          maxLength={1}
          className={cn(recipes.control, "size-8 text-center font-mono")}
          value={cell}
          onChange={(event) => {
            const next = cells.slice();
            next[index] = event.target.value.slice(-1);
            onChange?.(next.join(""));
          }}
        />
      ))}
    </div>
  );
}

export function FileInput({ id, label }: { id: string; label: string }) {
  return (
    <input id={id} aria-label={label} data-token="file-input" type="file" className={cn(recipes.control, "text-small file:mr-2 file:border-0")} />
  );
}

export function DatePicker({ id, label, value }: { id: string; label: string; value?: string }) {
  return (
    <div data-token="date-picker" className="flex gap-2">
      <input id={id} aria-label={label} type="date" defaultValue={value} className={cn(recipes.control, control)} />
      <input aria-label={`${label} end`} type="date" className={cn(recipes.control, control)} />
    </div>
  );
}

export function Combobox({
  id,
  label,
  options,
  value,
  onChange,
}: {
  id: string;
  label: string;
  options: string[];
  value: string;
  onChange?: (value: string) => void;
}) {
  const matches = options.filter((option) => option.toLowerCase().includes(value.toLowerCase()));
  return (
    <div data-token="combobox" className="relative">
      <input
        id={id}
        role="combobox"
        aria-label={label}
        aria-expanded="true"
        aria-controls={`${id}-list`}
        className={cn(recipes.control, control)}
        value={value}
        onChange={(event) => onChange?.(event.target.value)}
      />
      <ul id={`${id}-list`} role="listbox" className="border-border bg-card mt-1 rounded-md border">
        {matches.length === 0 ? (
          <li className="text-small text-muted-foreground px-2 py-1" role="option" aria-disabled="true" aria-selected="false">
            No match
          </li>
        ) : (
          matches.map((option) => (
            <li key={option} role="presentation">
              <button
                type="button"
                role="option"
                aria-selected={option === value}
                className="w-full px-2 py-1 text-left text-body"
                onClick={() => onChange?.(option)}
              >
                {option}
              </button>
            </li>
          ))
        )}
      </ul>
    </div>
  );
}
