"use client";

import * as React from "react";
import { createSpaceTypeRepository } from "./repository";
import type { SpaceTypeInstance } from "./types";

const repository = createSpaceTypeRepository();

export function useSpaceTypes() {
  const [spaces, setSpaces] = React.useState<SpaceTypeInstance[]>([]);
  const [ready, setReady] = React.useState(false);

  React.useEffect(() => {
    let alive = true;
    repository.list().then((items) => {
      if (!alive) return;
      setSpaces(items);
      setReady(true);
    });
    return () => {
      alive = false;
    };
  }, []);

  const rename = React.useCallback(async (id: string, label: string) => {
    setSpaces(await repository.rename(id, label));
  }, []);

  const reorder = React.useCallback(async (activeId: string, overId: string) => {
    setSpaces(await repository.reorder(activeId, overId));
  }, []);

  const setEnabled = React.useCallback(async (id: string, enabled: boolean) => {
    setSpaces(await repository.setEnabled(id, enabled));
  }, []);

  return { spaces, ready, rename, reorder, setEnabled };
}
