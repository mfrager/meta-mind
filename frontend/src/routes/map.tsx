import { useParams } from "react-router-dom";

import { MapView } from "@/components/mindmap/map-view.tsx";

export function MapPage() {
  const { source } = useParams();
  return <MapView source={source ?? "being"} />;
}
