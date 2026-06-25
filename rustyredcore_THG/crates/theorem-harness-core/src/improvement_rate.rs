use crate::metrics_composite::{session_composite_point, CompositeAxes};
use crate::session_metrics::{load_jsonl_metrics, SessionMetricsState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const EPSILON: f64 = 1e-9;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompositePoint {
    pub label: String,
    pub composite: f64,
    #[serde(default)]
    pub axes: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImprovementRate {
    pub current_score: f64,
    pub trend: f64,
    pub acceleration: Option<f64>,
    pub oscillating: bool,
    pub sign_changes: usize,
    pub window: usize,
    #[serde(default)]
    pub per_axis: BTreeMap<String, AxisRate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisRate {
    pub trend: f64,
    pub acceleration: Option<f64>,
    pub oscillating: bool,
    pub sign_changes: usize,
}

pub fn compute_improvement_rate(
    points: &[CompositePoint],
    window: usize,
) -> Option<ImprovementRate> {
    if window < 2 || points.len() < window {
        return None;
    }

    let recent = &points[points.len() - window..];
    let trend = slope(
        recent
            .iter()
            .map(|point| point.composite)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let acceleration = if points.len() >= window * 2 {
        let older = &points[points.len() - (window * 2)..points.len() - window];
        Some(round6(
            trend
                - slope(
                    older
                        .iter()
                        .map(|point| point.composite)
                        .collect::<Vec<_>>()
                        .as_slice(),
                ),
        ))
    } else {
        None
    };
    let signs = sign_changes(
        recent
            .iter()
            .map(|point| point.composite)
            .collect::<Vec<_>>()
            .as_slice(),
    );

    Some(ImprovementRate {
        current_score: recent.last().map(|point| point.composite).unwrap_or(0.0),
        trend,
        acceleration,
        oscillating: signs > 1,
        sign_changes: signs,
        window,
        per_axis: per_axis_rates(points, window),
    })
}

pub fn composite_points_from_metrics(metrics: &[SessionMetricsState]) -> Vec<CompositePoint> {
    metrics
        .iter()
        .enumerate()
        .map(|(index, metric)| {
            let (composite, axes) = session_composite_point(metric);
            CompositePoint {
                label: if metric.session_id.is_empty() {
                    format!("session:{index}")
                } else {
                    metric.session_id.clone()
                },
                composite,
                axes: axes.as_axis_map(),
            }
        })
        .collect()
}

pub fn load_composite_points_from_jsonl<I, S>(
    lines: I,
) -> Result<Vec<CompositePoint>, serde_json::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let metrics = load_jsonl_metrics(lines)?;
    Ok(composite_points_from_metrics(&metrics))
}

pub fn composite_point(
    label: impl Into<String>,
    composite: f64,
    axes: Option<CompositeAxes>,
) -> CompositePoint {
    CompositePoint {
        label: label.into(),
        composite,
        axes: axes.map(|axes| axes.as_axis_map()).unwrap_or_default(),
    }
}

fn per_axis_rates(points: &[CompositePoint], window: usize) -> BTreeMap<String, AxisRate> {
    let mut axis_names = points
        .iter()
        .flat_map(|point| point.axes.keys().cloned())
        .collect::<Vec<_>>();
    axis_names.sort();
    axis_names.dedup();

    axis_names
        .into_iter()
        .filter_map(|axis| {
            let values = points
                .iter()
                .filter_map(|point| point.axes.get(&axis).copied())
                .collect::<Vec<_>>();
            if values.len() < window {
                return None;
            }
            let recent = &values[values.len() - window..];
            let trend = slope(recent);
            let acceleration = if values.len() >= window * 2 {
                let older = &values[values.len() - (window * 2)..values.len() - window];
                Some(round6(trend - slope(older)))
            } else {
                None
            };
            let changes = sign_changes(recent);
            Some((
                axis,
                AxisRate {
                    trend,
                    acceleration,
                    oscillating: changes > 1,
                    sign_changes: changes,
                },
            ))
        })
        .collect()
}

fn slope(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    round6((values[values.len() - 1] - values[0]) / (values.len() - 1) as f64)
}

fn sign_changes(values: &[f64]) -> usize {
    let mut previous = 0;
    let mut changes = 0;
    for delta in values.windows(2).map(|pair| pair[1] - pair[0]) {
        let current = sign(delta);
        if current == 0 {
            continue;
        }
        if previous != 0 && current != previous {
            changes += 1;
        }
        previous = current;
    }
    changes
}

fn sign(value: f64) -> i8 {
    if value > EPSILON {
        1
    } else if value < -EPSILON {
        -1
    } else {
        0
    }
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotone_series_has_positive_trend_and_no_oscillation() {
        let points = [0.1, 0.2, 0.3, 0.4, 0.5]
            .into_iter()
            .enumerate()
            .map(|(index, value)| composite_point(format!("p{index}"), value, None))
            .collect::<Vec<_>>();

        let rate = compute_improvement_rate(&points, 5).unwrap();

        assert!(rate.trend > 0.0);
        assert!(!rate.oscillating);
        assert_eq!(rate.sign_changes, 0);
    }

    #[test]
    fn flip_flop_series_is_oscillating() {
        let points = [0.1, 0.4, 0.2, 0.5, 0.3]
            .into_iter()
            .enumerate()
            .map(|(index, value)| composite_point(format!("p{index}"), value, None))
            .collect::<Vec<_>>();

        let rate = compute_improvement_rate(&points, 5).unwrap();

        assert!(rate.oscillating);
        assert!(rate.sign_changes > 1);
    }
}
