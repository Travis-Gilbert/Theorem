import { useEffect, useMemo } from "react";
import {
  VeltProvider,
  VeltComments,
  VeltPresence,
  useVeltClient,
  useCurrentUser,
} from "@veltdev/react";
import { Editor } from "./Editor";

const API_KEY = import.meta.env.VITE_VELT_API_KEY as string;
const DOC_ID = (import.meta.env.VITE_DOC_ID as string) || "copresence-demo-doc-1";

function hash(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) | 0;
  return h;
}

// Dev identity. For production use VeltProvider authProvider + a backend JWT
// (see velt.dev/docs/get-started/advanced). organizationId gates access; keep it
// stable so every browser tab meets in the same space.
function makeDevUser() {
  const existing = localStorage.getItem("copresence-user-id");
  const id = existing ?? `user-${Math.random().toString(36).slice(2, 8)}`;
  localStorage.setItem("copresence-user-id", id);
  const palette = ["#E5484D", "#0091FF", "#30A46C", "#F76808", "#8E4EC6"];
  const color = palette[Math.abs(hash(id)) % palette.length];
  return {
    userId: id,
    organizationId: "copresence-demo-org",
    name: `Editor ${id.slice(-4)}`,
    email: `${id}@copresence.local`,
    color,
    photoUrl: `https://i.pravatar.cc/120?u=${id}`,
  };
}

function Session() {
  const { client } = useVeltClient();
  const user = useCurrentUser();
  const devUser = useMemo(makeDevUser, []);

  useEffect(() => {
    if (!client) return;
    client.identify(devUser);
    client.setDocument(DOC_ID, { documentName: "Co-presence Editor" });
  }, [client, devUser]);

  return (
    <div className="shell">
      <header className="bar">
        <span className="title">Co-presence Editor</span>
        <div className="presence">
          <VeltPresence />
        </div>
        <span className="who">{user ? `you: ${user.name}` : "connecting…"}</span>
      </header>
      <main className="main">
        <Editor key={user?.userId ?? "anon"} />
      </main>
    </div>
  );
}

export const App = () => {
  if (!API_KEY || API_KEY === "YOUR_VELT_API_KEY") {
    return (
      <div className="setup">
        <h1>Co-presence Editor</h1>
        <p>
          Set <code>VITE_VELT_API_KEY</code> in <code>apps/copresence-editor/.env</code>{" "}
          and safelist <code>localhost:5173</code> under Managed Domains in the{" "}
          <a href="https://console.velt.dev/">Velt console</a>.
        </p>
      </div>
    );
  }
  return (
    <VeltProvider apiKey={API_KEY}>
      <VeltComments />
      <Session />
    </VeltProvider>
  );
};
