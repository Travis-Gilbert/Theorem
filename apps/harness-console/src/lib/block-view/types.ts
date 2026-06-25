import type * as React from "react";

export type MaybePromise<T> = T | Promise<T>;
export type TypeRef = string;
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export interface Result<T> {
  readonly ok: boolean;
  readonly value?: T;
  readonly error?: string;
}

export type PropType =
  | "string"
  | "text"
  | "number"
  | "integer"
  | "boolean"
  | "json"
  | "id"
  | "timestamp_ms"
  | "vector"
  | "string_list";

export type Constraint =
  | { readonly kind: "required" }
  | { readonly kind: "enum"; readonly values: readonly string[] }
  | { readonly kind: "min"; readonly value: number }
  | { readonly kind: "max"; readonly value: number }
  | { readonly kind: "pattern"; readonly regex: string };

export interface PropertyDef {
  readonly name: string;
  readonly type: PropType;
  readonly constraints?: readonly Constraint[];
}

export type EdgeDirection = "in" | "out";

export interface RelationDef {
  readonly edge: string;
  readonly dir: EdgeDirection;
  readonly target: TypeRef;
}

export interface TypeAxes {
  readonly spatial?: boolean;
  readonly temporal?: boolean;
  readonly embeddable?: boolean;
}

export interface TypeDef {
  readonly name: TypeRef;
  readonly properties: readonly PropertyDef[];
  readonly relations: readonly RelationDef[];
  readonly axes: TypeAxes;
}

export interface TimeRange {
  readonly from_ms?: number;
  readonly to_ms?: number;
}

export interface H3Window {
  readonly cells: readonly string[];
}

export interface ObjectAxes {
  readonly h3?: string;
  readonly valid?: TimeRange;
  readonly embeddable?: boolean;
}

export interface ObjectRef {
  readonly id: string;
  readonly type: TypeRef;
  readonly properties: Readonly<Record<string, JsonValue>>;
  readonly relations?: Readonly<Record<string, readonly string[]>>;
  readonly axes?: ObjectAxes;
}

export type ObjectCardinality = "empty" | "one" | "many";

export interface ShapeRelation {
  readonly edge: string;
  readonly dir: EdgeDirection;
  readonly target?: TypeRef;
}

export interface ObjectShape {
  readonly types: readonly TypeRef[];
  readonly fields: readonly string[];
  readonly relations: readonly ShapeRelation[];
  readonly axes: TypeAxes;
  readonly cardinality: ObjectCardinality;
}

export type Predicate =
  | { readonly kind: "eq"; readonly field: string; readonly value: JsonValue }
  | { readonly kind: "not_eq"; readonly field: string; readonly value: JsonValue }
  | { readonly kind: "contains"; readonly field: string; readonly value: JsonValue }
  | { readonly kind: "exists"; readonly field: string }
  | {
      readonly kind: "relation_exists";
      readonly edge: string;
      readonly dir: EdgeDirection;
      readonly target?: string;
    }
  | { readonly kind: "and"; readonly all: readonly Predicate[] }
  | { readonly kind: "or"; readonly any: readonly Predicate[] }
  | { readonly kind: "not"; readonly predicate: Predicate };

export interface EdgeWalk {
  readonly edge: string;
  readonly dir: EdgeDirection;
  readonly target?: TypeRef;
}

export type Ranker =
  | { readonly kind: "field"; readonly field: string; readonly direction: "asc" | "desc" }
  | { readonly kind: "vector_knn"; readonly field: string; readonly vector: readonly number[]; readonly k: number }
  | { readonly kind: "fulltext"; readonly query: string; readonly fields?: readonly string[] }
  | {
      readonly kind: "graph";
      readonly seeds: readonly string[];
      readonly edge?: string;
      readonly direction?: EdgeDirection;
    };

export type FusionPolicy =
  | { readonly kind: "rrf"; readonly k: number }
  | { readonly kind: "weighted"; readonly weights: Readonly<Record<string, number>> };

export interface ObjectQuerySlice {
  readonly valid?: TimeRange;
  readonly tx?: TimeRange;
  readonly space?: H3Window;
}

export interface ProjectedRelation {
  readonly edge: string;
  readonly dir: EdgeDirection;
  readonly target?: TypeRef;
}

