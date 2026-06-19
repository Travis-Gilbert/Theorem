//! Custom scalars for the MCP GraphQL surface.

use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value as GqlValue};
use serde_json::Value;

/// An arbitrary JSON value, used for free-form payloads (e.g. `createHandoff`
/// payload, `graphAlgorithm` seeds, and the algorithm result block).
#[derive(Clone, Debug, Default)]
pub struct Json(pub Value);

#[Scalar(name = "JSON")]
impl ScalarType for Json {
    fn parse(value: GqlValue) -> InputValueResult<Self> {
        value
            .into_json()
            .map(Json)
            .map_err(|err| InputValueError::custom(err.to_string()))
    }

    fn to_value(&self) -> GqlValue {
        GqlValue::from_json(self.0.clone()).unwrap_or(GqlValue::Null)
    }
}
