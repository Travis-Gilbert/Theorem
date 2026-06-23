use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use postgres_types::Type;
use rustyred_thg_core::{
    execute_query, JoinPredicate, Predicate, Projection, QueryIr, QueryRelation, RelationalStore,
    ScalarValue, ThgExecutor, ThgRequest,
};
use serde_json::{json, Value};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// Spec-faithful planner-backed native-views surface (SPEC-RUSTYRED-PG-WIRE):
/// lowers SQL to the relational planner `QueryIr` and serves native relational
/// views, alongside the `ThgExecutor`-backed `serve()` below.
pub mod relational_server;
pub use relational_server::{
    demo_native_store, execute_native_sql, serve_relational, SharedRelationalStore,
};

pub type SharedExecutor = Arc<Mutex<Box<dyn ThgExecutor + Send>>>;

const PROTOCOL_VERSION_3: i32 = 196608;
const SSL_REQUEST: i32 = 80877103;

pub fn serve(listener: TcpListener, executor: SharedExecutor) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        let executor = Arc::clone(&executor);
        thread::spawn(move || {
            let _ = handle_stream(stream, executor);
        });
    }
    Ok(())
}

pub fn handle_stream(mut stream: TcpStream, executor: SharedExecutor) -> std::io::Result<()> {
    read_startup(&mut stream)?;
    stream.write_all(&startup_response())?;
    stream.flush()?;

    let mut prepared = BTreeMap::<String, String>::new();
    let mut portals = BTreeMap::<String, String>::new();
    loop {
        let Some(message) = read_frontend_message(&mut stream)? else {
            break;
        };
        let mut out = Vec::new();
        match message {
            FrontendMessage::Query(sql) => {
                match execute_executor_sql(&sql, &mut **executor.lock().unwrap()) {
                    Ok(result) => out.extend(encode_query_result(&result)),
                    Err(message) => out.extend(error_response(&message)),
                }
                out.extend(ready_for_query());
            }
            FrontendMessage::Parse { name, query } => {
                prepared.insert(name, query);
                out.extend(parse_complete());
            }
            FrontendMessage::Bind { portal, statement } => {
                let sql = prepared.get(&statement).cloned().unwrap_or(statement);
                portals.insert(portal, sql);
                out.extend(bind_complete());
            }
            FrontendMessage::Describe { name, .. } => {
                let sql = portals
                    .get(&name)
                    .or_else(|| prepared.get(&name))
                    .cloned()
                    .unwrap_or_default();
                match describe_executor_sql(&sql, &mut **executor.lock().unwrap()) {
                    Ok(columns) => out.extend(row_description(&columns)),
                    Err(message) => out.extend(error_response(&message)),
                }
            }
            FrontendMessage::Execute { portal } => {
                let sql = portals.get(&portal).cloned().unwrap_or_default();
                match execute_executor_sql(&sql, &mut **executor.lock().unwrap()) {
                    Ok(result) => out.extend(encode_query_result(&result)),
                    Err(message) => out.extend(error_response(&message)),
                }
            }
            FrontendMessage::Sync => out.extend(ready_for_query()),
            FrontendMessage::Flush => {}
            FrontendMessage::Terminate => break,
            FrontendMessage::Password(_) => {
                out.extend(error_response(
                    "password authentication is not enabled on this trust-mode pg surface",
                ));
                out.extend(ready_for_query());
            }
        }
        if !out.is_empty() {
            stream.write_all(&out)?;
            stream.flush()?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct PgColumn {
    pub name: String,
    pub type_oid: u32,
    pub type_size: i16,
}

impl PgColumn {
    pub fn text(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_oid: Type::TEXT.oid(),
            type_size: -1,
        }
    }

    pub fn int8(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_oid: Type::INT8.oid(),
            type_size: 8,
        }
    }

    pub fn float8(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_oid: Type::FLOAT8.oid(),
            type_size: 8,
        }
    }

    pub fn bool(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_oid: Type::BOOL.oid(),
            type_size: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PgQueryResult {
    pub columns: Vec<PgColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub command_tag: String,
    pub trace: Option<Value>,
}

pub fn execute_executor_sql(
    sql: &str,
    executor: &mut dyn ThgExecutor,
) -> Result<PgQueryResult, String> {
    validate_select_sql(sql)?;
    let parsed = ParsedSelect::parse(sql)?;
    if parsed.table.is_none() {
        return execute_constant_select(&parsed.projection, executor);
    }
    match parsed.table.as_deref() {
        Some("nodes") | Some("graph_nodes") => execute_nodes_select(parsed, executor),
        Some(table) => Err(format!("unsupported native pg view '{table}'")),
        None => unreachable!(),
    }
}

pub fn describe_executor_sql(
    sql: &str,
    executor: &mut dyn ThgExecutor,
) -> Result<Vec<PgColumn>, String> {
    execute_executor_sql(sql, executor).map(|result| result.columns)
}

pub fn execute_relational_sql(sql: &str, store: &RelationalStore) -> Result<PgQueryResult, String> {
    validate_select_sql(sql)?;
    let (query, columns) = lower_select_to_query_ir(sql)?;
    let result = execute_query(store, query).map_err(|error| error.message)?;
    let rows = result
        .rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| row.get(&column.name).map(scalar_to_text))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(PgQueryResult {
        command_tag: format!("SELECT {}", rows.len()),
        columns,
        rows,
        trace: Some(serde_json::to_value(result.trace).unwrap_or_else(|_| json!({}))),
    })
}

fn execute_constant_select(
    projection: &[String],
    executor: &mut dyn ThgExecutor,
) -> Result<PgQueryResult, String> {
    let fields = if projection.is_empty() {
        vec!["1".to_string()]
    } else {
        projection.to_vec()
    };
    let mut columns = Vec::new();
    let mut row = Vec::new();
    for raw in fields {
        let (expr, alias) = split_alias(&raw);
        let expr_lc = expr.to_ascii_lowercase();
        if expr_lc == "state_hash()" || expr_lc == "state_hash" {
            columns.push(PgColumn::text(
                alias.unwrap_or_else(|| "state_hash".to_string()),
            ));
            row.push(Some(executor.state().hash()));
        } else if expr_lc == "true" || expr_lc == "false" {
            columns.push(PgColumn::bool(alias.unwrap_or_else(|| expr_lc.clone())));
            row.push(Some(expr_lc));
        } else if let Ok(value) = expr.parse::<i64>() {
            columns.push(PgColumn::int8(
                alias.unwrap_or_else(|| "?column?".to_string()),
            ));
            row.push(Some(value.to_string()));
        } else if let Ok(value) = expr.parse::<f64>() {
            columns.push(PgColumn::float8(
                alias.unwrap_or_else(|| "?column?".to_string()),
            ));
            row.push(Some(value.to_string()));
        } else if is_quoted(&expr) {
            columns.push(PgColumn::text(
                alias.unwrap_or_else(|| "?column?".to_string()),
            ));
            row.push(Some(unquote(&expr)));
        } else {
            return Err(format!("unsupported select expression '{expr}'"));
        }
    }
    Ok(PgQueryResult {
        columns,
        rows: vec![row],
        command_tag: "SELECT 1".to_string(),
        trace: None,
    })
}

fn execute_nodes_select(
    parsed: ParsedSelect,
    executor: &mut dyn ThgExecutor,
) -> Result<PgQueryResult, String> {
    let mut args = json!({});
    if let Some(label) = parsed.where_label {
        args["label"] = Value::String(label);
    }
    if let Some(limit) = parsed.limit {
        args["limit"] = json!(limit);
    }
    let response =
        executor.execute_request(ThgRequest::new("RUSTYRED_THG.GRAPH.NODES.QUERY", args));
    if !response.ok {
        return Err(response
            .error
            .map(|error| error.message)
            .unwrap_or_else(|| response.status));
    }
    let nodes = response
        .payload
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if parsed.is_count {
        return Ok(PgQueryResult {
            columns: vec![PgColumn::int8("count")],
            rows: vec![vec![Some(nodes.len().to_string())]],
            command_tag: "SELECT 1".to_string(),
            trace: response.payload.get("plan").cloned(),
        });
    }
    let fields = parsed
        .projection
        .iter()
        .flat_map(|field| {
            if field == "*" {
                vec![
                    "id".to_string(),
                    "labels".to_string(),
                    "properties".to_string(),
                ]
            } else {
                vec![field.clone()]
            }
        })
        .collect::<Vec<_>>();
    let fields = if fields.is_empty() {
        vec!["id".to_string()]
    } else {
        fields
    };
    let columns = fields.iter().map(PgColumn::text).collect::<Vec<_>>();
    let rows = nodes
        .iter()
        .map(|node| {
            fields
                .iter()
                .map(|field| node_field(node, field))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(PgQueryResult {
        command_tag: format!("SELECT {}", rows.len()),
        columns,
        rows,
        trace: response.payload.get("plan").cloned(),
    })
}

fn node_field(node: &Value, field: &str) -> Option<String> {
    match field {
        "id" => node
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        "labels" | "label" => node.get("labels").and_then(Value::as_array).map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        }),
        "properties" => node.get("properties").map(Value::to_string),
        other => node
            .get("properties")
            .and_then(|properties| properties.get(other))
            .map(value_to_text),
    }
}

fn lower_select_to_query_ir(sql: &str) -> Result<(QueryIr, Vec<PgColumn>), String> {
    let parsed = ParsedSelect::parse(sql)?;
    let table = parsed
        .table
        .clone()
        .ok_or_else(|| "relational SQL requires a FROM relation".to_string())?;
    let root_alias = parsed.alias.clone().unwrap_or_else(|| table.clone());
    let mut relations = vec![QueryRelation {
        alias: root_alias.clone(),
        relation: table,
        predicates: Vec::new(),
    }];
    if let Some(label) = parsed.where_label {
        relations[0].predicates.push(Predicate::Equals {
            column: "label".to_string(),
            value: ScalarValue::String(label),
        });
    }
    let mut joins = Vec::new();
    for join in parsed.joins {
        relations.push(QueryRelation {
            alias: join.alias.clone(),
            relation: join.relation,
            predicates: Vec::new(),
        });
        joins.push(JoinPredicate {
            left_alias: join.left_alias,
            left_column: join.left_column,
            right_alias: join.alias,
            right_column: join.right_column,
        });
    }
    let projection = parsed
        .projection
        .iter()
        .filter(|field| *field != "*")
        .map(|field| {
            let (alias, column) = split_qualified(field, &root_alias);
            Projection { alias, column }
        })
        .collect::<Vec<_>>();
    let columns = projection
        .iter()
        .map(|field| PgColumn::text(format!("{}.{}", field.alias, field.column)))
        .collect::<Vec<_>>();
    Ok((
        QueryIr {
            relations,
            joins,
            projection,
            limit: parsed.limit,
            ..QueryIr::default()
        },
        columns,
    ))
}

fn validate_select_sql(sql: &str) -> Result<(), String> {
    let dialect = GenericDialect {};
    let statements = Parser::parse_sql(&dialect, sql).map_err(|error| error.to_string())?;
    if statements.len() != 1 {
        return Err("pg-wire SQL surface expects exactly one statement".to_string());
    }
    if !statements[0]
        .to_string()
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("select")
    {
        return Err("only SELECT is supported by this pg-wire slice".to_string());
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ParsedSelect {
    projection: Vec<String>,
    table: Option<String>,
    alias: Option<String>,
    where_label: Option<String>,
    joins: Vec<ParsedJoin>,
    limit: Option<usize>,
    is_count: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedJoin {
    relation: String,
    alias: String,
    left_alias: String,
    left_column: String,
    right_column: String,
}

impl ParsedSelect {
    fn parse(sql: &str) -> Result<Self, String> {
        let sql = sql.trim().trim_end_matches(';').trim();
        let lower = sql.to_ascii_lowercase();
        let Some(select_rest) = lower.strip_prefix("select ") else {
            return Err("only SELECT is supported".to_string());
        };
        let select_start = sql.len() - select_rest.len();
        let rest = &sql[select_start..];
        let Some(from_pos) = lower.find(" from ") else {
            let projection = split_csv(rest);
            return Ok(Self {
                is_count: projection
                    .iter()
                    .any(|field| field.eq_ignore_ascii_case("count(*)")),
                projection,
                ..Self::default()
            });
        };
        let projection = split_csv(sql[select_start..from_pos].trim());
        let mut from_tail = sql[from_pos + " from ".len()..].trim().to_string();
        let mut limit = None;
        if let Some((head, raw_limit)) = split_keyword(&from_tail, " limit ") {
            limit = raw_limit
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<usize>().ok());
            from_tail = head.to_string();
        }
        let mut where_label = None;
        if let Some((head, where_clause)) = split_keyword(&from_tail, " where ") {
            where_label = parse_label_predicate(where_clause);
            from_tail = head.to_string();
        }
        let (root_part, join_part) = split_keyword(&from_tail, " join ")
            .map(|(left, right)| (left.to_string(), Some(right.to_string())))
            .unwrap_or((from_tail, None));
        let (table, alias) = parse_relation_alias(&root_part)?;
        let mut joins = Vec::new();
        if let Some(join_part) = join_part {
            let (join_relation_part, on_clause) =
                split_keyword(&join_part, " on ").ok_or_else(|| "JOIN requires ON".to_string())?;
            let (relation, alias) = parse_relation_alias(join_relation_part)?;
            let (left, right) = on_clause
                .split_once('=')
                .ok_or_else(|| "JOIN ON requires equality".to_string())?;
            let (left_alias, left_column) =
                split_qualified(left.trim(), alias.as_deref().unwrap_or(&relation));
            let (_right_alias, right_column) =
                split_qualified(right.trim(), alias.as_deref().unwrap_or(&relation));
            joins.push(ParsedJoin {
                relation,
                alias: alias.unwrap_or_else(|| "joined".to_string()),
                left_alias,
                left_column,
                right_column,
            });
        }
        Ok(Self {
            is_count: projection
                .iter()
                .any(|field| field.eq_ignore_ascii_case("count(*)")),
            projection,
            table: Some(table),
            alias,
            where_label,
            joins,
            limit,
        })
    }
}

fn parse_relation_alias(raw: &str) -> Result<(String, Option<String>), String> {
    let parts = raw.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [relation] => Ok((normalize_ident(relation), None)),
        [relation, alias] => Ok((normalize_ident(relation), Some(normalize_ident(alias)))),
        [relation, as_kw, alias] if as_kw.eq_ignore_ascii_case("as") => {
            Ok((normalize_ident(relation), Some(normalize_ident(alias))))
        }
        _ => Err(format!("unsupported relation clause '{raw}'")),
    }
}

fn parse_label_predicate(where_clause: &str) -> Option<String> {
    let (left, right) = where_clause.split_once('=')?;
    let field = left.trim().rsplit('.').next()?.trim();
    if !field.eq_ignore_ascii_case("label") {
        return None;
    }
    Some(unquote(right.trim()))
}

fn split_qualified(raw: &str, default_alias: &str) -> (String, String) {
    let cleaned = normalize_ident(raw);
    if let Some((alias, column)) = cleaned.split_once('.') {
        (alias.to_string(), column.to_string())
    } else {
        (default_alias.to_string(), cleaned)
    }
}

fn split_keyword<'a>(value: &'a str, keyword: &str) -> Option<(&'a str, &'a str)> {
    let lower = value.to_ascii_lowercase();
    let index = lower.find(keyword)?;
    Some((&value[..index], &value[index + keyword.len()..]))
}

fn split_alias(raw: &str) -> (String, Option<String>) {
    if let Some((expr, alias)) = split_keyword(raw, " as ") {
        return (expr.trim().to_string(), Some(normalize_ident(alias)));
    }
    (raw.trim().to_string(), None)
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|part| normalize_ident(part.trim()))
        .filter(|part| !part.is_empty())
        .collect()
}

