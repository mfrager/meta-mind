import { useState } from "react";

import { Button } from "@/components/ui/button.tsx";
import { IMAGE_DEGRADED } from "@/components/chat/op-registry.tsx";

const ACCEPTED = new Set(["image/png", "image/jpeg", "image/webp", "image/svg+xml"]);

export type ImageAttachment = { name: string; type: string; size: number; caption: string };

export function ImageChooser({
  images,
  onChange,
}: {
  images: ImageAttachment[];
  onChange: (images: ImageAttachment[]) => void;
}) {
  const [error, setError] = useState<string | null>(null);

  const add = (list: FileList | null) => {
    if (!list) return;
    const next = [...images];
    for (const file of list) {
      if (!ACCEPTED.has(file.type)) {
        setError(`${file.type || "that file"} is not an accepted image type`);
        return;
      }
      if (file.size > 8 * 1024 * 1024) {
        setError("an image is larger than 8 MB");
        return;
      }
      if (next.length >= 6) {
        setError("at most 6 images");
        return;
      }
      next.push({ name: file.name, type: file.type, size: file.size, caption: file.name });
    }
    setError(null);
    onChange(next);
  };

  return (
    <div data-image-chooser className="space-y-2 text-sm">
      <label className="flex items-center gap-2">
        <span>Attach images</span>
        <input
          type="file"
          accept="image/png,image/jpeg,image/webp,image/svg+xml"
          multiple
          onChange={(event) => add(event.target.files)}
        />
      </label>
      <p data-image-degraded className="text-xs text-muted-foreground">
        {IMAGE_DEGRADED}
      </p>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
      {images.length ? (
        <ul className="flex flex-wrap gap-2">
          {images.map((image) => (
            <li key={`${image.name}-${image.size}`} className="rounded-md border border-border px-2 py-1 text-xs">
              {image.name}
              <Button
                type="button"
                variant="ghost"
                className="ml-1 h-5 px-1"
                onClick={() => onChange(images.filter((item) => item !== image))}
              >
                Remove
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
