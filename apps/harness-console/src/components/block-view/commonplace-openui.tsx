"use client";

import * as React from "react";
import { createLibrary, defineComponent } from "@openuidev/react-lang";
import { z } from "zod/v4";

const SceneArtifactPreview = defineComponent({
  name: "SceneArtifactPreview",
  description: "CommonPlace-skinned preview shell for a SceneOS scene package.",
  props: z.object({
    title: z.string().describe("Human-readable scene title"),
    sceneId: z.string().describe("SceneOS package or scene identifier"),
    summary: z.string().optional().describe("Short explanation of what the scene shows"),
    atoms: z.string().optional().describe("Approximate number of atoms in the scene"),
  }),
  component: ({ props }) => {
    const { title, sceneId, summary, atoms } = props;
    return (
      <section className="cpw-scene-preview" data-scene-id={sceneId}>
        <div className="cpw-scene-canvas" aria-hidden>
          <span className="cpw-scene-node cpw-scene-node-primary" />
          <span className="cpw-scene-node cpw-scene-node-secondary" />
          <span className="cpw-scene-node cpw-scene-node-tertiary" />
          <span className="cpw-scene-edge cpw-scene-edge-one" />
          <span className="cpw-scene-edge cpw-scene-edge-two" />
        </div>
        <div className="cpw-scene-copy">
          <strong>{title}</strong>
          <span>{sceneId}</span>
          {summary ? <p>{summary}</p> : null}
          {atoms ? <small>{atoms} atoms</small> : null}
        </div>
      </section>
    );
  },
});

export const commonplaceOpenUiLibrary = createLibrary({
  components: [SceneArtifactPreview],
  root: "SceneArtifactPreview",
});

export function sceneArtifactOpenUiResponse({
  title,
  sceneId,
  summary,
  atoms,
}: {
  title: string;
  sceneId: string;
  summary?: string;
  atoms?: string;
}): string {
  const args = [
    quoteOpenUi(title),
    quoteOpenUi(sceneId),
    summary ? quoteOpenUi(summary) : "",
    atoms ? quoteOpenUi(atoms) : "",
  ].filter(Boolean);

  return `root = SceneArtifactPreview(${args.join(", ")})`;
}

function quoteOpenUi(value: string): string {
  return `"${value.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"`;
}