fn normalize_ident(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('`')
        .to_ascii_lowercase()
}

fn scalar_to_text(value: &ScalarValue) -> String {
    match value {
        ScalarValue::String(value) => value.clone(),
        ScalarValue::I64(value) => value.to_string(),
        ScalarValue::F64(value) => value.to_string(),
        ScalarValue::Bool(value) => value.to_string(),
    }
}

fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn is_quoted(value: &str) -> bool {
    value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'')
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .replace("''", "'")
        .to_string()
}

#[derive(Clone, Debug, PartialEq)]
enum FrontendMessage {
    Query(String),
    Parse { name: String, query: String },
    Bind { portal: String, statement: String },
    Describe { target: u8, name: String },
    Execute { portal: String },
    Sync,
    Flush,
    Terminate,
    Password(String),
}

fn read_startup(stream: &mut TcpStream) -> std::io::Result<()> {
    loop {
        let len = read_i32(stream)? as usize;
        if len < 8 {
            return Err(invalid_data("invalid startup packet length"));
        }
        let mut body = vec![0_u8; len - 4];
        stream.read_exact(&mut body)?;
        let code = i32::from_be_bytes(body[..4].try_into().unwrap());
        if code == SSL_REQUEST {
            stream.write_all(b"N")?;
            continue;
        }
        if code != PROTOCOL_VERSION_3 {
            return Err(invalid_data("unsupported Postgres protocol version"));
        }
        return Ok(());
    }
}

