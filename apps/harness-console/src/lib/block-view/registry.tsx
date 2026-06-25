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
    render: EmptyView,
  },
] as const;

export class ViewRegistry {
  #descriptors: ViewDescriptor[];

  constructor(descriptors: readonly ViewDescriptor[] = DEFAULT_VIEW_DESCRIPTORS) {
    this.#descriptors = [...descriptors];
  }

  register(descriptor: ViewDescriptor): void {
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
