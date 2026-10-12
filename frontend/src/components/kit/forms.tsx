import { useState } from "react";
import { z } from "zod";
import katex from "katex";

import { Button } from "@/components/ui/actions.tsx";
import { Checkbox, Combobox, DatePicker, Field, Input, RadioGroup, Select, Slider, Switch, Textarea } from "@/components/ui/controls.tsx";
import { Form, useZodForm } from "@/components/ui/form.tsx";
import { Banner } from "@/components/ui/feedback.tsx";
import { Stepper } from "@/components/ui/navigation.tsx";

const commandSchema = z.object({
  graph: z.string().min(1, "Name the graph"),
});

export function FilterForm({ idPrefix = "filter" }: { idPrefix?: string }) {
  const stateId = `${idPrefix}-state`;
  return (
    <form data-token="filter-form" aria-label="Filter rows" className="flex items-end gap-2" onSubmit={(event) => event.preventDefault()}>
      <Field label="State" htmlFor={stateId}>
        <Select id={stateId} defaultValue="ok">
          <option value="ok">ok</option>
          <option value="unavailable">unavailable</option>
        </Select>
      </Field>
      <Button type="submit" size="sm" variant="outline">
        Clear
      </Button>
    </form>
  );
}

export function RecordForm() {
  return (
    <form data-token="record-form" aria-label="Edit record" className="grid grid-cols-2 gap-3" onSubmit={(event) => event.preventDefault()}>
      <Field label="Title" htmlFor="record-title" required>
        <Input id="record-title" defaultValue="Doctor" />
      </Field>
      <Field label="Owner" htmlFor="record-owner">
        <Input id="record-owner" defaultValue="local" />
      </Field>
      <div className="col-span-2 flex justify-end gap-2">
        <Button variant="outline">Cancel</Button>
        <Button type="submit">Save draft</Button>
      </div>
    </form>
  );
}

export function CommandForm() {
  const form = useZodForm(commandSchema, { graph: "" });
  const graph = form.watch("graph");
  return (
    <Form label="graph validate" onSubmit={() => void form.handleSubmit(() => undefined)()}>
      <Field label="Graph" htmlFor="cmd-graph" required error={form.formState.errors.graph?.message}>
        <Input id="cmd-graph" {...form.register("graph")} />
      </Field>
      <pre className="bg-muted rounded-xs p-2 font-mono text-value">{`mm graph validate --graph ${graph || "<name>"}`}</pre>
      <Button type="submit">Run graph validate</Button>
    </Form>
  );
}

export function WizardForm({ step = 0 }: { step?: number }) {
  return (
    <div data-token="wizard-form">
      <Stepper steps={["Change set", "Sandbox", "Promote"]} current={step} />
      <p className="text-small mt-2">Assemble the change set, then test it, then write the reason.</p>
    </div>
  );
}

export function SettingsForm({ dirty = 0, idPrefix = "settings" }: { dirty?: number; idPrefix?: string }) {
  const dataId = `${idPrefix}-data`;
  const reasonId = `${idPrefix}-reason`;
  return (
    <form data-token="settings-form" aria-label={`Kernel settings ${idPrefix}`} className="flex flex-col gap-3" onSubmit={(event) => event.preventDefault()}>
      {dirty > 0 ? (
        <Banner tone="warning">
          {dirty === 1 ? "1 field changed." : `${dirty} fields changed.`} A write needs a reason.
        </Banner>
      ) : null}
      <Field label="Data directory" htmlFor={dataId}>
        <Input id={dataId} placeholder="Path of the data directory" />
      </Field>
      <Field label="Reason" htmlFor={reasonId} required={dirty > 0}>
        <Input id={reasonId} placeholder="Why this write" />
      </Field>
      <Button type="submit">Save settings</Button>
    </form>
  );
}

export function InlineEdit({ value, editing }: { value: string; editing?: boolean }) {
  return editing ? (
    <Input aria-label="Rename" defaultValue={value} data-token="inline-edit" />
  ) : (
    <button type="button" data-token="inline-edit" className="text-body underline-offset-2 hover:underline">
      {value}
    </button>
  );
}

export function TextField({ id, error }: { id: string; error?: string }) {
  return (
    <Field label="Name" htmlFor={id} help="Shown on the document." error={error} required>
      <Input id={id} defaultValue={error ? "" : "doctor"} disabled={Boolean(error)} />
    </Field>
  );
}