export interface Projection {
  readonly fields?: readonly string[];
  readonly relations?: readonly ProjectedRelation[];
  readonly include_body_preview?: boolean;
  readonly include_metadata?: boolean;
}

export interface PageRequest {
  readonly limit: number;
  readonly cursor?: string;
}

export interface ObjectQuery {
  readonly types: readonly TypeRef[];
  readonly where?: Predicate;
  readonly traverse?: readonly EdgeWalk[];
  readonly rank?: readonly Ranker[];
  readonly fuse?: FusionPolicy;
  readonly slice?: ObjectQuerySlice;
  readonly project?: Projection;
  readonly page?: PageRequest;
  readonly live?: boolean;
}

export type Unsubscribe = () => void;

export interface ObjectSet {
  readonly objects: readonly ObjectRef[];
  readonly shape: ObjectShape;
  readonly next_cursor?: string;
  subscribe(callback: (next: ObjectSet) => void): Unsubscribe;
}

export type AgentTier = "simple" | "difficult" | "max";

export interface ObjectPointer {
  readonly id: string;
  readonly type?: TypeRef;
}

export interface JobSpec {
  readonly name: string;
  readonly args?: Readonly<Record<string, JsonValue>>;
  readonly idempotency_key?: string;
}

export type ObjectAction =
  | { readonly kind: "create"; readonly type: TypeRef; readonly props: Readonly<Record<string, JsonValue>> }
  | { readonly kind: "update"; readonly id: string; readonly patch: Readonly<Record<string, JsonValue>> }
  | { readonly kind: "delete"; readonly id: string }
  | { readonly kind: "link"; readonly from: string; readonly edge: string; readonly to: string; readonly confidence?: number }
  | { readonly kind: "unlink"; readonly from: string; readonly edge: string; readonly to: string }
  | { readonly kind: "run_agent"; readonly target: ObjectPointer | ObjectQuery; readonly tier: AgentTier }
  | { readonly kind: "invoke_tool"; readonly tool: string; readonly args: Readonly<Record<string, JsonValue>> }
  | { readonly kind: "dispatch"; readonly job: JobSpec }
  | { readonly kind: "open"; readonly id: string; readonly view?: string }
  | { readonly kind: "select"; readonly ids: readonly string[] };

export type ActionKind = ObjectAction["kind"];
export type ObjectActionStatus = "accepted" | "applied" | "deferred";

export interface ObjectActionReceipt {
  readonly action_kind: ActionKind;
  readonly status: ObjectActionStatus;
  readonly target_ids?: readonly string[];
  readonly graph_transform?: string;
  readonly actor_id?: string;
  readonly note?: string;
}

export interface ThemeTokens {
  readonly color: Readonly<Record<string, JsonValue>>;
  readonly space: Readonly<Record<string, JsonValue>>;
  readonly typography: Readonly<Record<string, JsonValue>>;
  readonly radius: Readonly<Record<string, JsonValue>>;
  readonly raw?: Readonly<Record<string, JsonValue>>;
}

export type CardinalityRequirement = "any" | "one" | "many";

export interface ShapeRelationMatch {
  readonly edge?: string;
  readonly dir?: EdgeDirection;
  readonly target?: TypeRef;
}

export interface ObjectShapeMatch {
  readonly required_types?: readonly TypeRef[];
  readonly required_fields?: readonly string[];
  readonly required_axes?: TypeAxes;
  readonly cardinality?: CardinalityRequirement;
  readonly requires_relation?: boolean;
  readonly required_edge?: ShapeRelationMatch;
}

export interface BlockHost {
  query(query: ObjectQuery): MaybePromise<ObjectSet>;
  emit(action: ObjectAction): Promise<Result<ObjectActionReceipt>>;
  viewsFor(shape: ObjectShape): readonly ViewDescriptor[];
  readonly tokens: ThemeTokens;
}

export interface ViewRenderProps {
  readonly set: ObjectSet;
  readonly host: BlockHost;
}

export interface ViewDescriptor {
  readonly id: string;
  readonly name: string;
  readonly accepts: ObjectShapeMatch;
  readonly emits: readonly ActionKind[];
  readonly render: React.ComponentType<ViewRenderProps>;
}
