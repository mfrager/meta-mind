import { lazy, Suspense } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { TooltipProvider } from "@/agentgui/components/ui/tooltip";

const AppShell = lazy(() =>
  import("@/agentgui/components/shell/app-shell").then((mod) => ({ default: mod.AppShell })),
);
const MapPage = lazy(() => import("@/routes/map.tsx").then((mod) => ({ default: mod.MapPage })));
const SettingsPage = lazy(() => import("@/routes/settings.tsx").then((mod) => ({ default: mod.SettingsPage })));
const TokenGallery = lazy(() => import("@/routes/catalog.tsx").then((mod) => ({ default: mod.TokenGallery })));

function RoutePending() {
  return (
    <div className="text-muted-foreground p-4 text-small" data-route-pending>
      Loading…
    </div>
  );
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: true,
    },
  },
});

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={180}>
        <BrowserRouter>
          <Routes>
            <Route
              element={
                <Suspense fallback={<div className="bg-background h-dvh" data-shell-pending />}>
                  <AppShell />
                </Suspense>
              }
            >
              <Route path="/" element={null} />
              <Route path="/chat/:sessionId?" element={null} />
              <Route
                path="/dev/catalog"
                element={
                  <Suspense fallback={<RoutePending />}>
                    <TokenGallery />
                  </Suspense>
                }
              />
              <Route
                path="/settings"
                element={
                  <Suspense fallback={<RoutePending />}>
                    <SettingsPage />
                  </Suspense>
                }
              />
              <Route
                path="/map/:source"
                element={
                  <Suspense fallback={<RoutePending />}>
                    <MapPage />
                  </Suspense>
                }
              />
              <Route path="*" element={<Navigate to="/" replace />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </TooltipProvider>
    </QueryClientProvider>
  );
}
