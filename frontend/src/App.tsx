import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { TokenGallery } from "@/routes/catalog.tsx";
import { WorkspacePlaceholder } from "@/routes/workspace.tsx";

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<TokenGallery />} />
        <Route path="/dev/catalog" element={<TokenGallery />} />
        <Route path="/workspace" element={<WorkspacePlaceholder />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