fn read_frontend_message(stream: &mut TcpStream) -> std::io::Result<Option<FrontendMessage>> {
    let mut tag = [0_u8; 1];
    match stream.read_exact(&mut tag) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = read_i32(stream)? as usize;
    if len < 4 {
        return Err(invalid_data("invalid frontend message length"));
    }
    let mut body = vec![0_u8; len - 4];
    stream.read_exact(&mut body)?;
    Ok(Some(parse_frontend_message(tag[0], &body)?))
}

fn parse_frontend_message(tag: u8, body: &[u8]) -> std::io::Result<FrontendMessage> {
    Ok(match tag {
        b'Q' => {
            let mut cursor = 0;
            FrontendMessage::Query(read_cstr_at(body, &mut cursor)?)
        }
        b'P' => {
            let mut cursor = 0;
            let name = read_cstr_at(body, &mut cursor)?;
            let query = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Parse { name, query }
        }
        b'B' => {
            let mut cursor = 0;
            let portal = read_cstr_at(body, &mut cursor)?;
            let statement = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Bind { portal, statement }
        }
        b'D' => {
            let mut cursor = 0;
            let target = *body.first().ok_or_else(|| invalid_data("empty Describe"))?;
            cursor += 1;
            let name = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Describe { target, name }
        }
        b'E' => {
            let mut cursor = 0;
            let portal = read_cstr_at(body, &mut cursor)?;
            FrontendMessage::Execute { portal }
        }
        b'S' => FrontendMessage::Sync,
        b'H' => FrontendMessage::Flush,
        b'X' => FrontendMessage::Terminate,
        b'p' => {
            let mut cursor = 0;
            FrontendMessage::Password(read_cstr_at(body, &mut cursor)?)
        }
        _ => {
            return Err(invalid_data(format!(
                "unsupported frontend message {}",
                tag as char
            )))
        }
    })
}

