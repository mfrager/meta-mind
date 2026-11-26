import { cleanup, render } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";

import { catalogEntries } from "@/catalog/registry.tsx";

afterEach(() => {
  cleanup();
});

describe("catalog", () => {
  it("lists every planned component once", () => {
    const ids = catalogEntries.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(
      expect.arrayContaining([
        "button",
        "icon-chip",
        "table-card",
        "card-carousel",
        "command-form",
        "combobox-field",
        "dashboard-overview",
        "settings-block",
      ]),
    );
  });

  for (const entry of catalogEntries) {
    it(`${entry.id} renders every state and passes axe`, async () => {
      expect(entry.states.map((state) => state.id)).toEqual([
        "default",
        "hover",
        "active",
        "focus",
        "disabled",
        "loading",
        "empty",
        "error",
      ]);
      const { container } = render(
        <main>
          <h1>Catalog</h1>
          <h2>{entry.name}</h2>
          {entry.states.map((state) => (
            <div key={state.id} data-specimen={`${entry.id}-${state.id}`}>
              {state.node}
            </div>
          ))}
        </main>,
      );
      for (const state of entry.states) {
        expect(container.querySelector(`[data-specimen="${entry.id}-${state.id}"]`)).toBeTruthy();
      }
      const results = await axe.run(container, {
        rules: {
          "color-contrast": { enabled: false },
          "page-has-heading-one": { enabled: false },
        },
      });
      expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
    });
  }
});
