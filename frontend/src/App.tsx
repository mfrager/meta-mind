import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { WorkspacePage } from "@/components/workspace/workspace-page.tsx";

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<WorkspacePage />} />
        <Route path="/chat/:sessionId?" element={<WorkspacePage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
