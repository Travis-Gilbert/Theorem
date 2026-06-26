"use client";

import type {
  CardinalityRequirement,
  ObjectShape,
  ObjectShapeMatch,
  ShapeRelation,
  ShapeRelationMatch,
  ViewDescriptor,
} from "./types";

function EmptyView() {
  return <div data-block-view="empty" className="min-h-0" />;
}

export const DEFAULT_VIEW_DESCRIPTORS: readonly ViewDescriptor[] = [
  {
    id: "table",
    name: "Table",
    accepts: {
      required_fields: ["title"],
      cardinality: "many",
    },
    emits: ["open", "select"],
    source: {
      package: "@tanstack/react-table",
      component: "Table adapter",
      mode: "wrap",
      regime: "css-vars",
    },
    render: EmptyView,
  },
  {
    id: "board",
    name: "Board",
    accepts: {
      required_fields: ["status"],
      required_axes: { temporal: true },
      cardinality: "many",
    },
    emits: ["update", "open", "select"],
    source: {
      package: "@dnd-kit/core",
      component: "Drag board adapter",
      mode: "wrap",
      regime: "css-vars",
    },
    render: EmptyView,
  },
  {
    id: "card",
    name: "Card",
    accepts: {
      required_fields: ["title"],
      cardinality: "any",
    },
    emits: ["open", "select"],
    source: {
      package: "@/components/ui",
      component: "shadcn card primitives",
      mode: "bespoke",
      regime: "css-vars",
      allowedBespokeReason: "Generic fallback cards are shell primitives over object data.",
    },
    render: EmptyView,
  },
  {
    id: "timeline",
    name: "Timeline",
    accepts: {
      required_axes: { temporal: true },
      cardinality: "many",
    },
    emits: ["open", "select"],
    source: {
      package: "@/components/ui",
      component: "shadcn timeline primitives",
      mode: "bespoke",
      regime: "css-vars",
      allowedBespokeReason: "Timeline fallback encodes temporal object semantics without a single upstream owner.",
    },
    render: EmptyView,
  },
  {
    id: "graph",
    name: "Graph",
    accepts: {
      requires_relation: true,
      cardinality: "many",
    },
    emits: ["link", "unlink", "open"],
    source: {
      package: "@xyflow/react",
      component: "ReactFlow",
      mode: "reskin",
      regime: "css-vars",
    },
    render: EmptyView,
  },
  {
    id: "patch-review",
    name: "PatchReviewPanel",
    accepts: {
      required_types: ["patch"],
      cardinality: "one",
    },
    emits: ["dispatch", "run_agent", "open"],
    source: {
      package: "react-codemirror-merge",
      component: "CodeMirrorMerge",
      mode: "wrap",
      regime: "css-vars",
    },
    render: EmptyView,
  },
  {
    id: "file-tree",
    name: "FileTreePanel",
    accepts: {
      required_types: ["file"],
      required_edge: { edge: "CONTAINS", dir: "out" },
    },
    emits: ["open", "select"],
    source: {
      package: "react-arborist",
      component: "Tree",
      mode: "reskin",
      regime: "css-vars",
    },
    render: EmptyView,
  },
] as const;

export class ViewRegistry {
  #descriptors: ViewDescriptor[];

  constructor(descriptors: readonly ViewDescriptor[] = DEFAULT_VIEW_DESCRIPTORS) {
    assertViewDescriptorSources(descriptors);
    this.#descriptors = [...descriptors];
  }

  register(descriptor: ViewDescriptor): void {
    assertViewDescriptorSources([descriptor]);
    this.#descriptors = this.#descriptors.filter((entry) => entry.id !== descriptor.id);
    this.#descriptors.push(descriptor);
  }

  viewsFor(shape: ObjectShape): ViewDescriptor[] {
    return this.#descriptors.filter((descriptor) => shapeMatches(descriptor.accepts, shape));
  }

  list(): ViewDescriptor[] {
    return [...this.#descriptors];
  }
}

export const defaultViewRegistry = new ViewRegistry();

export function viewsForShape(shape: ObjectShape): ViewDescriptor[] {
  return defaultViewRegistry.viewsFor(shape);
}

export function registerViewDescriptor(descriptor: ViewDescriptor): void {
  defaultViewRegistry.register(descriptor);
}

export function assertViewDescriptorSources(descriptors: readonly ViewDescriptor[]): void {
  for (const descriptor of descriptors) {
    const source = descriptor.source;
    if (!source.package || !source.component) {
      throw new Error(`ViewDescriptor ${descriptor.id} must declare an upstream source package and component.`);
    }

    if (source.mode === "bespoke" && !source.allowedBespokeReason) {
      throw new Error(`Bespoke ViewDescriptor ${descriptor.id} must explain why bespoke rendering is allowed.`);
    }

    if (source.mode !== "bespoke" && source.allowedBespokeReason) {
      throw new Error(`ViewDescriptor ${descriptor.id} has a bespoke reason but mode is ${source.mode}.`);
    }
  }
}

export function shapeMatches(accepts: ObjectShapeMatch, shape: ObjectShape): boolean {
  const types = new Set(shape.types.map(normalizeTypeRef));
  const fields = new Set(shape.fields);
  const requiredTypes = accepts.required_types ?? [];
  const requiredFields = accepts.required_fields ?? [];

  if (requiredTypes.some((typeRef) => !types.has(normalizeTypeRef(typeRef)))) {
    return false;
  }

  if (requiredFields.some((field) => !fields.has(field))) {
    return false;
  }

  if (accepts.required_axes?.spatial && !shape.axes.spatial) {
    return false;
  }

  if (accepts.required_axes?.temporal && !shape.axes.temporal) {
    return false;
  }

  if (accepts.required_axes?.embeddable && !shape.axes.embeddable) {
    return false;
  }

  if (accepts.cardinality && !cardinalityMatches(accepts.cardinality, shape.cardinality)) {
    return false;
  }

  if (accepts.requires_relation && shape.relations.length === 0) {
    return false;
  }

  const requiredEdge = accepts.required_edge;
  if (requiredEdge) {
    return shape.relations.some((relation) => relationMatches(requiredEdge, relation));
  }

  return true;
}

function cardinalityMatches(
  requirement: CardinalityRequirement,
  cardinality: ObjectShape["cardinality"],
): boolean {
  if (requirement === "any") {
    return true;
  }

  return requirement === cardinality;
}

function relationMatches(required: ShapeRelationMatch, relation: ShapeRelation): boolean {
  if (required.edge && required.edge !== relation.edge) {
    return false;
  }

  if (required.dir && required.dir !== relation.dir) {
    return false;
  }

  if (required.target && required.target !== relation.target) {
    return false;
  }

  return true;
}

function normalizeTypeRef(value: string): string {
  return value.trim().toLowerCase().replaceAll("_", "-");
}