fn startup_response() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(authentication_ok());
    out.extend(parameter_status("server_version", "16.0-rustyred"));
    out.extend(parameter_status("client_encoding", "UTF8"));
    out.extend(parameter_status("DateStyle", "ISO, MDY"));
    out.extend(backend_key_data(1001, 42));
    out.extend(ready_for_query());
    out
}

fn encode_query_result(result: &PgQueryResult) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(row_description(&result.columns));
    for row in &result.rows {
        out.extend(data_row(row));
    }
    out.extend(command_complete(&result.command_tag));
    out
}

fn authentication_ok() -> Vec<u8> {
    message(b'R', &0_i32.to_be_bytes())
}

fn parameter_status(name: &str, value: &str) -> Vec<u8> {
    let mut body = Vec::new();
    write_cstr(&mut body, name);
    write_cstr(&mut body, value);
    message(b'S', &body)
}

fn backend_key_data(pid: i32, secret: i32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(pid.to_be_bytes());
    body.extend(secret.to_be_bytes());
    message(b'K', &body)
}

fn ready_for_query() -> Vec<u8> {
    message(b'Z', b"I")
}

fn parse_complete() -> Vec<u8> {
    message(b'1', &[])
}

fn bind_complete() -> Vec<u8> {
    message(b'2', &[])
}

