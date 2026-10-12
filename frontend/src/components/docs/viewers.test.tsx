import { render } from "@testing-library/react";
import { expect, it } from "vitest";

import { DocBody } from "@/components/docs/viewers.tsx";
import type { FocusDoc } from "@/lib/wire/focus.ts";

const doc: FocusDoc = {
  v: 1,
  id: "graph",
  kind: "tree",
  title: "graph",
  status: "unavailable",
  reason: "the RDF store is opened on demand and no route needs it yet",
  version: 1,
  source: { kind: "nav" },
  capabilities: ["refresh"],
  snapshot: {},
};

it("shows an unavailable reason verbatim", () => {
  const { container } = render(<DocBody doc={doc} editing={false} draft="" onDraft={() => undefined} />);
  expect(container.querySelector("[data-doc-reason]")?.textContent).toBe(doc.reason);
});
