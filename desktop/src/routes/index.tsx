// Route: where should the operator land first after pairing?
import { createFileRoute, Navigate } from "@tanstack/react-router";
import { useConnectionStore } from "@/stores/connection";

export const Route = createFileRoute("/")({
  component: IndexRedirect,
});

function IndexRedirect() {
  const token = useConnectionStore((s) => s.token);

  if (!token) {
    return <Navigate to="/pair" />;
  }

  return <Navigate to="/dashboard" />;
}
