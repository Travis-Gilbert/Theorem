use serde_json::{json, Value};

use rustyred_thg_core::{OrderedIndexRegistry, OrderedMode};

#[derive(Clone, Debug, PartialEq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Vec<RespValue>),
}

impl RespValue {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::SimpleString(value) => format!("+{value}\r\n").into_bytes(),
            Self::Error(value) => format!("-ERR {value}\r\n").into_bytes(),
            Self::Integer(value) => format!(":{value}\r\n").into_bytes(),
            Self::Bulk(Some(value)) => {
                let mut out = format!("${}\r\n", value.len()).into_bytes();
                out.extend_from_slice(value);
                out.extend_from_slice(b"\r\n");
                out
            }
            Self::Bulk(None) => b"$-1\r\n".to_vec(),
            Self::Array(values) => {
                let mut out = format!("*{}\r\n", values.len()).into_bytes();
                for value in values {
                    out.extend_from_slice(&value.encode());
                }
                out
            }
        }
    }
}

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

pub fn execute_resp_command(indexes: &mut OrderedIndexRegistry, parts: &[String]) -> RespValue {
    let Some(command) = parts.first().map(|value| value.to_ascii_uppercase()) else {
        return RespValue::Error("empty command".to_string());
    };
    match command.as_str() {
        "PING" => RespValue::SimpleString("PONG".to_string()),
        "ZADD" => zadd(indexes, parts),
        "ZSCORE" => zscore(indexes, parts),
        "ZPOPMIN" => zpop(indexes, parts, PopSide::Min),
        "ZPOPMAX" => zpop(indexes, parts, PopSide::Max),
        "ZRANGEBYSCORE" => zrange_by_score(indexes, parts),
        "ZREM" => zrem(indexes, parts),
        "ZCARD" => zcard(indexes, parts),
        "ZRANK" => zrank(indexes, parts),
        _ if resp_command_to_thg(parts).is_some() => {
            RespValue::Error("RUSTYRED_THG command transport is not wired in this RESP slice".to_string())
        }
        _ => RespValue::Error(format!(
            "unknown command '{command}'; this RustyRed RESP surface implements scoped ZSET commands only"
        )),
    }
}

#[derive(Clone, Copy)]
enum PopSide {
    Min,
    Max,
}

fn zadd(indexes: &mut OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() < 4 || parts.len() % 2 != 0 {
        return RespValue::Error("ZADD requires key score member [score member ...]".to_string());
    }
    let key = &parts[1];
    let mut added = 0_i64;
    let index = indexes.index_mut(key, OrderedMode::Transient);
    for pair in parts[2..].chunks(2) {
        let score = match parse_score(&pair[0]) {
            Ok(score) => score,
            Err(error) => return RespValue::Error(error),
        };
        match index.zadd(pair[1].as_bytes().to_vec(), score) {
            Ok(true) => added += 1,
            Ok(false) => {}
            Err(error) => return RespValue::Error(error.message),
        }
    }
    RespValue::Integer(added)
}

fn zscore(indexes: &OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() != 3 {
        return RespValue::Error("ZSCORE requires key member".to_string());
    }
    indexes
        .index(&parts[1])
        .and_then(|index| index.zscore(parts[2].as_bytes()))
        .map(|score| RespValue::Bulk(Some(format_score(score).into_bytes())))
        .unwrap_or(RespValue::Bulk(None))
}

fn zpop(indexes: &mut OrderedIndexRegistry, parts: &[String], side: PopSide) -> RespValue {
    if parts.len() < 2 || parts.len() > 3 {
        return RespValue::Error("ZPOP requires key [count]".to_string());
    }
    let count = match parts.get(2) {
        Some(raw) => match raw.parse::<usize>() {
            Ok(count) => count,
            Err(_) => {
                return RespValue::Error("ZPOP count must be a non-negative integer".to_string())
            }
        },
        None => 1,
    };
    let Some(index) = indexes.index_mut_existing(&parts[1]) else {
        return RespValue::Array(Vec::new());
    };
    let mut out = Vec::new();
    for _ in 0..count {
        let next = match side {
            PopSide::Min => index.zpop_min(),
            PopSide::Max => index.zpop_max(),
        };
        let Some((member, score)) = next else {
            break;
        };
        out.push(RespValue::Bulk(Some(member)));
        out.push(RespValue::Bulk(Some(format_score(score).into_bytes())));
    }
    RespValue::Array(out)
}

