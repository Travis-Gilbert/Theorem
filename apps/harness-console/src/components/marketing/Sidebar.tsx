import { DocTree } from "@/components/marketing/DocTree";
import { UploadDock } from "@/components/marketing/UploadDock";

/** The marketing landing's persistent left rail — bare on the canvas (no panel):
 *  wordmark, the harness file tree, and the upload end-cap pinned at the bottom.
 *  Sticky, full-viewport height; hidden below lg. */
export function Sidebar() {
  return (
    <aside className="sticky top-0 hidden h-screen w-[300px] shrink-0 flex-col px-5 py-6 lg:flex">
      <div className="mb-3 px-1.5">
        <b className="font-title text-[19px]" style={{ fontWeight: 500 }}>
          Theorem&apos;s Harness
        </b>
      </div>

      <DocTree />

      <div className="flex-1" />

      <UploadDock />
    </aside>
  );
}
