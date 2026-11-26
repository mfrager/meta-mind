import { useState } from "react";

import { Button } from "@/components/ui/button.tsx";

export function ChatForm({
  fields,
  onSubmit,
}: {
  fields: { name: string; label: string }[];
  onSubmit: (values: Record<string, string>) => void;
}) {
  const [values, setValues] = useState<Record<string, string>>({});
  return (
    <form
      data-chat-form
      className="space-y-2"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(values);
      }}
    >
      {fields.map((field) => (
        <label key={field.name} className="block space-y-1 text-sm">
          {field.label}
          <input
            className="w-full rounded-md border border-border bg-background px-2 py-1"
            value={values[field.name] ?? ""}
            onChange={(event) => setValues({ ...values, [field.name]: event.target.value })}
          />
        </label>
      ))}
      <Button type="submit">Submit</Button>
    </form>
  );
}

export function SuggestionChips({ suggestions, onPick }: { suggestions: string[]; onPick: (text: string) => void }) {
  if (!suggestions.length) return null;
  return (
    <ul data-suggestions className="flex flex-wrap gap-2">
      {suggestions.map((suggestion) => (
        <li key={suggestion}>
          <Button type="button" variant="outline" onClick={() => onPick(suggestion)}>
            {suggestion}
          </Button>
        </li>
      ))}
    </ul>
  );
}
