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
      <output data-palette={palette ? "open" : "closed"} data-pane={pane} />
    </MemoryRouter>
  );
}

beforeEach(() => {
  usePaneStore.setState({ paletteOpen: false, shortcutsOpen: false, sidebarSheet: false, activePane: "document" });
});

it("opens the palette with Ctrl+K and closes it with Escape", () => {
  const { getByRole } = render(<Harness />);
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
  expect(getByRole("status").getAttribute("data-palette")).toBe("open");
  fireEvent.keyDown(window, { key: "Escape" });
  expect(getByRole("status").getAttribute("data-palette")).toBe("closed");
});

it("still opens the palette from a text field, and ignores other shortcuts there", () => {
  usePaneStore.setState({ activePane: "chat" });
  const { getByRole, getByLabelText } = render(<Harness />);
  const field = getByLabelText("Note");
  fireEvent.keyDown(field, { key: "1", ctrlKey: true });
  expect(getByRole("status").getAttribute("data-pane")).toBe("chat");
  fireEvent.keyDown(field, { key: "k", ctrlKey: true });
  expect(getByRole("status").getAttribute("data-palette")).toBe("open");
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
