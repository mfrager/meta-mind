import { toast, Toaster } from "sonner";

export function ToastHost() {
  return (
    <Toaster
      position="bottom-right"
      toastOptions={{
        className: "rounded-lg border border-border bg-popover text-popover-foreground",
      }}
    />
  );
}

export function showToast(message: string) {
  toast(message);
}
