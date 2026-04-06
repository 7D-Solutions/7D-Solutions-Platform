import { redirect } from "next/navigation";

// Root → dashboard
export default function HomePage() {
  redirect("/dashboard");
}