fn row_description(columns: &[PgColumn]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((columns.len() as i16).to_be_bytes());
    for column in columns {
        write_cstr(&mut body, &column.name);
        body.extend(0_u32.to_be_bytes()); // table oid
        body.extend(0_i16.to_be_bytes()); // attribute number
        body.extend(column.type_oid.to_be_bytes());
        body.extend(column.type_size.to_be_bytes());
        body.extend((-1_i32).to_be_bytes()); // type modifier
        body.extend(0_i16.to_be_bytes()); // text format
    }
    message(b'T', &body)
}

fn data_row(values: &[Option<String>]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend((values.len() as i16).to_be_bytes());
    for value in values {
        match value {
            Some(value) => {
                body.extend((value.len() as i32).to_be_bytes());
                body.extend(value.as_bytes());
            }
            None => body.extend((-1_i32).to_be_bytes()),
        }
    }
    message(b'D', &body)
}

fn command_complete(tag: &str) -> Vec<u8> {
    let mut body = Vec::new();
    write_cstr(&mut body, tag);
    message(b'C', &body)
}

fn error_response(text: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(b'S');
    write_cstr(&mut body, "ERROR");
    body.push(b'C');
    write_cstr(&mut body, "XX000");
    body.push(b'M');
    write_cstr(&mut body, text);
    body.push(0);
    message(b'E', &body)
}

