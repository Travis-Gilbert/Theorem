import type * as React from "react";
import type { LucideIcon } from "lucide-react";

export type SpaceCapability =
  | "view"
  | "editor"
  | "organize"
  | "agent"
  | "account"
  | "plugin";

export interface SpaceTypeDefinition {
  readonly typeKey: string;
  readonly defaultLabel: string;
  readonly icon: LucideIcon;
  readonly href?: string;
  readonly capabilities: readonly SpaceCapability[];
  readonly view: React.ComponentType;
  readonly defaultEditor: React.ComponentType;
}

export interface SpaceTypeInstance {
  readonly id: string;
  readonly typeKey: string;
  readonly label: string;
  readonly order: number;
  readonly enabled: boolean;
  readonly parent?: string;
  readonly config: Record<string, unknown>;
}

export interface SpaceTypeRepository {
  list(): Promise<SpaceTypeInstance[]>;
  save(instance: SpaceTypeInstance): Promise<SpaceTypeInstance>;
  rename(id: string, label: string): Promise<SpaceTypeInstance[]>;
  reorder(activeId: string, overId: string): Promise<SpaceTypeInstance[]>;
  setEnabled(id: string, enabled: boolean): Promise<SpaceTypeInstance[]>;
  create(input: Omit<SpaceTypeInstance, "id" | "order" | "enabled"> & Partial<Pick<SpaceTypeInstance, "id" | "order" | "enabled">>): Promise<SpaceTypeInstance[]>;
}
