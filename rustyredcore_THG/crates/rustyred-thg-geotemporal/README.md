# rustyred-thg-geotemporal

Tenant-scoped composition of spatial and temporal filtering over the core's H3 spatial index, plus a plugin that registers the planner-facing `time_series` access method.

## Key API

- `GeoTemporalIndex`: `new()`, `upsert_node(tenant_id, SpatialDesignation, &NodeRecord)`, `execute(GeoTemporalQuery) -> Vec<NodeId>`. Indexes by `(tenant, SpatialDesignation)` and filters candidates by required label and time (point `at_ms` or `interval` overlap).
- `GeoTemporalQuery { tenant_id, designation, spatial, at_ms, interval, required_label, limit }`.
- `SpatialPredicate`: `RadiusKm { lat, lon, radius_km }`, `Bbox { min_lat, min_lon, max_lat, max_lon }`.
- `GeoTemporalAccessPlugin` plus `geotemporal_access_plugin()`. Registers the `time_series` access method (`TimeSeriesAccessMethod`).
- `GeoTemporalError`.

H3 itself lives in `rustyred-thg-core` (`spatial.rs`, via h3o; an S2 backend also exists). This crate composes those primitives with `TimeInterval`; `SpatialDesignation.resolution` is an H3 resolution. Path dep: `rustyred-thg-core`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-geotemporal
```

Tests are inline in `lib.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
