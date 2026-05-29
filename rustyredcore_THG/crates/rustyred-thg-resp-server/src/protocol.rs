use serde_json::{json, Value};

#[allow(dead_code)]
pub fn resp_command_to_thg(parts: &[String]) -> Option<(String, Value)> {
    let command = parts.first()?.to_ascii_uppercase();
    match command.as_str() {
        "RUSTYRED_THG.RUN.BEGIN" => Some((
            "RUSTYRED_THG.RUN.BEGIN".to_string(),
            json!({
                "run_id": parts.get(1).cloned().unwrap_or_default(),
                "task": parts.get(2).cloned().unwrap_or_default()
            }),
        )),
        "RUSTYRED_THG.RUN.STEP" => Some((
            "RUSTYRED_THG.RUN.STEP".to_string(),
            json!({
                "run_id": parts.get(1).cloned().unwrap_or_default(),
                "step_id": parts.get(2).cloned().unwrap_or_default(),
                "kind": parts.get(3).cloned().unwrap_or_else(|| "observation".to_string())
            }),
        )),
        "RUSTYRED_THG.RUN.GET" => Some((
            "RUSTYRED_THG.RUN.GET".to_string(),
            json!({ "run_id": parts.get(1).cloned().unwrap_or_default() }),
        )),
        "RUSTYRED_THG.STATE.HASH" => Some(("RUSTYRED_THG.STATE.HASH".to_string(), json!({}))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::resp_command_to_thg;

    #[test]
    fn maps_resp_run_begin_to_thg_command() {
        let parts = vec![
            "RUSTYRED_THG.RUN.BEGIN".to_string(),
            "run:1".to_string(),
            "ship".to_string(),
        ];
        let (command, args) = resp_command_to_thg(&parts).unwrap();

        assert_eq!(command, "RUSTYRED_THG.RUN.BEGIN");
        assert_eq!(args["run_id"], "run:1");
        assert_eq!(args["task"], "ship");
    }
}