export function TextAreaField({ id }: { id: string }) {
  return (
    <Field label="Reason" htmlFor={id}>
      <Textarea id={id} className="font-mono" defaultValue="The store opened." />
    </Field>
  );
}

export function NumberField({ id }: { id: string }) {
  return (
    <Field label="Limit" htmlFor={id}>
      <Input id={id} inputMode="numeric" defaultValue="4" />
    </Field>
  );
}

export function SelectField({ id }: { id: string }) {
  return (
    <Field label="Graph" htmlFor={id}>
      <Select id={id} defaultValue="being">
        <option value="being">being</option>
        <option value="memory">memory</option>
      </Select>
    </Field>
  );
}

export function TagsField() {
  return (
    <div data-token="tags-field">
      <p className="text-small" id="tags-label">
        Tags
      </p>
      <div aria-labelledby="tags-label" className="mt-1 flex gap-1">
        <span className="micro-label rounded-pill border border-border px-2">kernel</span>
        <Input aria-label="Add a tag" className="h-7" />
      </div>
    </div>
  );
}

export function JsonField({ invalid = false, id = "json-field" }: { invalid?: boolean; id?: string }) {
  const [text, setText] = useState(invalid ? "{," : '{ "ok": true }');
  let error = "";
  try {
    JSON.parse(text);
  } catch (err) {
    error = err instanceof Error ? err.message : "Invalid JSON";
  }
  return (
    <Field label="JSON" htmlFor={id} error={error || undefined}>
      <Textarea id={id} className="font-mono" value={text} onChange={(event) => setText(event.target.value)} />
    </Field>
  );
}

export function SemField() {
  return (
    <Field label="SEM" htmlFor="sem-field" help="Indent is preserved.">
      <Textarea id="sem-field" className="font-mono" defaultValue={"being\n  identity"} />
    </Field>
  );
}

export function LatexField() {
  const source = "E = mc^2";
  const html = katex.renderToString(source, { throwOnError: false });
  return (
    <div data-token="latex-field">
      <label htmlFor="latex-field" className="text-small">
        Formula
      </label>
      <Input id="latex-field" defaultValue={source} className="mt-1 font-mono" />
      <p className="micro-label mt-2">Preview</p>
      <div className="mt-1" dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}

export function FileField() {
  return (
    <Field label="Fixture" htmlFor="file-field">
      <Input id="file-field" type="file" />
    </Field>
  );
}

export function ImageField() {
  return (
    <div data-token="image-field" className="border-border rounded-xs border border-dashed p-3">
      <p className="text-small">Drop an image, or choose a file. The chooser validates type and size.</p>
      <Input aria-label="Choose an image" type="file" accept="image/*" className="mt-2" />
    </div>
  );
}

export function ComboboxField({ id }: { id: string }) {
  return (
    <Field label="Command" htmlFor={id} help="Search the command list.">
      <Combobox id={id} label="Command" options={["doctor", "replay"]} value="" />
    </Field>
  );
}

export function CheckboxField({ id }: { id: string }) {
  return <Checkbox id={id} label="Record the ledger" />;
}

export function SwitchField({ id }: { id: string }) {
  return <Switch id={id} label="Compact density" checked={false} />;
}

export function RadioField({ name }: { name: string }) {
  return (
    <RadioGroup
      name={name}
      label="Verdict"
      value="allow"
      options={[
        { value: "allow", label: "Allow" },
        { value: "deny", label: "Deny" },
      ]}
    />
  );
}

export function SliderField({ id }: { id: string }) {
  return <Slider id={id} label="Threshold" value={40} />;
}

export function DateField({ id }: { id: string }) {
  return <DatePicker id={id} label="Range" value="2026-10-11" />;
}

export function ButtonGridField() {
  const [choice, setChoice] = useState("allow");
  return (
    <div role="radiogroup" aria-label="Firewall answer" data-token="button-grid" className="grid grid-cols-2 gap-2">
      {["allow", "deny"].map((option) => (
        <button key={option} type="button" role="radio" aria-checked={choice === option} className="border-border h-8 rounded-md border text-small aria-checked:bg-accent" onClick={() => setChoice(option)}>
          {option}
        </button>
      ))}
    </div>
  );
}
