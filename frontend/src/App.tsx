import { lazy, Suspense } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/blocks/app-shell.tsx";

const Workspace = lazy(() => import("@/routes/workspace.tsx").then((mod) => ({ default: mod.Workspace })));
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
      <BrowserRouter>
        <Routes>
          <Route element={<AppShell />}>
            <Route
              path="/"
              element={
                <Suspense fallback={<RoutePending />}>
                  <Workspace />
                </Suspense>
              }
            />
            <Route
              path="/chat/:sessionId?"
              element={
                <Suspense fallback={<RoutePending />}>
                  <Workspace />
                </Suspense>
              }
            />
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
    </QueryClientProvider>
  );
}
