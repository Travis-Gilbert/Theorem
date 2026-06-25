use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use rustyred_thg_core::{
    run_epistemic_cron_pass, ConnectionFeatures, ConnectionScore, ConnectionScorer,
    EpistemicCronInput, EpistemicCronReport, EpistemicEnrichmentError, GraphStore,
    GraphStoreResult, LearnedConnectionScorerPair, LearnedConnectionScorerRequest,
    LearnedConnectionScorerResponse, NliClassifier, NliEpistemicEnricher, NliPairSelectionConfig,
    DEFAULT_CONNECTION_CALIBRATION_VERSION, DEFAULT_CONNECTION_FEATURE_VERSION,
    DEFAULT_CONNECTION_SCORER_MODEL_ID, EPISTEMIC_SCORER_CALIBRATION_ENV,
    EPISTEMIC_SCORER_ENDPOINT_ENV, EPISTEMIC_SCORER_MODEL_ENV,
};

pub const EPISTEMIC_SCORER_TOKEN_ENV: &str = "THEOREM_EPISTEMIC_SCORER_TOKEN";
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunPodConnectionScorerConfig {
    pub endpoint: Option<String>,
    pub model_id: String,
    pub calibration_version: String,
    pub feature_version: String,
    pub bearer_token: Option<String>,
    pub timeout_seconds: u64,
}

impl Default for RunPodConnectionScorerConfig {
    fn default() -> Self {
        Self {
            endpoint: std::env::var(EPISTEMIC_SCORER_ENDPOINT_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty()),
            model_id: std::env::var(EPISTEMIC_SCORER_MODEL_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_CONNECTION_SCORER_MODEL_ID.to_string()),
            calibration_version: std::env::var(EPISTEMIC_SCORER_CALIBRATION_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_CONNECTION_CALIBRATION_VERSION.to_string()),
            feature_version: DEFAULT_CONNECTION_FEATURE_VERSION.to_string(),
            bearer_token: std::env::var(EPISTEMIC_SCORER_TOKEN_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty()),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunPodConnectionScorer {
    config: RunPodConnectionScorerConfig,
    client: Client,
}

impl RunPodConnectionScorer {
    pub fn new(config: RunPodConnectionScorerConfig) -> Result<Self, EpistemicEnrichmentError> {
        let timeout_seconds = config.timeout_seconds.max(1);
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_seconds))
            .build()
            .map_err(|error| {
                EpistemicEnrichmentError::new(
                    "learned_scorer_client_failed",
                    format!("failed to create learned scorer HTTP client: {error}"),
                )
            })?;
        Ok(Self { config, client })
    }

    pub fn request_for_features(
        &self,
        features: &[ConnectionFeatures],
    ) -> LearnedConnectionScorerRequest {
        LearnedConnectionScorerRequest {
            model_id: self.config.model_id.clone(),
            calibration_version: self.config.calibration_version.clone(),
            feature_version: self.config.feature_version.clone(),
            pairs: features
                .iter()
                .map(|feature| LearnedConnectionScorerPair {
                    left_content_id: feature.from_content_id.clone(),
                    right_content_id: feature.to_content_id.clone(),
                    premise: feature.premise.clone(),
                    hypothesis: feature.hypothesis.clone(),
                    features: feature.clone(),
                })
                .collect(),
        }
    }

    pub fn parse_scores(value: Value) -> Result<Vec<ConnectionScore>, EpistemicEnrichmentError> {
        if let Ok(response) =
            serde_json::from_value::<LearnedConnectionScorerResponse>(value.clone())
        {
            return Ok(response.scores);
        }
        if let Some(output) = value.get("output") {
            if let Ok(response) =
                serde_json::from_value::<LearnedConnectionScorerResponse>(output.clone())
            {
                return Ok(response.scores);
            }
        }
        Err(EpistemicEnrichmentError::new(
            "learned_scorer_response_invalid",
            "learned scorer response must be {scores:[...]} or RunPod {output:{scores:[...]}}",
        ))
    }
}

impl ConnectionScorer for RunPodConnectionScorer {
    fn score(
        &self,
        features: &ConnectionFeatures,
    ) -> Result<ConnectionScore, EpistemicEnrichmentError> {
        self.score_batch(std::slice::from_ref(features))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                EpistemicEnrichmentError::new(
                    "learned_scorer_empty_response",
                    "learned scorer returned no score",
                )
            })
    }

    fn score_batch(
        &self,
        features: &[ConnectionFeatures],
    ) -> Result<Vec<ConnectionScore>, EpistemicEnrichmentError> {
        if features.is_empty() {
            return Ok(Vec::new());
        }
        let endpoint = self.config.endpoint.as_deref().ok_or_else(|| {
            EpistemicEnrichmentError::new(
                "learned_scorer_unavailable",
                format!(
                    "learned connection scorer is the default; configure {EPISTEMIC_SCORER_ENDPOINT_ENV}"
                ),
            )
        })?;
        let mut request = self
            .client
            .post(endpoint)
            .json(&self.request_for_features(features));
        if let Some(token) = self.config.bearer_token.as_deref() {
            request = request.bearer_auth(token);
        }
        let value = request
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|error| {
                EpistemicEnrichmentError::new(
                    "learned_scorer_request_failed",
                    format!("learned scorer request failed: {error}"),
                )
            })?
            .json::<Value>()
            .map_err(|error| {
                EpistemicEnrichmentError::new(
                    "learned_scorer_response_invalid",
                    format!("learned scorer response was not JSON: {error}"),
                )
            })?;
        let scores = Self::parse_scores(value)?;
        if scores.len() != features.len() {
            return Err(EpistemicEnrichmentError::new(
                "learned_scorer_count_mismatch",
                format!(
                    "learned scorer returned {} scores for {} feature rows",
                    scores.len(),
                    features.len()
                ),
            ));
        }
        Ok(scores)
    }
}