fn message(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(tag);
    out.extend(((body.len() + 4) as i32).to_be_bytes());
    out.extend(body);
    out
}

fn write_cstr(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes());
    out.push(0);
}

fn read_i32(stream: &mut TcpStream) -> std::io::Result<i32> {
    let mut bytes = [0_u8; 4];
    stream.read_exact(&mut bytes)?;
    Ok(i32::from_be_bytes(bytes))
}

fn read_cstr_at(bytes: &[u8], cursor: &mut usize) -> std::io::Result<String> {
    let Some(end) = bytes[*cursor..].iter().position(|byte| *byte == 0) else {
        return Err(invalid_data("missing C string terminator"));
    };
    let start = *cursor;
    let end_abs = start + end;
    *cursor = end_abs + 1;
    Ok(String::from_utf8_lossy(&bytes[start..end_abs]).to_string())
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryThgExecutor, RelationalRow, ThgExecutor};
    use std::collections::BTreeMap;

    fn executor() -> InMemoryThgExecutor {
        InMemoryThgExecutor::new()
    }

    #[test]
    fn executor_sql_selects_state_hash_with_text_oid() {
        let mut executor = executor();
        let result = execute_executor_sql("SELECT state_hash()", &mut executor).unwrap();
        assert_eq!(result.columns[0].name, "state_hash");
        assert_eq!(result.columns[0].type_oid, Type::TEXT.oid());
        assert_eq!(result.rows.len(), 1);
        assert!(result.rows[0][0].as_ref().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn nodes_view_returns_typed_rows_from_executor() {
        let mut executor = executor();
        let response = executor.execute_request(ThgRequest::new(
            "RUSTYRED_THG.GRAPH.NODE.UPSERT",
            json!({
                "id": "mem:1",
                "labels": ["Memory"],
                "properties": { "topic": "planning" }
            }),
        ));
        assert!(response.ok);

        let result = execute_executor_sql(
            "SELECT id, labels, topic FROM nodes WHERE label = 'Memory'",
            &mut executor,
        )
        .unwrap();
        assert_eq!(result.columns[0].type_oid, Type::TEXT.oid());
        assert_eq!(
            result.rows,
            vec![vec![
                Some("mem:1".to_string()),
                Some("Memory".to_string()),
                Some("planning".to_string()),
            ]]
        );
    }

    #[test]
    fn relational_sql_lowers_join_to_query_ir_planner() {
        let mut store = RelationalStore::new();
        store
            .upsert_row(RelationalRow::new(
                "content",
                "c1",
                BTreeMap::from([("key".to_string(), ScalarValue::String("doc:1".to_string()))]),
            ))
            .unwrap();
        store
            .upsert_row(RelationalRow::new(
                "epistemic",
                "e1",
                BTreeMap::from([
                    (
                        "content_key".to_string(),
                        ScalarValue::String("doc:1".to_string()),
                    ),
                    (
                        "claim".to_string(),
                        ScalarValue::String("supports".to_string()),
                    ),
                ]),
            ))
            .unwrap();
        let result = execute_relational_sql(
            "SELECT c.key, e.claim FROM content c JOIN epistemic e ON c.key = e.content_key",
            &store,
        )
        .unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][1], Some("supports".to_string()));
        let trace = result.trace.unwrap();
        assert_eq!(trace["join_algorithm"], "hash_join");
    }

    #[test]
    fn row_description_and_data_row_encode_postgres_messages() {
        let result = PgQueryResult {
            columns: vec![PgColumn::int8("n"), PgColumn::text("label")],
            rows: vec![vec![Some("1".to_string()), Some("ok".to_string())]],
            command_tag: "SELECT 1".to_string(),
            trace: None,
        };
        let encoded = encode_query_result(&result);
        assert_eq!(encoded[0], b'T');
        assert!(encoded.contains(&b'D'));
        assert!(encoded.contains(&b'C'));
    }

    #[test]
    fn parses_extended_protocol_messages() {
        let mut parse_body = Vec::new();
        write_cstr(&mut parse_body, "stmt1");
        write_cstr(&mut parse_body, "SELECT 1");
        parse_body.extend(0_i16.to_be_bytes());
        assert_eq!(
            parse_frontend_message(b'P', &parse_body).unwrap(),
            FrontendMessage::Parse {
                name: "stmt1".to_string(),
                query: "SELECT 1".to_string()
            }
        );

        let mut bind_body = Vec::new();
        write_cstr(&mut bind_body, "");
        write_cstr(&mut bind_body, "stmt1");
        bind_body.extend(0_i16.to_be_bytes());
        bind_body.extend(0_i16.to_be_bytes());
        bind_body.extend(0_i16.to_be_bytes());
        assert_eq!(
            parse_frontend_message(b'B', &bind_body).unwrap(),
            FrontendMessage::Bind {
                portal: "".to_string(),
                statement: "stmt1".to_string()
            }
        );
    }
}