fn zrange_by_score(indexes: &OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() < 4 {
        return RespValue::Error(
            "ZRANGEBYSCORE requires key min max [WITHSCORES] [LIMIT offset count]".to_string(),
        );
    }
    let min = match parse_score(&parts[2]) {
        Ok(score) => score,
        Err(error) => return RespValue::Error(error),
    };
    let max = match parse_score(&parts[3]) {
        Ok(score) => score,
        Err(error) => return RespValue::Error(error),
    };
    let mut with_scores = false;
    let mut offset = 0_usize;
    let mut count = None;
    let mut cursor = 4;
    while cursor < parts.len() {
        match parts[cursor].to_ascii_uppercase().as_str() {
            "WITHSCORES" => {
                with_scores = true;
                cursor += 1;
            }
            "LIMIT" => {
                if cursor + 2 >= parts.len() {
                    return RespValue::Error(
                        "ZRANGEBYSCORE LIMIT requires offset and count".to_string(),
                    );
                }
                offset = match parts[cursor + 1].parse::<usize>() {
                    Ok(value) => value,
                    Err(_) => {
                        return RespValue::Error(
                            "ZRANGEBYSCORE LIMIT offset must be a non-negative integer".to_string(),
                        )
                    }
                };
                count = match parts[cursor + 2].parse::<usize>() {
                    Ok(value) => Some(value),
                    Err(_) => {
                        return RespValue::Error(
                            "ZRANGEBYSCORE LIMIT count must be a non-negative integer".to_string(),
                        )
                    }
                };
                cursor += 3;
            }
            option => {
                return RespValue::Error(format!("unsupported ZRANGEBYSCORE option {option}"))
            }
        }
    }
    let Some(index) = indexes.index(&parts[1]) else {
        return RespValue::Array(Vec::new());
    };
    let range = match index.zrange_by_score(min, max, None) {
        Ok(range) => range,
        Err(error) => return RespValue::Error(error.message),
    };
    let mut out = Vec::new();
    for (member, score) in range
        .into_iter()
        .skip(offset)
        .take(count.unwrap_or(usize::MAX))
    {
        out.push(RespValue::Bulk(Some(member)));
        if with_scores {
            out.push(RespValue::Bulk(Some(format_score(score).into_bytes())));
        }
    }
    RespValue::Array(out)
}

fn zrem(indexes: &mut OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() < 3 {
        return RespValue::Error("ZREM requires key member [member ...]".to_string());
    }
    let Some(index) = indexes.index_mut_existing(&parts[1]) else {
        return RespValue::Integer(0);
    };
    let removed = parts[2..]
        .iter()
        .filter(|member| index.zrem(member.as_bytes()))
        .count() as i64;
    RespValue::Integer(removed)
}

fn zcard(indexes: &OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() != 2 {
        return RespValue::Error("ZCARD requires key".to_string());
    }
    RespValue::Integer(
        indexes
            .index(&parts[1])
            .map(|index| index.zcard() as i64)
            .unwrap_or_default(),
    )
}

fn zrank(indexes: &OrderedIndexRegistry, parts: &[String]) -> RespValue {
    if parts.len() != 3 {
        return RespValue::Error("ZRANK requires key member".to_string());
    }
    indexes
        .index(&parts[1])
        .and_then(|index| index.zrank(parts[2].as_bytes()))
        .map(|rank| RespValue::Integer(rank as i64))
        .unwrap_or(RespValue::Bulk(None))
}

fn parse_score(raw: &str) -> Result<f64, String> {
    let score = match raw.to_ascii_lowercase().as_str() {
        "-inf" => f64::NEG_INFINITY,
        "+inf" | "inf" => f64::INFINITY,
        _ => raw
            .parse::<f64>()
            .map_err(|_| format!("invalid floating point score '{raw}'"))?,
    };
    if score.is_nan() {
        return Err("ordered scores must not be NaN".to_string());
    }
    Ok(score)
}

fn format_score(score: f64) -> String {
    if score.fract() == 0.0 && score.is_finite() {
        format!("{score:.0}")
    } else {
        score.to_string()
    }
}

trait OrderedIndexRegistryExt {
    fn index_mut_existing(&mut self, name: &str) -> Option<&mut rustyred_thg_core::OrderedIndex>;
}

impl OrderedIndexRegistryExt for OrderedIndexRegistry {
    fn index_mut_existing(&mut self, name: &str) -> Option<&mut rustyred_thg_core::OrderedIndex> {
        if self.index(name).is_none() {
            return None;
        }
        Some(self.index_mut(name, OrderedMode::Transient))
    }
}

#[cfg(test)]
mod tests {
    use super::{execute_resp_command, resp_command_to_thg, RespValue};
    use rustyred_thg_core::OrderedIndexRegistry;

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

    #[test]
    fn zset_commands_follow_scoped_semantics() {
        let mut indexes = OrderedIndexRegistry::new();

        assert_eq!(
            execute_resp_command(
                &mut indexes,
                &["ZADD", "frontier", "1", "b", "1", "a", "3", "c"]
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            ),
            RespValue::Integer(3)
        );
        assert_eq!(
            execute_resp_command(
                &mut indexes,
                &["ZRANGEBYSCORE", "frontier", "0", "2", "WITHSCORES"]
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            ),
            RespValue::Array(vec![
                RespValue::Bulk(Some(b"a".to_vec())),
                RespValue::Bulk(Some(b"1".to_vec())),
                RespValue::Bulk(Some(b"b".to_vec())),
                RespValue::Bulk(Some(b"1".to_vec())),
            ])
        );
        assert_eq!(
            execute_resp_command(
                &mut indexes,
                &["ZPOPMAX", "frontier"]
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            ),
            RespValue::Array(vec![
                RespValue::Bulk(Some(b"c".to_vec())),
                RespValue::Bulk(Some(b"3".to_vec())),
            ])
        );
        assert_eq!(
            execute_resp_command(
                &mut indexes,
                &["LPUSH", "frontier", "x"]
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            ),
            RespValue::Error(
                "unknown command 'LPUSH'; this RustyRed RESP surface implements scoped ZSET commands only"
                    .to_string()
            )
        );
    }
}
