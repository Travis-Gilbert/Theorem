"use client";

import * as React from "react";
import {
  fixtureRustyRedDataPayload,
  normalizeCommonplaceRustyRedViewId,
  type CommonplaceRustyRedDataPayload,
  type CommonplaceRustyRedViewId,
} from "@/lib/commonplace/rustyred-data-contract";

export interface CommonplaceRustyRedDataState {
  payload: CommonplaceRustyRedDataPayload;
  isLoading: boolean;
  error?: string;
}

export function useCommonplaceRustyRedData(view: CommonplaceRustyRedViewId): CommonplaceRustyRedDataState {
  const normalizedView = normalizeCommonplaceRustyRedViewId(view);
  const [payload, setPayload] = React.useState<CommonplaceRustyRedDataPayload>(() =>
    fixtureRustyRedDataPayload(normalizedView),
  );
  const [error, setError] = React.useState<string>();

  React.useEffect(() => {
    const controller = new AbortController();

    fetch(`/api/commonplace/rustyred?view=${encodeURIComponent(normalizedView)}`, {
      signal: controller.signal,
      headers: { Accept: "application/json" },
    })
      .then(async (response) => {
        const data = (await response.json()) as CommonplaceRustyRedDataPayload | { message?: string };
        if (!response.ok) {
          throw new Error("message" in data && data.message ? data.message : "RustyRed data request failed.");
        }
        setError(undefined);
        setPayload(data as CommonplaceRustyRedDataPayload);
      })
      .catch((requestError: unknown) => {
        if (requestError instanceof Error && requestError.name === "AbortError") return;
        const message = requestError instanceof Error ? requestError.message : String(requestError);
        setError(message);
        setPayload(fixtureRustyRedDataPayload(normalizedView, message));
      });

    return () => controller.abort();
  }, [normalizedView]);

  const isLoading = payload.view !== normalizedView && !error;

  return { payload, isLoading, error };
}
