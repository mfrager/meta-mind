import { Accordion } from "@/components/ui/layout.tsx";
import { SettingsForm } from "@/components/kit/forms.tsx";

export function SettingsBlock() {
  return (
    <div data-token="settings-block" className="flex max-w-xl flex-col gap-4">
      <h2 className="text-h2">Settings</h2>
      <Accordion
        openId="kernel"
        items={[
          { id: "kernel", title: "Kernel", meta: "1", body: "The console writes through mm-serve. Direct store writes stay closed." },
          { id: "appearance", title: "Appearance", meta: "0", body: "Theme and density live in the sidebar." },
        ]}
      />
      <SettingsForm dirty={1} idPrefix="settings-block" />
    </div>
  );
}