pub fn run_epistemic_cron_pass_with_runpod_scorer<S, C>(
    store: &mut S,
    input: EpistemicCronInput,
    classifier: C,
    scorer_config: RunPodConnectionScorerConfig,
) -> GraphStoreResult<EpistemicCronReport>
where
    S: GraphStore,
    C: NliClassifier,
{
    let scorer = RunPodConnectionScorer::new(scorer_config).map_err(|error| {
        rustyred_thg_core::GraphStoreError::new(
            error.code,
            format!(
                "failed to initialize learned connection scorer: {}",
                error.message
            ),
        )
    })?;
    let enricher = NliEpistemicEnricher::new(classifier, scorer, NliPairSelectionConfig::default());
    run_epistemic_cron_pass(store, input, &enricher)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use rustyred_thg_core::{
        ConnectionFeatures, EpistemicRelationKind, DEFAULT_CONNECTION_CALIBRATION_VERSION,
        DEFAULT_CONNECTION_FEATURE_VERSION,
    };

    use super::{RunPodConnectionScorer, RunPodConnectionScorerConfig};

    #[test]
    fn runpod_request_carries_text_nli_graph_features_and_provenance() {
        let scorer = RunPodConnectionScorer::new(RunPodConnectionScorerConfig {
            endpoint: Some("https://example.test/run".to_string()),
            ..RunPodConnectionScorerConfig::default()
        })
        .unwrap();
        let request = scorer.request_for_features(&[ConnectionFeatures {
            from_content_id: "claim:a".to_string(),
            to_content_id: "claim:b".to_string(),
            premise: "cache is safe".to_string(),
            hypothesis: "cache races corrupt data".to_string(),
            candidate_evidence: vec!["edge:CITES:a:b".to_string()],
            provenance: vec!["nli_model:fixture".to_string()],
            nli_entailment_score: 0.03,
            nli_neutral_score: 0.04,
            nli_contradiction_score: 0.93,
            support_in_degree: 2,
            attack_in_degree: 1,
            bridge_score: 0.5,
            source_reliability_mean: Some(0.8),
            graph_edge_count: 4,
            feature_version: DEFAULT_CONNECTION_FEATURE_VERSION.to_string(),
        }]);

        assert_eq!(request.pairs.len(), 1);
        assert_eq!(request.pairs[0].premise, "cache is safe");
        assert_eq!(request.pairs[0].hypothesis, "cache races corrupt data");
        assert_eq!(request.pairs[0].features.nli_contradiction_score, 0.93);
        assert_eq!(request.pairs[0].features.support_in_degree, 2);
        assert_eq!(
            request.pairs[0].features.candidate_evidence,
            ["edge:CITES:a:b".to_string()]
        );
        assert_eq!(
            request.pairs[0].features.provenance,
            ["nli_model:fixture".to_string()]
        );
    }

    #[test]
    fn runpod_response_accepts_raw_and_output_wrapped_scores() {
        let raw = json!({
            "scores": [{
                "kind": "undercuts",
                "score": 0.88,
                "confidence": 0.84,
                "model_id": "runpod-connection-v1",
                "calibration_version": DEFAULT_CONNECTION_CALIBRATION_VERSION,
                "feature_version": DEFAULT_CONNECTION_FEATURE_VERSION,
                "evidence": "learned"
            }]
        });
        let wrapped = json!({ "output": raw.clone() });

        for value in [raw, wrapped] {
            let scores = RunPodConnectionScorer::parse_scores(value).unwrap();
            assert_eq!(scores.len(), 1);
            assert_eq!(scores[0].kind, Some(EpistemicRelationKind::Undercuts));
            assert_eq!(scores[0].model_id, "runpod-connection-v1");
        }
    }
}
