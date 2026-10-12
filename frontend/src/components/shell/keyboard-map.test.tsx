import { fireEvent, render } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import { MemoryRouter } from "react-router-dom";

import { KeyboardMap } from "@/components/shell/keyboard-map.tsx";
import { usePaneStore } from "@/lib/store/pane.ts";

function Harness() {
  const palette = usePaneStore((state) => state.paletteOpen);
  const pane = usePaneStore((state) => state.activePane);
  return (
    <MemoryRouter>
      <KeyboardMap />
      <input aria-label="Note" />
      <textarea id="composer" data-composer="true" aria-label="Composer" />
      <output data-palette={palette ? "open" : "closed"} data-pane={pane} />
    </MemoryRouter>
  );
}

beforeEach(() => {
  usePaneStore.setState({ paletteOpen: false, shortcutsOpen: false, sidebarSheet: false, activePane: "document" });
});

it("focuses the composer with Ctrl+K and does not send on Escape", () => {
  const { getByRole, getByLabelText } = render(<Harness />);
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
  expect(document.activeElement).toBe(getByLabelText("Composer"));
  expect(getByRole("status").getAttribute("data-palette")).toBe("closed");
  fireEvent.keyDown(getByLabelText("Composer"), { key: "Escape" });
  expect(getByRole("status").getAttribute("data-palette")).toBe("closed");
});

it("focuses the composer from another text field", () => {
  usePaneStore.setState({ activePane: "chat" });
  const { getByRole, getByLabelText } = render(<Harness />);
  const field = getByLabelText("Note");
  fireEvent.keyDown(field, { key: "1", ctrlKey: true });
  expect(getByRole("status").getAttribute("data-pane")).toBe("chat");
  fireEvent.keyDown(field, { key: "k", ctrlKey: true });
  expect(document.activeElement).toBe(getByLabelText("Composer"));
  expect(getByRole("status").getAttribute("data-palette")).toBe("closed");
});

it("does not treat Escape inside a dialog as a shell dismiss", () => {
  const { getByRole } = render(
    <MemoryRouter>
      <KeyboardMap />
      <dialog open>
        <button type="button">Stay</button>
      </dialog>
      <output data-palette="probe" />
    </MemoryRouter>,
  );
  usePaneStore.setState({ paletteOpen: true });
  fireEvent.keyDown(getByRole("button"), { key: "Escape" });
  expect(usePaneStore.getState().paletteOpen).toBe(true);
});
