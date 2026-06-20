import { Shell } from "@/components/shell/Shell";

/** Authenticated console surfaces live inside the global shell. */
export default function ConsoleLayout({ children }: { children: React.ReactNode }) {
  return <Shell>{children}</Shell>;
}
