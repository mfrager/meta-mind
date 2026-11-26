import { SettingsBlock } from "@/components/blocks/settings.tsx";

export function SettingsPage() {
  return (
    <div className="pane-scroll h-full overflow-auto px-6 py-6">
      <SettingsBlock />
    </div>
  );
}
