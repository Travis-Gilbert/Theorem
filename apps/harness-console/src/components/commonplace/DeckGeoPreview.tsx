"use client";

import * as React from "react";
import DeckGL from "@deck.gl/react";
import { ScatterplotLayer } from "@deck.gl/layers";
import type { CommonplaceRustyRedGeoPoint } from "@/lib/commonplace/rustyred-data-contract";

const EMPTY_VIEW_STATE = {
  longitude: -83.6875,
  latitude: 43.0125,
  zoom: 10,
  pitch: 0,
  bearing: 0,
};

export function DeckGeoPreview({ points }: { points: readonly CommonplaceRustyRedGeoPoint[] }) {
  const layers = React.useMemo(
    () => [
      new ScatterplotLayer<CommonplaceRustyRedGeoPoint>({
        id: "commonplace-rustyred-geo-points",
        data: points,
        getPosition: (point) => point.coordinates,
        getRadius: (point) => point.radius,
        getFillColor: [45, 95, 107, 150],
        getLineColor: [184, 98, 61, 210],
        lineWidthMinPixels: 1,
        stroked: true,
        filled: true,
        pickable: true,
      }),
    ],
    [points],
  );

  return (
    <div className="cpw-deck-preview" aria-label="Deck.gl object-coordinate preview">
      <DeckGL controller={false} initialViewState={EMPTY_VIEW_STATE} layers={layers} />
    </div>
  );
}
